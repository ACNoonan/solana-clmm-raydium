//! Mainnet replay test: load each fixture, run our `compute_swap_step` against
//! the decoded pool state with proper multi-tick walking, assert byte-exact
//! match on amount_in and amount_out.
//!
//! Walking logic mirrors raydium-clmm/programs/amm/src/instructions/swap.rs::swap_internal:
//!   - find the next initialized tick toward the limit
//!   - target_price = the next-tick sqrt price OR the user limit, whichever the
//!     swap reaches first
//!   - call compute_swap_step
//!   - if we reached the next tick exactly, cross it (apply liquidity_net,
//!     negated for zero_for_one)
//!   - otherwise recompute current tick from sp_next
//!   - loop until amount_remaining == 0 or sp_current == sp_limit

mod support;

use base64::Engine;
use serde_json::Value;
use solana_clmm_raydium::{
    add_delta, compute_swap_step, get_sqrt_price_at_tick, get_tick_at_sqrt_price,
    MAX_SQRT_PRICE_X64, MAX_TICK, MIN_SQRT_PRICE_X64, MIN_TICK,
};
use std::path::PathBuf;
use support::decode::{PoolState, TickArrayState, TickState};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_metadata() -> Value {
    let p = fixtures_dir().join("clmm_pool_metadata.json");
    serde_json::from_slice(&std::fs::read(&p).expect("clmm_pool_metadata.json missing"))
        .expect("invalid metadata json")
}

fn load_fixtures() -> Vec<(PathBuf, Value)> {
    let dir = fixtures_dir();
    let mut out = vec![];
    for entry in std::fs::read_dir(&dir).expect("fixtures dir") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("swap_") && name.ends_with(".json") {
            let bytes = std::fs::read(entry.path()).unwrap();
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            out.push((entry.path(), v));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .expect("valid base64")
}

/// Result of replaying a swap through our crate's math.
#[allow(dead_code)] // final_sqrt_price_x64 retained for future tighter assertions
struct ReplayOut {
    amount_in: u64,  // gross input (incl fees), should match observed_amount_in
    amount_out: u64, // gross output, should match observed_amount_out
    final_sqrt_price_x64: u128,
}

#[allow(clippy::too_many_arguments)] // mirrors compute_swap_step's surface area
fn replay_swap(
    pool: &PoolState,
    arrays: &[TickArrayState],
    amount_specified: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
    zero_for_one: bool,
    fee_rate_ppm: u32,
    block_timestamp: u32,
) -> ReplayOut {
    let mut sp_current = pool.sqrt_price_x64;
    let mut tick_current = pool.tick_current;
    let mut liquidity = pool.liquidity;
    let mut amount_remaining = amount_specified;
    let mut amount_calculated: u64 = 0;

    // Resolve the user's limit: 0 means "no limit"; on-chain remaps to ±1
    // away from the absolute domain bound.
    let sp_limit = if sqrt_price_limit_x64 == 0 {
        if zero_for_one {
            MIN_SQRT_PRICE_X64 + 1
        } else {
            MAX_SQRT_PRICE_X64 - 1
        }
    } else {
        sqrt_price_limit_x64
    };

    // Flatten initialized ticks across all snapshotted arrays into a sorted view.
    // (For tick-spacing-1 SOL/USDC, the snapshot includes only the arrays the
    // on-chain swap touched, so this is small.)
    let mut all_ticks: Vec<TickState> = arrays
        .iter()
        .flat_map(|ta| ta.ticks.iter().copied().filter(|t| t.is_initialized()))
        .collect();
    all_ticks.sort_by_key(|t| t.tick);

    // Hard cap on iterations: a real CLMM swap shouldn't traverse more than a
    // few dozen tick crossings even at the extremes. Hitting 256 means the loop
    // is stuck (no-op step) — fail loudly so we see the proximate cause rather
    // than a downstream amount mismatch.
    let mut steps = 0;
    while amount_remaining > 0 && sp_current != sp_limit {
        steps += 1;
        assert!(
            steps <= 256,
            "swap exceeded 256 steps — likely infinite loop. \
             tick={tick_current} sp={sp_current} L={liquidity} amt_rem={amount_remaining}"
        );

        // Next initialized tick in swap direction.
        let next_tick_opt: Option<i32> = if zero_for_one {
            // largest initialized tick <= state.tick (on-chain uses `<=` for the
            // first cross-search; mirror that)
            all_ticks
                .iter()
                .rev()
                .find(|t| t.tick <= tick_current)
                .map(|t| t.tick)
        } else {
            // smallest initialized tick > state.tick
            all_ticks
                .iter()
                .find(|t| t.tick > tick_current)
                .map(|t| t.tick)
        };

        // If no initialized tick in our snapshot, fall back to the domain bound
        // — the swap should hit sp_limit before reaching it.
        let next_tick = next_tick_opt.unwrap_or(if zero_for_one { MIN_TICK } else { MAX_TICK });
        let next_tick = next_tick.clamp(MIN_TICK, MAX_TICK);

        let sp_next_tick = get_sqrt_price_at_tick(next_tick).unwrap();
        let target_price = if zero_for_one {
            // moving down: target is the higher of next_tick_sp and sp_limit
            // (closer to current — closer is hit first)
            if sp_next_tick < sp_limit {
                sp_limit
            } else {
                sp_next_tick
            }
        } else {
            // moving up: target is the lower of next_tick_sp and sp_limit
            if sp_next_tick > sp_limit {
                sp_limit
            } else {
                sp_next_tick
            }
        };

        if std::env::var("REPLAY_TRACE").is_ok() {
            eprintln!(
                "  step {steps}: tick={tick_current} sp={sp_current} L={liquidity} amt_rem={amount_remaining} \
                 next_tick={next_tick} target={target_price}"
            );
        }
        let step = compute_swap_step(
            sp_current,
            target_price,
            liquidity,
            amount_remaining,
            fee_rate_ppm,
            is_base_input,
            zero_for_one,
            block_timestamp,
        )
        .unwrap();
        if std::env::var("REPLAY_TRACE").is_ok() {
            eprintln!(
                "         out: amount_in={} amount_out={} fee={} sp_next={}",
                step.amount_in, step.amount_out, step.fee_amount, step.sqrt_price_next_x64
            );
        }

        sp_current = step.sqrt_price_next_x64;
        if is_base_input {
            amount_remaining = amount_remaining
                .checked_sub(step.amount_in + step.fee_amount)
                .expect("amount_remaining underflow");
            amount_calculated = amount_calculated
                .checked_add(step.amount_out)
                .expect("amount_calculated overflow");
        } else {
            amount_remaining = amount_remaining
                .checked_sub(step.amount_out)
                .expect("amount_remaining underflow");
            amount_calculated = amount_calculated
                .checked_add(step.amount_in + step.fee_amount)
                .expect("amount_calculated overflow");
        }

        // Did we hit the target exactly? (i.e. reach the next initialized tick)
        if sp_current == target_price && target_price == sp_next_tick {
            // We crossed the next initialized tick — apply its liquidity_net.
            // For zero_for_one we negate liquidity_net (crossing right→left).
            let crossed = all_ticks
                .iter()
                .find(|t| t.tick == next_tick)
                .copied()
                .expect("next_tick should be in initialized set");
            let mut net = crossed.liquidity_net;
            if zero_for_one {
                net = -net;
            }
            liquidity = add_delta(liquidity, net).expect("liquidity overflow on cross");
            tick_current = if zero_for_one {
                next_tick - 1
            } else {
                next_tick
            };
        } else if sp_current == target_price && target_price == sp_limit {
            // Reached user limit — stop.
            break;
        } else {
            // Didn't reach target — amount exhausted mid-range.
            tick_current = get_tick_at_sqrt_price(sp_current).unwrap_or(tick_current);
        }
    }

    let consumed = amount_specified - amount_remaining;
    let (amount_in, amount_out) = if is_base_input {
        (consumed, amount_calculated)
    } else {
        (amount_calculated, consumed)
    };
    ReplayOut {
        amount_in,
        amount_out,
        final_sqrt_price_x64: sp_current,
    }
}

#[test]
fn replay_fixtures_match_observed() {
    let fixtures = load_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no swap_*.json fixtures in tests/fixtures/ — \
         the committed fixture set is required for this test to be meaningful. \
         Re-clone or run scripts/fetch_fixtures.py to regenerate."
    );
    let meta = load_metadata();
    let fee_rate_ppm: u32 = (meta["fee_rate"].as_f64().unwrap() * 1_000_000.0).round() as u32;

    let mut passed = 0;
    let mut failures: Vec<String> = vec![];

    for (path, fx) in &fixtures {
        let pool =
            PoolState::from_bytes(&b64(fx["pool_b64"].as_str().unwrap())).expect("decode pool");
        let arrays: Vec<TickArrayState> = fx["tick_arrays"]
            .as_array()
            .unwrap()
            .iter()
            .map(|ta| TickArrayState::from_bytes(&b64(ta["data_b64"].as_str().unwrap())).unwrap())
            .collect();

        let swap = &fx["swap"];
        let amount = swap["amount"].as_u64().unwrap();
        let is_base_input = swap["is_base_input"].as_bool().unwrap();
        let zero_for_one = swap["zero_for_one"].as_bool().unwrap();
        let block_time = swap["block_time"].as_u64().unwrap_or(0) as u32;
        let observed_in = swap["observed_amount_in"].as_u64().unwrap();
        let observed_out = swap["observed_amount_out"].as_u64().unwrap();
        let user_limit: u128 = swap["sqrt_price_limit_x64"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let replay = replay_swap(
            &pool,
            &arrays,
            amount,
            user_limit,
            is_base_input,
            zero_for_one,
            fee_rate_ppm,
            block_time,
        );

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if replay.amount_in == observed_in && replay.amount_out == observed_out {
            passed += 1;
            eprintln!("✓ {name}  in:{observed_in}  out:{observed_out}");
        } else {
            failures.push(format!(
                "✗ {name}\n    pred_in={} obs_in={} (Δ={})\n    pred_out={} obs_out={} (Δ={})",
                replay.amount_in,
                observed_in,
                replay.amount_in as i128 - observed_in as i128,
                replay.amount_out,
                observed_out,
                replay.amount_out as i128 - observed_out as i128,
            ));
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("{f}");
        }
        panic!(
            "{} of {} fixtures failed replay",
            failures.len(),
            fixtures.len()
        );
    }
    eprintln!("{passed}/{} fixtures replayed exactly", fixtures.len());
}
