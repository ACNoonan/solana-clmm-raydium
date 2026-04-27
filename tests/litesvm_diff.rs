//! Litesvm differential test scaffolding.
//!
//! Goal: load the on-chain Raydium CLMM program ELF into an in-process VM,
//! execute the same swap inputs through both (a) the real on-chain program
//! and (b) our extracted [`compute_swap_full`], assert byte-exact match.
//! When this lands, we no longer need fresh mainnet replay fixtures to
//! verify correctness — fuzzing the inputs catches drift the moment a
//! Raydium program upgrade introduces a math change.
//!
//! See the audit (`docs/audits/v0.1.0-external-review.md` §4.8) for
//! background. Reference impl pattern:
//! `MeteoraAg/dlmm-sdk/commons/tests/integration/test_swap.rs`.
//!
//! ## Status
//!
//! - **Smoke test** ([`smoke_program_loads`]) — verifies the ELF loads
//!   into litesvm and the program account is registered as executable.
//!   Catches regressions in the dev-dep graph or the ELF artifact itself.
//!
//! - **Differential test** ([`differential_swap_byte_exact`]) — `#[ignore]`
//!   placeholder. The remaining work is encoder-side: turning
//!   `tests/support/decode.rs`'s read-side decoders into matching
//!   write-side encoders for `PoolState`, `TickArrayState`, `AmmConfig`,
//!   and `ObservationState`, plus SPL mint / vault / ATA setup, plus
//!   building the Anchor `swap_v2` instruction with correct account metas
//!   and discriminator. Tracked in the v0.3 milestone (#8).

mod support;

use std::path::PathBuf;

use litesvm::LiteSVM;
use support::raydium::PROGRAM_ID;

/// Path to the program ELF. Captured once via `getAccountInfo` on the
/// upgradeable programdata account at slot ~415M; see commit history for
/// the fetch script. Excluded from `cargo publish` via `package.exclude`.
fn program_elf_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raydium_clmm.so")
}

#[test]
fn smoke_program_loads() {
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(PROGRAM_ID, program_elf_path())
        .expect("ELF loads");

    let acct = svm
        .get_account(&PROGRAM_ID)
        .expect("program account registered after add_program_from_file");
    assert!(acct.executable, "program account must be executable");
    assert!(
        acct.data.len() > 1_000_000,
        "ELF should be ~1.7MB; got {} bytes — wrong file?",
        acct.data.len()
    );
}

#[test]
#[ignore = "requires v0.3 differential-test infrastructure (see TODOs)"]
fn differential_swap_byte_exact() {
    // TODO(v0.3, audit §4.8) — drive a single swap through both the
    // on-chain program (in litesvm) and our `compute_swap_full`, assert
    // byte-exact match on amount_in / amount_out / final sqrt_price.
    //
    // Remaining work, ordered:
    //
    //   1. Encoders for synthesized account state. `tests/support/decode.rs`
    //      has the read side; we need symmetric `to_bytes()` for at least:
    //
    //        - `PoolState` (~480 B, packed Anchor layout, references
    //          amm_config / mints / vaults / observation_key)
    //        - `TickArrayState` (10240 B, 60-tick array)
    //        - `AmmConfig` (~120 B, contains fee_rate)
    //        - `ObservationState` (oracle ring buffer; can likely start
    //          as zeroed bytes — Raydium writes more than it reads)
    //
    //      Each needs the right Anchor 8-byte discriminator
    //      (`sha256("account:<TypeName>")[..8]`).
    //
    //   2. Token setup. SPL Token program is built into litesvm, so:
    //
    //        - Initialize mints at mint_0 / mint_1 with matching decimals.
    //        - Create vaults (token accounts owned by the pool's authority
    //          PDA) with large balances.
    //        - Create the user's input/output ATAs and fund the input.
    //
    //   3. Instruction. Anchor `swap` discriminator is
    //      `sha256("global:swap")[..8]`; data layout is `amount: u64,
    //      other_amount_threshold: u64, sqrt_price_limit_x64: u128,
    //      is_base_input: bool` (16 + 1 = 17 bytes after the disc).
    //      Account metas in instruction order — see
    //      `raydium-io/raydium-clmm/programs/amm/src/instructions/swap.rs`.
    //      Tick arrays go in `remaining_accounts`.
    //
    //   4. Comparison. After execution, read post-state lamports/balances
    //      and pool sqrt_price; run the same `(pool, ticks, amount, ...)`
    //      through `compute_swap_full`; assert each field matches.
    //
    //   5. Fuzz. Once one case works, parameterize over (tick_current,
    //      liquidity, amount, zero_for_one, is_base_input) via proptest
    //      with shrinking. This is what closes #8's litesvm bullet.

    let mut svm = LiteSVM::new();
    svm.add_program_from_file(PROGRAM_ID, program_elf_path())
        .expect("ELF loads");
    panic!("not implemented — see TODO list above");
}
