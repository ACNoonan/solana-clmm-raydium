# solana-clmm-raydium

Pure-Rust, no-RPC swap math for the [Raydium](https://github.com/raydium-io/raydium-clmm)
concentrated-liquidity AMM (CLMM) on Solana.

This crate is the deterministic integer arithmetic that the on-chain Raydium
program executes — extracted unchanged into a library that has **no** dependency
on `anchor-lang`, `solana-program`, the Solana runtime, or the Anchor account
model. Given pre-decoded pool state and tick-array data, every function here is
a pure function of its inputs.

It is the missing analogue of [`uniswap_v3_math`](https://crates.io/crates/uniswap_v3_math)
for Solana CLMMs. Use it for backtesting, simulation, MEV/LVR research, route
finding, or anywhere you want to know the exact swap output of a Raydium pool
without round-tripping through the chain.

## Status

`v0.1.0`. Math compiles, the published Raydium boundary constants
(`MIN_SQRT_PRICE_X64`, `MAX_SQRT_PRICE_X64`) round-trip exactly, and the
crate-root smoke tests pass. Mainnet historical-swap differential tests are in
progress — assume bugs until those land.

## Quickstart

```rust
use solana_clmm_raydium::{
    get_sqrt_price_at_tick, get_tick_at_sqrt_price, compute_swap_step,
};

// tick → sqrt-price (Q64.64)
let sqrt_price = get_sqrt_price_at_tick(1_000)?;

// sqrt-price → tick (round-trip is exact across the full tick domain)
let tick = get_tick_at_sqrt_price(sqrt_price)?;
assert_eq!(tick, 1_000);

// Single-tick swap step. Caller is responsible for walking tick arrays
// and feeding successive `compute_swap_step` calls until the swap settles.
let step = compute_swap_step(
    sqrt_price_current_x64,
    sqrt_price_target_x64,
    liquidity,
    amount_remaining,
    fee_pips,
    is_base_input,
    zero_for_one,
    block_timestamp,
)?;
```

## Scope

**In scope.** Tick ↔ sqrt-price, liquidity ↔ token-amount, single-tick swap
step, tick-array bitmap navigation. See the
[crate-level docs](src/lib.rs) for the full curated public API.

**Out of scope.**
- Pool / tick-array account decoding — this crate takes pre-decoded state.
- Multi-tick `compute_swap_full` orchestration — composing it requires fetching
  tick-array accounts at runtime; that is the consumer's job.
- Token-2022 transfer-fee / transfer-hook accounting.
- Position fee and reward accumulation beyond `liquidity_from_amounts`.

## Provenance

Math is extracted from
[`raydium-io/raydium-clmm`](https://github.com/raydium-io/raydium-clmm)
`programs/amm/src/libraries/`. The arithmetic is byte-for-byte identical to the
on-chain implementation. The only changes are:

1. Import-path rewrites (`crate::libraries::*` → crate-root paths).
2. A small internal `ErrorCode` enum that replaces `anchor_lang::error::Error`.
3. Free-function rehosting of three static methods (`tick_count`,
   `get_array_start_index`, `check_is_out_of_boundary`,
   `check_is_valid_start_index`) that used no struct fields.

Upstream tests (`#[cfg(test)]` blocks using `proptest` / `quickcheck`) were
removed in favor of new tests anchored to mainnet ground-truth.

## License

Dual-licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
