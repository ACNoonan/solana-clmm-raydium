//! Direct unit tests for the public API functions that the integration tests
//! only exercise transitively. Audit (§4.5) flagged that these had zero
//! direct coverage:
//!
//! - `next_initialized_tick_array_start_index` (in curated public API but
//!   bypassed by the replay test, which uses a flat-Vec linear scan)
//! - `add_delta` (only exercised on tick crosses; no direct test)
//! - `get_liquidity_from_amounts` (and its round-trip with
//!   `get_delta_amount_*_unsigned`)
//!
//! Tests assert specific values rather than printing-and-walking like the
//! upstream tests we deleted in M1.

use solana_clmm_raydium::{
    add_delta, get_delta_amount_0_unsigned, get_delta_amount_1_unsigned,
    get_liquidity_from_amounts, get_sqrt_price_at_tick, next_initialized_tick_array_start_index,
    ErrorCode, PoolTickBitmap, TICK_ARRAY_SIZE,
};

// ---- next_initialized_tick_array_start_index ----
//
// Bitmap layout (from `next_initialized_tick_array_start_index`):
//
//   bit position 512  = array_start_index 0
//   bit position 511  = array_start_index = -multiplier
//   bit position 513  = array_start_index = +multiplier
//
// where multiplier = tick_spacing * TICK_ARRAY_SIZE (= 60 * tick_spacing).
//
// `PoolTickBitmap` stores 1024 bits across 16 u64 limbs in little-endian
// order: bit i lives in `limbs[i / 64]` at position `i % 64`.

const SPACING: u16 = 10;
const MULTIPLIER: i32 = SPACING as i32 * TICK_ARRAY_SIZE; // 600

/// Helper: build a bitmap with only the given bit positions set.
fn bitmap_with_bits(bits: &[i32]) -> PoolTickBitmap {
    let mut limbs = [0u64; 16];
    for &bit in bits {
        assert!((0..1024).contains(&bit), "bit out of range");
        limbs[(bit / 64) as usize] |= 1u64 << (bit % 64);
    }
    PoolTickBitmap(limbs)
}

/// All-bits-set bitmap (= every tick-array slot is initialized).
const FULL_BM: PoolTickBitmap = PoolTickBitmap([u64::MAX; 16]);

/// Helper: convert array start index to its bit position in the bitmap.
fn bit_pos(start_index: i32) -> i32 {
    let mut compressed = start_index / MULTIPLIER + 512;
    if start_index < 0 && start_index % MULTIPLIER != 0 {
        compressed -= 1;
    }
    compressed.abs()
}

#[test]
fn next_array_all_bits_walk_down_strict() {
    // Every array slot is initialized → walking down by one tick array each
    // call yields a strict descending sequence: -MULTIPLIER, -2*MULTIPLIER, ...
    let bm = FULL_BM;
    let mut start = 5 * MULTIPLIER;
    let expected = [
        4 * MULTIPLIER,
        3 * MULTIPLIER,
        2 * MULTIPLIER,
        MULTIPLIER,
        0,
        -MULTIPLIER,
    ];
    for &exp in &expected {
        let (found, next) = next_initialized_tick_array_start_index(&bm, start, SPACING, true);
        assert!(found, "expected found at start={start}");
        assert_eq!(next, exp, "wrong next-down from start={start}");
        start = next;
    }
}

#[test]
fn next_array_all_bits_walk_up_strict() {
    let bm = FULL_BM;
    let mut start = -3 * MULTIPLIER;
    let expected = [
        -2 * MULTIPLIER,
        -MULTIPLIER,
        0,
        MULTIPLIER,
        2 * MULTIPLIER,
        3 * MULTIPLIER,
    ];
    for &exp in &expected {
        let (found, next) = next_initialized_tick_array_start_index(&bm, start, SPACING, false);
        assert!(found, "expected found at start={start}");
        assert_eq!(next, exp, "wrong next-up from start={start}");
        start = next;
    }
}

#[test]
fn next_array_empty_bitmap_returns_not_found_at_boundary() {
    // No bits set → walking either direction returns (false, boundary).
    let bm = PoolTickBitmap::EMPTY;
    let max_in_bitmap =
        i32::from(SPACING) * TICK_ARRAY_SIZE * solana_clmm_raydium::TICK_ARRAY_BITMAP_SIZE;

    let (found_down, val_down) = next_initialized_tick_array_start_index(&bm, 0, SPACING, true);
    assert!(!found_down);
    assert_eq!(
        val_down, -max_in_bitmap,
        "down-not-found should clamp to -max"
    );

    let (found_up, val_up) = next_initialized_tick_array_start_index(&bm, 0, SPACING, false);
    assert!(!found_up);
    assert_eq!(
        val_up,
        max_in_bitmap - MULTIPLIER,
        "up-not-found should clamp to max-multiplier"
    );
}

#[test]
fn next_array_single_bit_finds_only_initialized_array() {
    // Initialize ONLY array at start_index = 5 * MULTIPLIER. Walk down from
    // far above and the function should jump straight to it.
    let target = 5 * MULTIPLIER;
    let bm = bitmap_with_bits(&[bit_pos(target)]);

    let (found, next) =
        next_initialized_tick_array_start_index(&bm, 50 * MULTIPLIER, SPACING, true);
    assert!(found);
    assert_eq!(
        next, target,
        "walk-down should find the lone initialized array"
    );

    // Walking up from far below: same array.
    let (found, next) =
        next_initialized_tick_array_start_index(&bm, -50 * MULTIPLIER, SPACING, false);
    assert!(found);
    assert_eq!(
        next, target,
        "walk-up should find the lone initialized array"
    );
}

#[test]
fn next_array_walk_down_crosses_zero() {
    // A swap going down through 0: from +600 to the next initialized below 0.
    let bm = bitmap_with_bits(&[bit_pos(-3 * MULTIPLIER), bit_pos(MULTIPLIER)]);

    // From start = MULTIPLIER going down: first hit is -3*MULTIPLIER (past 0).
    let (found, next) = next_initialized_tick_array_start_index(&bm, MULTIPLIER, SPACING, true);
    assert!(found);
    assert_eq!(next, -3 * MULTIPLIER);
}

#[test]
fn next_array_walk_up_crosses_zero() {
    let bm = bitmap_with_bits(&[bit_pos(-MULTIPLIER), bit_pos(3 * MULTIPLIER)]);

    let (found, next) = next_initialized_tick_array_start_index(&bm, -MULTIPLIER, SPACING, false);
    assert!(found);
    assert_eq!(next, 3 * MULTIPLIER);
}

#[test]
fn next_array_at_negative_boundary_returns_not_found() {
    // Stepping below the bitmap's lowest representable array → not-found.
    let bm = FULL_BM;
    let min_array_start =
        -(i32::from(SPACING) * TICK_ARRAY_SIZE * solana_clmm_raydium::TICK_ARRAY_BITMAP_SIZE);
    // last_tick_array_start_index = lowest-representable. Going down should
    // try to step below the bitmap and return not-found.
    let (found, val) = next_initialized_tick_array_start_index(&bm, min_array_start, SPACING, true);
    assert!(!found);
    assert_eq!(
        val, min_array_start,
        "should preserve the input start index"
    );
}

// ---- add_delta ----

#[test]
fn add_delta_normal_increase() {
    assert_eq!(add_delta(100, 50).unwrap(), 150);
    assert_eq!(add_delta(0, 1).unwrap(), 1);
    assert_eq!(add_delta(u128::MAX - 5, 5).unwrap(), u128::MAX);
}

#[test]
fn add_delta_normal_decrease() {
    assert_eq!(add_delta(100, -50).unwrap(), 50);
    assert_eq!(add_delta(u128::MAX, -1).unwrap(), u128::MAX - 1);
}

#[test]
fn add_delta_zero_is_identity() {
    assert_eq!(add_delta(0, 0).unwrap(), 0);
    assert_eq!(add_delta(12345, 0).unwrap(), 12345);
    assert_eq!(add_delta(u128::MAX, 0).unwrap(), u128::MAX);
}

#[test]
fn add_delta_overflow_traps() {
    // Upstream's `add_delta` does the arithmetic THEN checks via `require_gte!`.
    // In debug builds, Rust's overflow check panics on the `x + |y|` first
    // (before the `require!` runs). In release builds, the addition wraps
    // and `require_gte!(z, x)` catches it as `LiquidityAddValueErr`.
    //
    // Either outcome is "overflow detected". The test asserts neither path
    // silently returns a wrong value.
    let result = std::panic::catch_unwind(|| add_delta(u128::MAX, 1));
    match result {
        Err(_) => { /* debug: panic on overflow — fine */ }
        Ok(Err(ErrorCode::LiquidityAddValueErr)) => { /* release: caught */ }
        Ok(Ok(v)) => panic!("add_delta(MAX, 1) silently returned {v}"),
        Ok(Err(other)) => panic!("unexpected error variant: {other:?}"),
    }
}

#[test]
fn add_delta_subtract_underflow_traps() {
    // Subtracting more than `x` underflows the unsigned subtraction. Same
    // dual-mode behavior as overflow: debug panics, release wraps and the
    // `require_gt!` catches.
    let result = std::panic::catch_unwind(|| add_delta(0, -1));
    match result {
        Err(_) => { /* debug: panic on underflow */ }
        Ok(Err(ErrorCode::LiquiditySubValueErr)) => { /* release: caught */ }
        Ok(Ok(v)) => panic!("add_delta(0, -1) silently returned {v}"),
        Ok(Err(other)) => panic!("unexpected error variant: {other:?}"),
    }
}

// ---- get_liquidity_from_amounts round-trip ----

#[test]
fn liquidity_from_amounts_round_trips_with_delta_amounts() {
    // For a position that spans [tick_a, tick_b] around current_tick:
    //   amounts → liquidity → amounts should stay within ±1 of the input
    //   (rounding is direction-specific).
    let tick_lower = -100;
    let tick_upper = 100;
    let tick_current = 0;
    let amount_0 = 1_000_000u64;
    let amount_1 = 1_000_000u64;
    let sp_lower = get_sqrt_price_at_tick(tick_lower).unwrap();
    let sp_upper = get_sqrt_price_at_tick(tick_upper).unwrap();
    let sp_current = get_sqrt_price_at_tick(tick_current).unwrap();

    let l = get_liquidity_from_amounts(sp_current, sp_lower, sp_upper, amount_0, amount_1);
    assert!(l > 0, "liquidity should be positive for non-zero amounts");

    let recovered_0 = get_delta_amount_0_unsigned(sp_current, sp_upper, l, false).unwrap();
    let recovered_1 = get_delta_amount_1_unsigned(sp_lower, sp_current, l, false).unwrap();

    // get_liquidity_from_amounts takes the MIN of (L_from_amount_0, L_from_amount_1),
    // so one side will be ≤ input and the other side will be ≤ input by potentially
    // a larger margin (the binding constraint exhausts; the other doesn't).
    assert!(
        recovered_0 <= amount_0,
        "recovered amount_0 {} > input {}",
        recovered_0,
        amount_0
    );
    assert!(
        recovered_1 <= amount_1,
        "recovered amount_1 {} > input {}",
        recovered_1,
        amount_1
    );
    // At least one side must be tight (within rounding) — the binding side.
    let tight_0 = amount_0 - recovered_0 <= 1;
    let tight_1 = amount_1 - recovered_1 <= 1;
    assert!(
        tight_0 || tight_1,
        "neither amount round-trips tight: amount_0 lost {}, amount_1 lost {}",
        amount_0 - recovered_0,
        amount_1 - recovered_1
    );
}

#[test]
fn delta_amounts_zero_when_prices_equal() {
    // sp_a == sp_b → no price range, no token amount on either side.
    let sp = get_sqrt_price_at_tick(0).unwrap();
    let liq = 1_000_000u128;
    assert_eq!(get_delta_amount_0_unsigned(sp, sp, liq, false).unwrap(), 0);
    assert_eq!(get_delta_amount_1_unsigned(sp, sp, liq, false).unwrap(), 0);
}

#[test]
fn delta_amounts_round_up_geq_round_down() {
    // For the same range and liquidity, round-up output >= round-down output.
    let sp_a = get_sqrt_price_at_tick(-100).unwrap();
    let sp_b = get_sqrt_price_at_tick(100).unwrap();
    let liq = 1_000_000_000u128;

    let amt0_up = get_delta_amount_0_unsigned(sp_a, sp_b, liq, true).unwrap();
    let amt0_down = get_delta_amount_0_unsigned(sp_a, sp_b, liq, false).unwrap();
    assert!(amt0_up >= amt0_down);
    assert!(amt0_up - amt0_down <= 1);

    let amt1_up = get_delta_amount_1_unsigned(sp_a, sp_b, liq, true).unwrap();
    let amt1_down = get_delta_amount_1_unsigned(sp_a, sp_b, liq, false).unwrap();
    assert!(amt1_up >= amt1_down);
    assert!(amt1_up - amt1_down <= 1);
}

// ---- get_delta_amount_*_unsigned: value-pinned (issue #7) ----
//
// Captured from the byte-exact-extracted math; if any of these change, the
// arithmetic has drifted. Round-up = round-down + 0 or 1 always (also
// asserted as a property in tests/properties.rs).

#[test]
fn delta_amounts_pinned_unit_liquidity_one_tick() {
    // t=[0, 1], L=1: sub-unit fractions get rounded to 0 (down) or 1 (up).
    let sp_lo = get_sqrt_price_at_tick(0).unwrap();
    let sp_hi = get_sqrt_price_at_tick(1).unwrap();
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, 1, false).unwrap(),
        0
    );
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, 1, true).unwrap(),
        1
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, 1, false).unwrap(),
        0
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, 1, true).unwrap(),
        1
    );
}

#[test]
fn delta_amounts_pinned_1e15_liquidity_one_tick() {
    // t=[0, 1], L=1e15. sp_lo = Q64 = 2^64. The amount_0 and amount_1
    // formulas differ slightly so the values aren't identical:
    //   Δy ≈ L * (sp_hi/Q64 - 1)  ≈ 1e15 * (1.0001^0.5 - 1)
    //   Δx ≈ L * (sp_hi/Q64 - 1) / (sp_hi/Q64) — the 1/sp_hi factor makes Δx slightly smaller.
    let sp_lo = get_sqrt_price_at_tick(0).unwrap();
    let sp_hi = get_sqrt_price_at_tick(1).unwrap();
    let liq: u128 = 1_000_000_000_000_000;
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, liq, false).unwrap(),
        49_996_250_312
    );
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, liq, true).unwrap(),
        49_996_250_313
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, liq, false).unwrap(),
        49_998_750_062
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, liq, true).unwrap(),
        49_998_750_063
    );
}

#[test]
fn delta_amounts_pinned_1e18_liquidity_symmetric_range() {
    // Symmetric range ([-100, 100]) at L=1e18. Δx and Δy are equal here
    // because the symmetry of the price range makes the integrals match.
    let sp_lo = get_sqrt_price_at_tick(-100).unwrap();
    let sp_hi = get_sqrt_price_at_tick(100).unwrap();
    let liq: u128 = 1_000_000_000_000_000_000;
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, liq, false).unwrap(),
        9_999_541_693_797_069
    );
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, liq, true).unwrap(),
        9_999_541_693_797_070
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, liq, false).unwrap(),
        9_999_541_693_797_069
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, liq, true).unwrap(),
        9_999_541_693_797_070
    );
}

#[test]
fn delta_amounts_args_swapped_yields_same() {
    // The functions internally swap args so sqrt_a < sqrt_b; passing them
    // in either order must yield identical output.
    let sp_a = get_sqrt_price_at_tick(50).unwrap();
    let sp_b = get_sqrt_price_at_tick(150).unwrap();
    let liq: u128 = 1_000_000_000_000;
    assert_eq!(
        get_delta_amount_0_unsigned(sp_a, sp_b, liq, false).unwrap(),
        get_delta_amount_0_unsigned(sp_b, sp_a, liq, false).unwrap(),
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_a, sp_b, liq, true).unwrap(),
        get_delta_amount_1_unsigned(sp_b, sp_a, liq, true).unwrap(),
    );
}

#[test]
fn delta_amounts_overflow_at_max_liquidity() {
    // L = u128::MAX over a wide range overflows the u64 token amount —
    // verifies the MaxTokenOverflow error path on both functions.
    let sp_lo = get_sqrt_price_at_tick(-1000).unwrap();
    let sp_hi = get_sqrt_price_at_tick(1000).unwrap();
    assert_eq!(
        get_delta_amount_0_unsigned(sp_lo, sp_hi, u128::MAX, false).unwrap_err(),
        ErrorCode::MaxTokenOverflow,
    );
    assert_eq!(
        get_delta_amount_1_unsigned(sp_lo, sp_hi, u128::MAX, true).unwrap_err(),
        ErrorCode::MaxTokenOverflow,
    );
}

// ---- cross() (issue #3) ----

#[test]
fn cross_adds_liquidity_net_when_moving_up() {
    // zero_for_one=false → moving up the price, add liquidity_net as-is.
    let result = solana_clmm_raydium::cross(1_000, 250, false).unwrap();
    assert_eq!(result, 1_250);
}

#[test]
fn cross_subtracts_liquidity_net_when_moving_down() {
    // zero_for_one=true → moving down, subtract liquidity_net.
    let result = solana_clmm_raydium::cross(1_000, 250, true).unwrap();
    assert_eq!(result, 750);
}

#[test]
fn cross_handles_negative_liquidity_net() {
    // Negative liquidity_net (typical at upper boundary of a position).
    assert_eq!(solana_clmm_raydium::cross(1_000, -250, false).unwrap(), 750);
    assert_eq!(
        solana_clmm_raydium::cross(1_000, -250, true).unwrap(),
        1_250
    );
}

#[test]
fn cross_overflow_traps() {
    // `cross` delegates to `add_delta`, which has dual-mode behavior:
    // debug panics on the unsigned add/sub, release wraps and the require!
    // macro catches. Either outcome is "overflow detected" — the failure
    // we guard against is a silent wrong return.
    let underflow = std::panic::catch_unwind(|| solana_clmm_raydium::cross(100, -200, false));
    match underflow {
        Err(_) => {}
        Ok(Err(ErrorCode::LiquiditySubValueErr)) => {}
        Ok(other) => panic!("cross(100, -200, false) returned {other:?}, expected error"),
    }
    let overflow = std::panic::catch_unwind(|| solana_clmm_raydium::cross(u128::MAX, 1, false));
    match overflow {
        Err(_) => {}
        Ok(Err(ErrorCode::LiquidityAddValueErr)) => {}
        Ok(other) => panic!("cross(u128::MAX, 1, false) returned {other:?}, expected error"),
    }
}

// ---- ErrorCode Display + Error (issue #2) ----

#[test]
fn error_code_display_returns_reason_string() {
    use core::error::Error;
    let e = ErrorCode::MaxTokenOverflow;
    let formatted = format!("{e}");
    assert_eq!(formatted, e.reason());
    // core::error::Error trait is implemented (compile-time check).
    let _: &dyn Error = &e;
}
