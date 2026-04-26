use proptest::prelude::*;
use solana_clmm_raydium::{
    compute_swap_step, get_sqrt_price_at_tick, get_tick_at_sqrt_price, MAX_SQRT_PRICE_X64,
    MAX_TICK, MIN_SQRT_PRICE_X64, MIN_TICK,
};

// ---- tick <-> sqrt_price ----

// Round-trip is asymmetric at MAX_TICK: get_sqrt_price_at_tick(MAX_TICK) returns
// exactly MAX_SQRT_PRICE_X64, but get_tick_at_sqrt_price's domain check is strict
// (sqrt_price < MAX_SQRT_PRICE_X64). MAX_SQRT_PRICE_X64 is the unattainable
// upper bound a swap can approach but not reach — same convention as Uniswap V3.
// We therefore round-trip over MIN_TICK..MAX_TICK and assert the boundary
// behavior separately.

#[test]
fn tick_round_trip_sampled() {
    for tick in (MIN_TICK..MAX_TICK).step_by(100) {
        let sp = get_sqrt_price_at_tick(tick).expect("valid tick");
        let recovered = get_tick_at_sqrt_price(sp).expect("valid sqrt_price");
        assert_eq!(recovered, tick, "round-trip failed at tick={tick} sp={sp}");
    }
}

#[test]
#[ignore = "exhaustive — run with `cargo test -- --ignored`"]
fn tick_round_trip_exhaustive() {
    // every tick in the round-trippable domain — ~887k iterations.
    for tick in MIN_TICK..MAX_TICK {
        let sp = get_sqrt_price_at_tick(tick).expect("valid tick");
        let recovered = get_tick_at_sqrt_price(sp).expect("valid sqrt_price");
        assert_eq!(recovered, tick, "round-trip failed at tick={tick} sp={sp}");
    }
}

#[test]
fn max_boundary_is_one_way() {
    // MAX_TICK encodes to MAX_SQRT_PRICE_X64, but inverse rejects it.
    let sp = get_sqrt_price_at_tick(MAX_TICK).unwrap();
    assert_eq!(sp, MAX_SQRT_PRICE_X64);
    assert!(get_tick_at_sqrt_price(sp).is_err());
}

#[test]
fn sqrt_price_is_monotonic_in_tick() {
    // strict monotonicity is the load-bearing property of get_tick_at_sqrt_price's
    // binary-search inverse. If this breaks, every CLMM swap is wrong.
    let mut prev = get_sqrt_price_at_tick(MIN_TICK).unwrap();
    for tick in (MIN_TICK + 1..=MAX_TICK).step_by(100) {
        let cur = get_sqrt_price_at_tick(tick).unwrap();
        assert!(cur > prev, "non-monotone at tick={tick}: {prev} → {cur}");
        prev = cur;
    }
}

#[test]
fn out_of_domain_ticks_error() {
    assert!(get_sqrt_price_at_tick(MIN_TICK - 1).is_err());
    assert!(get_sqrt_price_at_tick(MAX_TICK + 1).is_err());
    assert!(get_tick_at_sqrt_price(MIN_SQRT_PRICE_X64 - 1).is_err());
    assert!(get_tick_at_sqrt_price(MAX_SQRT_PRICE_X64).is_err()); // half-open: MAX is exclusive
}

proptest! {
    #[test]
    fn proptest_round_trip(tick in MIN_TICK..MAX_TICK) {
        let sp = get_sqrt_price_at_tick(tick).unwrap();
        let recovered = get_tick_at_sqrt_price(sp).unwrap();
        prop_assert_eq!(recovered, tick);
    }
}

// ---- compute_swap_step invariants ----

const FEE_RATE_25_BPS: u32 = 2_500; // 0.25%
const FEE_RATE_5_BPS: u32 = 500; // 0.05%

proptest! {
    /// For zero_for_one (token0→token1): price moves down. Output token1 ≤ liquidity bound.
    #[test]
    fn swap_step_zero_for_one_price_decreases(
        // tick range loose enough to give meaningful liquidity space but tight
        // enough to avoid overflow corner cases on first pass
        current_tick in -100_000i32..100_000i32,
        // target is below current, but no more than 5_000 ticks away (one tick array)
        delta in 1i32..5_000i32,
        liquidity in 1_000u128..1_000_000_000_000u128,
        amount_remaining in 1u64..1_000_000_000_000u64,
    ) {
        let target_tick = current_tick - delta;
        prop_assume!(target_tick >= MIN_TICK);
        let sp_current = get_sqrt_price_at_tick(current_tick).unwrap();
        let sp_target = get_sqrt_price_at_tick(target_tick).unwrap();
        let step = compute_swap_step(
            sp_current, sp_target, liquidity, amount_remaining,
            FEE_RATE_25_BPS,
            /* is_base_input */ true,
            /* zero_for_one  */ true,
            /* block_timestamp */ 0,
        ).unwrap();
        prop_assert!(step.sqrt_price_next_x64 <= sp_current,
            "price should not increase on zero_for_one swap");
        prop_assert!(step.sqrt_price_next_x64 >= sp_target,
            "price should not pass target on zero_for_one swap");
        prop_assert!(step.amount_in <= amount_remaining,
            "amount_in {} cannot exceed amount_remaining {}",
            step.amount_in, amount_remaining);
    }

    /// For one_for_zero (token1→token0): price moves up.
    #[test]
    fn swap_step_one_for_zero_price_increases(
        current_tick in -100_000i32..100_000i32,
        delta in 1i32..5_000i32,
        liquidity in 1_000u128..1_000_000_000_000u128,
        amount_remaining in 1u64..1_000_000_000_000u64,
    ) {
        let target_tick = current_tick + delta;
        prop_assume!(target_tick <= MAX_TICK);
        let sp_current = get_sqrt_price_at_tick(current_tick).unwrap();
        let sp_target = get_sqrt_price_at_tick(target_tick).unwrap();
        let step = compute_swap_step(
            sp_current, sp_target, liquidity, amount_remaining,
            FEE_RATE_5_BPS,
            true, false, 0,
        ).unwrap();
        prop_assert!(step.sqrt_price_next_x64 >= sp_current,
            "price should not decrease on one_for_zero swap");
        prop_assert!(step.sqrt_price_next_x64 <= sp_target,
            "price should not pass target on one_for_zero swap");
        prop_assert!(step.amount_in <= amount_remaining);
    }

    /// Fee accounting: amount_in is post-fee. fee_amount is at most the implied fee on amount_in.
    #[test]
    fn swap_step_fee_is_bounded(
        current_tick in -50_000i32..50_000i32,
        delta in 1i32..2_000i32,
        liquidity in 100_000u128..100_000_000u128,
        amount_remaining in 1_000u64..100_000_000u64,
    ) {
        let target_tick = current_tick - delta;
        prop_assume!(target_tick >= MIN_TICK);
        let sp_current = get_sqrt_price_at_tick(current_tick).unwrap();
        let sp_target = get_sqrt_price_at_tick(target_tick).unwrap();
        let step = compute_swap_step(
            sp_current, sp_target, liquidity, amount_remaining,
            FEE_RATE_25_BPS, true, true, 0,
        ).unwrap();
        // total cost (in + fee) cannot exceed amount_remaining
        let total_cost = step.amount_in.saturating_add(step.fee_amount);
        prop_assert!(total_cost <= amount_remaining,
            "amount_in {} + fee {} = {} exceeds amount_remaining {}",
            step.amount_in, step.fee_amount, total_cost, amount_remaining);
    }
}
