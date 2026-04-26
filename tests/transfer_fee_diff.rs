//! Differential proptest: locks `transfer_fee` byte-exact parity against the
//! canonical Token-2022 implementation in `spl_token_2022_interface`.
//!
//! Strategy: for every sampled `(bps, max_fee, amount)`, run both
//! implementations and assert their outputs match (mapping our
//! `Result<u64, ErrorCode>` to upstream's `Option<u64>` by collapsing all
//! errors to `None`). If upstream patches its math, this test will surface
//! the drift on the next dev build.
//!
//! This is the test the v0.1 audit would have wanted us to ship alongside
//! the verbatim-ported unit vectors. Run with `cargo test --test
//! transfer_fee_diff`.

use proptest::prelude::*;
use solana_clmm_raydium::{
    apply_transfer_fee, calculate_fee, reverse_apply_transfer_fee, TransferFee,
    MAX_FEE_BASIS_POINTS,
};
use spl_pod::primitives::{PodU16, PodU64};
use spl_token_2022_interface::extension::transfer_fee::TransferFee as SplTransferFee;

fn ours(bps: u16, maximum_fee: u64) -> TransferFee {
    TransferFee {
        transfer_fee_basis_points: bps,
        maximum_fee,
    }
}

fn theirs(bps: u16, maximum_fee: u64) -> SplTransferFee {
    SplTransferFee {
        epoch: PodU64::from(0),
        maximum_fee: PodU64::from(maximum_fee),
        transfer_fee_basis_points: PodU16::from(bps),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        // 4096 cases per test ≈ ~30s total; light enough for CI, dense enough
        // to catch drift in any branch (cap, 100% fee, ceil-rounding, zeros).
        cases: 4096,
        ..ProptestConfig::default()
    })]

    #[test]
    fn diff_calculate_fee(
        bps in 0u16..=MAX_FEE_BASIS_POINTS,
        maximum_fee in any::<u64>(),
        amount in any::<u64>(),
    ) {
        let ours = calculate_fee(&ours(bps, maximum_fee), amount).ok();
        let theirs = theirs(bps, maximum_fee).calculate_fee(amount);
        prop_assert_eq!(ours, theirs);
    }

    #[test]
    fn diff_apply_transfer_fee(
        bps in 0u16..=MAX_FEE_BASIS_POINTS,
        maximum_fee in any::<u64>(),
        amount in any::<u64>(),
    ) {
        let ours = apply_transfer_fee(&ours(bps, maximum_fee), amount).ok();
        let theirs = theirs(bps, maximum_fee).calculate_post_fee_amount(amount);
        prop_assert_eq!(ours, theirs);
    }

    #[test]
    fn diff_reverse_apply_transfer_fee(
        bps in 0u16..=MAX_FEE_BASIS_POINTS,
        maximum_fee in any::<u64>(),
        post_fee_amount in any::<u64>(),
    ) {
        let ours = reverse_apply_transfer_fee(&ours(bps, maximum_fee), post_fee_amount).ok();
        let theirs = theirs(bps, maximum_fee).calculate_pre_fee_amount(post_fee_amount);
        prop_assert_eq!(ours, theirs);
    }
}
