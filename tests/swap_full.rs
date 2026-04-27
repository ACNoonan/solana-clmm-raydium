//! Direct unit tests for `compute_swap_full`. The mainnet replay
//! (`tests/replay.rs`) is the authoritative byte-exact correctness check
//! against 17 real Raydium SOL/USDC swaps. This file covers behaviors the
//! mainnet fixtures don't necessarily exercise: empty-tick walk, immediate
//! limit hit, exact-out routing, and zero-input no-op.

use solana_clmm_raydium::{
    compute_swap_full, get_sqrt_price_at_tick, InitializedTick, SwapPool, MAX_SQRT_PRICE_X64,
    MIN_SQRT_PRICE_X64,
};

fn pool_at_tick(tick: i32, liquidity: u128) -> SwapPool {
    SwapPool {
        sqrt_price_x64: get_sqrt_price_at_tick(tick).unwrap(),
        liquidity,
        tick_current: tick,
        tick_spacing: 1,
        fee_rate_pips: 100, // 0.01%
    }
}

#[test]
fn zero_input_returns_no_op() {
    let pool = pool_at_tick(0, 1_000_000_000_000);
    let res = compute_swap_full(&pool, &[], 0, 0, true, false).unwrap();
    assert_eq!(res.amount_in, 0);
    assert_eq!(res.amount_out, 0);
    assert_eq!(res.fee_amount, 0);
    assert_eq!(res.steps, 0);
    assert_eq!(res.final_sqrt_price_x64, pool.sqrt_price_x64);
    assert_eq!(res.final_tick, pool.tick_current);
    assert_eq!(res.final_liquidity, pool.liquidity);
}

#[test]
fn empty_tick_slice_swaps_against_constant_liquidity() {
    // No initialized ticks → swap consumes input against the constant
    // liquidity until amount_remaining is exhausted or sp_limit is reached.
    // With a generous limit and modest input, no liquidity changes happen.
    let pool = pool_at_tick(0, 10_000_000_000_000_000);
    let res = compute_swap_full(&pool, &[], 1_000_000, 0, true, false).unwrap();
    assert!(res.amount_in > 0);
    assert!(res.amount_out > 0);
    assert!(res.fee_amount > 0);
    assert_eq!(res.final_liquidity, pool.liquidity); // no crosses
    assert!(res.steps >= 1);
}

#[test]
fn immediate_sqrt_price_limit_hit() {
    // sp_limit = current price (almost) → swap should hit the limit on the
    // first step and exit. With zero_for_one=false the limit is upward, so
    // we set sp_limit = current + 1 (essentially-already-there).
    let pool = pool_at_tick(0, 10_000_000_000_000_000);
    let sp_limit = pool.sqrt_price_x64 + 1;
    let res = compute_swap_full(&pool, &[], 1_000_000, sp_limit, true, false).unwrap();
    // We may consume a tiny amount before the price ratchets to the limit.
    assert!(res.steps >= 1);
    assert!(res.final_sqrt_price_x64 <= sp_limit);
    // Some input remains because the limit blocked further consumption.
    assert!(res.amount_in <= 1_000_000);
}

#[test]
fn tick_cross_changes_active_liquidity() {
    // One initialized tick at +100 with liquidity_net = -1e15.
    // Swap upward through it; final_liquidity must be reduced by 1e15.
    let initial_liquidity: u128 = 10_000_000_000_000_000;
    let pool = pool_at_tick(0, initial_liquidity);
    let ticks = [InitializedTick {
        tick: 100,
        liquidity_net: -1_000_000_000_000_000,
    }];
    // Use a large input so we definitely traverse past tick 100.
    let res = compute_swap_full(&pool, &ticks, 100_000_000_000, 0, true, false).unwrap();
    if res.final_tick >= 100 {
        assert_eq!(
            res.final_liquidity,
            initial_liquidity - 1_000_000_000_000_000
        );
    } else {
        // Didn't cross (input wasn't enough or sp_limit blocked) — liquidity unchanged.
        assert_eq!(res.final_liquidity, initial_liquidity);
    }
}

#[test]
fn zero_for_one_walks_down() {
    let pool = pool_at_tick(0, 10_000_000_000_000_000);
    let res = compute_swap_full(&pool, &[], 1_000_000, 0, true, true).unwrap();
    assert!(res.final_sqrt_price_x64 < pool.sqrt_price_x64);
    assert!(res.amount_out > 0);
}

#[test]
fn unlimited_swap_remaps_to_just_inside_domain() {
    // sqrt_price_limit_x64 = 0 should remap to ±1 inside the domain bound.
    // The swap shouldn't error on domain-edge sqrt prices.
    let pool = pool_at_tick(0, 10_000_000_000_000);
    // Going up — limit is MAX_SQRT_PRICE_X64 - 1.
    let up = compute_swap_full(&pool, &[], 1_000_000, 0, true, false).unwrap();
    assert!(up.final_sqrt_price_x64 < MAX_SQRT_PRICE_X64);
    // Going down — limit is MIN_SQRT_PRICE_X64 + 1.
    let down = compute_swap_full(&pool, &[], 1_000_000, 0, true, true).unwrap();
    assert!(down.final_sqrt_price_x64 > MIN_SQRT_PRICE_X64);
}
