//! End-to-end wiring test for Token-2022 transfer fees around
//! `compute_swap_step`. Mirrors the README's "Token-2022 transfer fees"
//! snippet on a synthetic pool so we get a concrete, RPC-free regression.
//!
//! The math itself is verified separately:
//! - `tests/transfer_fee_diff.rs` locks `transfer_fee` against
//!   `spl_token_2022_interface` byte-for-byte across the full domain.
//! - `tests/replay.rs` replays real Raydium SOL/USDC swaps end-to-end.
//!
//! What this file adds: confidence that the *composition* — input-side
//! `apply_transfer_fee` → `compute_swap_step` → output-side
//! `apply_transfer_fee` — is wired in the right order with no
//! double-application or off-by-one. That's the only thing real-mainnet
//! Token-2022 fixtures would catch beyond the differential proptest, and
//! none of the active Raydium CLMM pools use `TransferFeeConfig` without
//! also requiring a non-replayable `TransferHook` (see CHANGELOG note).

use solana_clmm_raydium::{
    apply_transfer_fee, compute_swap_step, get_sqrt_price_at_tick, reverse_apply_transfer_fee,
    SwapStep, TransferFee,
};

const POOL_FEE_PPM: u32 = 100; // 0.01% pool fee
const LIQUIDITY: u128 = 10_000_000_000_000_000;
const USER_AMOUNT_IN: u64 = 100_000_000;

fn no_fee() -> TransferFee {
    TransferFee {
        transfer_fee_basis_points: 0,
        maximum_fee: 0,
    }
}

fn fee(bps: u16, maximum_fee: u64) -> TransferFee {
    TransferFee {
        transfer_fee_basis_points: bps,
        maximum_fee,
    }
}

/// Wire transfer fees around `compute_swap_step` exactly as the README
/// snippet shows: input-side fee → pool sees post-fee amount → swap step →
/// output-side fee → user sees post-fee amount.
fn swap_with_fees(fee_in: TransferFee, fee_out: TransferFee) -> (SwapStep, u64, u64) {
    let sp_current = get_sqrt_price_at_tick(0).unwrap();
    let sp_target = get_sqrt_price_at_tick(1_000).unwrap();
    let pool_amount_in = apply_transfer_fee(&fee_in, USER_AMOUNT_IN).unwrap();
    let step = compute_swap_step(
        sp_current,
        sp_target,
        LIQUIDITY,
        pool_amount_in,
        POOL_FEE_PPM,
        /* is_base_input */ true,
        /* zero_for_one  */ false,
        /* block_timestamp */ 0,
    )
    .unwrap();
    let user_amount_out = apply_transfer_fee(&fee_out, step.amount_out).unwrap();
    (step, pool_amount_in, user_amount_out)
}

#[test]
fn zero_fees_are_identity() {
    let (step, pool_in, user_out) = swap_with_fees(no_fee(), no_fee());
    assert_eq!(pool_in, USER_AMOUNT_IN);
    assert_eq!(user_out, step.amount_out);
}

#[test]
fn input_fee_reduces_pool_input_by_exact_transfer_fee() {
    let f_in = fee(30, u64::MAX); // 0.3%, no cap
    let (_step, pool_in, _) = swap_with_fees(f_in, no_fee());

    // 30 bps of 100M, ceil rounded = 300_000. Pool sees 99_700_000.
    assert_eq!(pool_in, 99_700_000);
    assert_eq!(USER_AMOUNT_IN - pool_in, 300_000);
}

#[test]
fn output_fee_reduces_user_amount_by_exact_transfer_fee() {
    let f_out = fee(50, u64::MAX); // 0.5%, no cap
    let (step, _, user_out) = swap_with_fees(no_fee(), f_out);

    let expected_user_out = apply_transfer_fee(&f_out, step.amount_out).unwrap();
    assert_eq!(user_out, expected_user_out);
    // Sanity: the user gets strictly less than the pool produced.
    assert!(user_out < step.amount_out);
    // 50 bps of step.amount_out, ceil. The exact number is liquidity-
    // dependent, so we only assert the bps relation, not a magic constant.
    let withheld = step.amount_out - user_out;
    let expected_withheld = (step.amount_out as u128 * 50).div_ceil(10_000) as u64;
    assert_eq!(withheld, expected_withheld);
}

#[test]
fn cap_dominates_when_amount_is_large_enough() {
    // Cap fee_out at 1_000 base units. Output is well over 200k base units,
    // so 50 bps would yield ~1k+; the cap should kick in.
    let f_out = fee(50, 1_000);
    let (step, _, user_out) = swap_with_fees(no_fee(), f_out);
    let withheld = step.amount_out - user_out;
    assert!(
        withheld <= 1_000,
        "cap should bound withheld amount; got {withheld} > 1_000"
    );
    // And if the bps-derived fee would have exceeded the cap, we should be
    // exactly at the cap.
    let raw_bps_fee = (step.amount_out as u128 * 50).div_ceil(10_000) as u64;
    if raw_bps_fee > 1_000 {
        assert_eq!(withheld, 1_000);
    }
}

#[test]
fn double_apply_is_caught_by_wiring_check() {
    // Sanity guard: if a future refactor accidentally applies the input fee
    // twice (e.g. once in a wrapper, once in the caller), the result would
    // diverge from a single-apply baseline. This test would fail loudly.
    let f_in = fee(50, u64::MAX);
    let single = apply_transfer_fee(&f_in, USER_AMOUNT_IN).unwrap();
    let doubled = apply_transfer_fee(&f_in, single).unwrap();
    assert_ne!(single, doubled);
    // The intended `swap_with_fees` shape applies fee_in exactly once.
    let (_step, pool_in, _) = swap_with_fees(f_in, no_fee());
    assert_eq!(pool_in, single, "wiring must apply input fee exactly once");
}

#[test]
fn exact_out_via_reverse_apply_routes_user_to_target() {
    // Exact-out routing: user wants to *receive* `target_user_out` post-fee.
    // We size the swap so the pool produces `reverse_apply(target_user_out)`.
    let f_out = fee(50, u64::MAX);
    let target_user_out: u64 = 1_000_000;
    let target_pool_out = reverse_apply_transfer_fee(&f_out, target_user_out).unwrap();
    // Round-trip: applying the fee to the pool output must land at — or one
    // base unit above — the user's target (Token-2022 ceil-rounding can
    // make the pool overproduce by a single unit; never underproduce).
    let recovered_user_out = apply_transfer_fee(&f_out, target_pool_out).unwrap();
    assert!(
        recovered_user_out >= target_user_out,
        "exact-out routing must never under-deliver: target={target_user_out} got={recovered_user_out}",
    );
    assert!(
        recovered_user_out - target_user_out <= 1,
        "exact-out over-delivery should be at most 1 base unit",
    );
}
