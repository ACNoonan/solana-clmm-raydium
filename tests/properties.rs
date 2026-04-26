use proptest::prelude::*;
use solana_clmm_raydium::{
    compute_swap_step, get_sqrt_price_at_tick, get_tick_at_sqrt_price, FEE_RATE_DENOMINATOR_VALUE,
    MAX_SQRT_PRICE_X64, MAX_TICK, MIN_SQRT_PRICE_X64, MIN_TICK,
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

// Property tests over the full tick / sqrt-price domain. Names mirror the
// upstream raydium-clmm proptests (deleted during extraction in M1) so they
// stay diffable. Prefer these over the hand-written direction-specific
// proptests we shipped in M2 — they cover the full domain with a single test
// each and exercise boundary cases the M2 proptests skipped.

proptest! {
    /// `sqrt_price_at_tick(t)` lies inside the canonical CLMM price domain
    /// and is strictly between sqrt_price_at_tick(t-1) and sqrt_price_at_tick(t+1).
    #[test]
    fn get_sqrt_price_at_tick_test(tick in MIN_TICK + 1..MAX_TICK - 1) {
        let sqrt_price_x64 = get_sqrt_price_at_tick(tick).unwrap();
        prop_assert!(sqrt_price_x64 >= MIN_SQRT_PRICE_X64);
        prop_assert!(sqrt_price_x64 <= MAX_SQRT_PRICE_X64);

        let minus = get_sqrt_price_at_tick(tick - 1).unwrap();
        let plus = get_sqrt_price_at_tick(tick + 1).unwrap();
        prop_assert!(minus < sqrt_price_x64 && sqrt_price_x64 < plus);
    }

    /// `tick_at_sqrt_price(sp)` returns a tick whose canonical interval
    /// `[sp(t), sp(t+1))` actually contains sp.
    #[test]
    fn get_tick_at_sqrt_price_test(
        sqrt_price in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
    ) {
        let tick = get_tick_at_sqrt_price(sqrt_price).unwrap();
        prop_assert!(tick >= MIN_TICK);
        prop_assert!(tick <= MAX_TICK);
        prop_assert!(
            sqrt_price >= get_sqrt_price_at_tick(tick).unwrap()
                && sqrt_price < get_sqrt_price_at_tick(tick + 1).unwrap()
        );
    }

    /// `tick → sqrt_price → tick` round-trip across the full domain.
    /// Replaces our M2 narrower `proptest_round_trip` — same intent, full range.
    #[test]
    fn tick_and_sqrt_price_symmetry_test(tick in MIN_TICK..MAX_TICK) {
        let sqrt_price_x64 = get_sqrt_price_at_tick(tick).unwrap();
        let resolved = get_tick_at_sqrt_price(sqrt_price_x64).unwrap();
        prop_assert_eq!(resolved, tick);
    }

    /// Strict adjacent-pair monotonicity over the entire tick domain.
    /// `sqrt_price_is_monotonic_in_tick` already tests this with step-100
    /// sampling; this catches any non-monotone *adjacent* pair the sample skips.
    #[test]
    fn get_sqrt_price_at_tick_is_sequence_test(tick in MIN_TICK + 1..MAX_TICK) {
        let cur = get_sqrt_price_at_tick(tick).unwrap();
        let prev = get_sqrt_price_at_tick(tick - 1).unwrap();
        prop_assert!(prev < cur);
    }

    /// Tick-from-sqrt-price is non-decreasing across small price deltas.
    #[test]
    fn get_tick_at_sqrt_price_is_sequence_test(
        sqrt_price in (MIN_SQRT_PRICE_X64 + 10)..MAX_SQRT_PRICE_X64,
    ) {
        let tick = get_tick_at_sqrt_price(sqrt_price).unwrap();
        let prev_tick = get_tick_at_sqrt_price(sqrt_price - 10).unwrap();
        prop_assert!(prev_tick <= tick);
    }
}

// ---- compute_swap_step ----
//
// Single proptest covering the full domain across both directions and both
// exact-in / exact-out modes. Mirrors upstream's `compute_swap_step_test`,
// adapted for our public API. Replaces the three direction-specific proptests
// shipped in M2 — those were a strict subset of this. See audit §4.2.

proptest! {
    #[test]
    fn compute_swap_step_test(
        sqrt_price_current_x64 in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        sqrt_price_target_x64 in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        liquidity in 1u128..u32::MAX as u128,
        amount_remaining in 1u64..u64::MAX,
        fee_rate in 1u32..FEE_RATE_DENOMINATOR_VALUE / 2,
        is_base_input in proptest::bool::ANY,
    ) {
        prop_assume!(sqrt_price_current_x64 != sqrt_price_target_x64);
        let zero_for_one = sqrt_price_current_x64 > sqrt_price_target_x64;

        let step = compute_swap_step(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            amount_remaining,
            fee_rate,
            is_base_input,
            zero_for_one,
            /* block_timestamp */ 1,
        ).unwrap();

        let amount_used = if is_base_input {
            step.amount_in + step.fee_amount
        } else {
            step.amount_out
        };

        // If we did not reach the target, ALL of amount_remaining must have
        // been used (this is the tighter equality the M2 proptest only
        // bounded, see audit §4.2).
        if step.sqrt_price_next_x64 != sqrt_price_target_x64 {
            prop_assert_eq!(amount_used, amount_remaining);
        } else {
            prop_assert!(amount_used <= amount_remaining);
        }

        // sp_next stays inside the [current, target] interval (both directions).
        let lower = sqrt_price_current_x64.min(sqrt_price_target_x64);
        let upper = sqrt_price_current_x64.max(sqrt_price_target_x64);
        prop_assert!(step.sqrt_price_next_x64 >= lower);
        prop_assert!(step.sqrt_price_next_x64 <= upper);
    }
}
