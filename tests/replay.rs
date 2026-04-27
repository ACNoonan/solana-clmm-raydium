//! Mainnet replay test: load each fixture, run our public `compute_swap_full`
//! against the decoded pool state, assert byte-exact match on amount_in and
//! amount_out for all 17 captured Raydium SOL/USDC swaps.
//!
//! Until v0.2 the loop was inlined here; the public `compute_swap_full`
//! orchestrator (issue #1) makes that inlined version redundant. This test
//! is now a thin wrapper that verifies the public function reproduces every
//! mainnet fixture exactly — proof of correctness for downstream consumers.

mod support;

use base64::Engine;
use serde_json::Value;
use solana_clmm_raydium::{compute_swap_full, InitializedTick, SwapPool};
use std::path::PathBuf;
use support::decode::{PoolState, TickArrayState};

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
/// Flatten the snapshotted tick arrays into the sorted slice that
/// `compute_swap_full` expects. For tick-spacing-1 SOL/USDC fixtures this
/// is at most a few thousand ticks.
fn flatten_ticks(arrays: &[TickArrayState]) -> Vec<InitializedTick> {
    let mut out: Vec<InitializedTick> = arrays
        .iter()
        .flat_map(|ta| {
            ta.ticks.iter().copied().filter_map(|t| {
                t.is_initialized().then_some(InitializedTick {
                    tick: t.tick,
                    liquidity_net: t.liquidity_net,
                })
            })
        })
        .collect();
    out.sort_by_key(|t| t.tick);
    out
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
        let observed_in = swap["observed_amount_in"].as_u64().unwrap();
        let observed_out = swap["observed_amount_out"].as_u64().unwrap();
        let user_limit: u128 = swap["sqrt_price_limit_x64"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();

        let ticks = flatten_ticks(&arrays);
        let swap_pool = SwapPool {
            sqrt_price_x64: pool.sqrt_price_x64,
            liquidity: pool.liquidity,
            tick_current: pool.tick_current,
            tick_spacing: meta["tick_spacing"].as_u64().unwrap_or(0) as u16,
            fee_rate_pips: fee_rate_ppm,
        };
        let replay = compute_swap_full(
            &swap_pool,
            &ticks,
            amount,
            user_limit,
            is_base_input,
            zero_for_one,
        )
        .expect("compute_swap_full");

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
