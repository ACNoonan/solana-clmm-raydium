# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-04-27

The "everything the audit and v0.1 issue tracker called out" release. New
multi-tick orchestrator, Token-2022 transfer fees, two breaking signature
fixes, MSRV 1.81. Closes audit issues #1–#7 and partially closes #8 (the
v0.3 tracker — Token-2022 bullet done; Litesvm + DLMM remain).

### Added
- **`compute_swap_full` multi-tick orchestrator** (#1, single most-asked
  v0.2 feature per the audit). New `swap_full` module with `SwapPool`,
  `InitializedTick`, `SwapResult`. Lifts the multi-tick walking loop out
  of `tests/replay.rs` into the public surface; mainnet replay test now
  uses the public function and reproduces all 17 fixtures byte-exact.
- **Token-2022 transfer-fee math** (`transfer_fee` module). New public
  API: `TransferFee`, `calculate_fee`, `apply_transfer_fee`,
  `reverse_apply_transfer_fee`, `MAX_FEE_BASIS_POINTS`. Mirrors
  `spl_token_2022_interface::extension::transfer_fee` byte-exactly,
  with verbatim-ported test vectors and a 4096-case-per-function
  differential proptest against the upstream impl.
- **`cross()` helper** (#3) — promote the tick-cross liquidity update
  primitive (`add_delta(L, ±liquidity_net)`) from `tests/replay.rs` to
  a public function. Every multi-tick walker reimplemented this.
- **`ErrorCode` Display + `core::error::Error`** (#2). Each variant has a
  stable `reason()` string. Lets consumers `?`-bubble through
  `anyhow::Error` and surfaces readable panic messages.
- **Hoisted re-exports** (#6, "pub by accident" cleanup):
  `get_next_sqrt_price_from_input`/`_output`,
  `get_next_sqrt_price_from_amount_0_rounding_up`,
  `get_next_sqrt_price_from_amount_1_rounding_down`,
  `get_delta_amount_0_signed`/`_1_signed`,
  `get_liquidity_from_amount_0`/`_1`,
  `check_current_tick_array_is_initialized`. All previously
  module-accessible only.
- **Value-pinned unit tests** for `get_delta_amount_0/1_unsigned` (#7) —
  five tests with concrete expected values across a unit-liquidity edge,
  the `1e15` regime at one-tick range, the `1e18` symmetric range, and
  the `MaxTokenOverflow` error path.
- **End-to-end Token-2022 wiring test** (`tests/transfer_fee_swap.rs`):
  six tests composing `apply_transfer_fee` → `compute_swap_step` →
  `apply_transfer_fee` against a synthetic pool, catching
  double-application and ordering bugs without RPC.

### Changed
- **MSRV 1.75 → 1.81** for `core::error::Error` (stabilized in 1.81).
- **README quickstart restructured**: `compute_swap_full` is now the
  recommended entry point; `compute_swap_step` + `cross` documented as
  the lower-level escape hatch. New "Token-2022 transfer fees" section.
- Scope narrowed: Token-2022 transfer-fee math + multi-tick orchestration
  are now in scope; mint-extension TLV decoding and transfer-hook CPI
  remain out of scope (caller resolves the active fee for the current
  epoch and CPIs hook programs).

### Breaking
- **`compute_swap_step` no longer takes `block_timestamp: u32`** (#5).
  Drop the trailing argument from any callsite. The parameter was unused —
  it existed only because upstream's `#[cfg(test)]` helper branched on
  it; the production body that we ship doesn't read it. All 17 mainnet
  replay fixtures still match byte-exact, confirming it was dead code.
- **`next_initialized_tick_array_start_index` and
  `check_current_tick_array_is_initialized` now take `&PoolTickBitmap`**
  instead of `U1024` (#4). `PoolTickBitmap` is a public newtype wrapping
  `[u64; 16]` with `From<[u64; 16]>`, `PoolTickBitmap::EMPTY`, and a
  `.limbs()` accessor. `U1024` returns to internal-only.
- **`tick_array_bit_map::most_significant_bit` / `least_significant_bit`
  demoted to `pub(crate)`** — they would re-leak `U1024` in their public
  signatures. Anyone calling these needs to inline the trivial
  `leading_zeros` / `trailing_zeros` themselves.

### Notes
- **Mainnet replay for transfer-fee swaps is not shipped.** A scan of
  the top 500 Raydium CLMM pools by 24h volume finds 90 with at least
  one Token-2022 mint but only three with a `TransferFeeConfig`
  *without* a non-replayable `TransferHook` (`WSOL/LAUNCHCOIN`,
  `SEAS/USDC`, `WSOL/SOS`, all sub-$10k/day). The high-volume Token-2022
  mints (HRP, TrumpPepe, WCOR…) carry no extensions and behave
  identically to SPL Token. Math parity is locked via the differential
  proptest; the wiring test covers composition.

## [0.1.0] — 2026-04-26

Initial release. Pure-Rust, no-RPC swap math for the Raydium concentrated-
liquidity AMM (CLMM) on Solana, extracted byte-for-byte from
[`raydium-io/raydium-clmm`](https://github.com/raydium-io/raydium-clmm)
`programs/amm/src/libraries/`.

### Added
- **Curated public API**: `compute_swap_step`, `get_sqrt_price_at_tick`,
  `get_tick_at_sqrt_price`, `get_liquidity_from_amounts`,
  `get_delta_amounts_signed`, `next_initialized_tick_array_start_index`,
  free-function helpers for tick-array geometry, plus the constants
  `MIN_TICK`, `MAX_TICK`, `MIN_SQRT_PRICE_X64`, `MAX_SQRT_PRICE_X64`,
  `TICK_ARRAY_SIZE`, `TICK_ARRAY_BITMAP_SIZE`, `FEE_RATE_DENOMINATOR_VALUE`.
- **Internal `ErrorCode`** replacing `anchor_lang::error::Error`. No Anchor,
  `solana-program`, or runtime dependency.
- **Tests**: full-domain (887k-tick) round-trip, monotonicity, swap-step
  invariants under `proptest`, plus a mainnet replay harness that snapshots
  pool + tick-array state via Helius and asserts byte-exact match on
  `amount_in` and `amount_out` for real swaps.

### Notes on the extraction
- The arithmetic itself is unmodified. Only the surrounding scaffolding
  changed: import-path rewrites, error handling, and free-function rehosting
  of three static methods on `TickState` / `TickArrayState` that used no
  struct fields.
- `MAX_TICK` is **one-way** in the inverse: `get_sqrt_price_at_tick(MAX_TICK)`
  returns `MAX_SQRT_PRICE_X64`, but `get_tick_at_sqrt_price(MAX_SQRT_PRICE_X64)`
  errors. This matches Uniswap V3's convention — `MAX_SQRT_PRICE_X64` is the
  unattainable upper bound.

### Out of scope (deferred to v0.2)
- Pool / tick-array account decoding (this crate takes pre-decoded state).
- Multi-tick `compute_swap_full` orchestration (composing the primitive
  requires fetching tick-array accounts; that is the consumer's job).
- Token-2022 mint-extension TLV decoding and transfer-hook CPI (the
  transfer-fee math itself ships in 0.2).
- Position fee / reward accumulation beyond `liquidity_from_amounts`.
- Litesvm differential test (replays against a forked program).

[Unreleased]: https://github.com/ACNoonan/solana-clmm-raydium/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/ACNoonan/solana-clmm-raydium/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ACNoonan/solana-clmm-raydium/releases/tag/v0.1.0
