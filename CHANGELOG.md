# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Token-2022 transfer-fee math** (`transfer_fee` module). New public API:
  `TransferFee`, `calculate_fee`, `apply_transfer_fee`,
  `reverse_apply_transfer_fee`, `MAX_FEE_BASIS_POINTS`. Mirrors
  `spl_token_2022_interface::extension::transfer_fee` byte-exactly,
  including all boundary cases (cap, 100% fee, ceil-rounding) and the
  documented round-trip inequality for exact-out routing. Test vectors
  ported verbatim from the spl reference suite.

### Changed
- `Out of scope` narrowed: Token-2022 transfer-fee math is now in scope;
  mint-extension TLV decoding and transfer-hook CPI remain out of scope
  (caller resolves the active fee for the current epoch).

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

[Unreleased]: https://github.com/ACNoonan/solana-clmm-raydium/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ACNoonan/solana-clmm-raydium/releases/tag/v0.1.0
