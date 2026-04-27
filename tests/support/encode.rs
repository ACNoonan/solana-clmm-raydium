//! Write-side encoders for the litesvm differential test.
//!
//! Symmetric to `decode.rs`: produces byte-exact account images for the
//! Raydium CLMM Anchor accounts (`AmmConfig`, `ObservationState`) and the
//! supporting SPL Token accounts (`Mint`, `TokenAccount`) so we can inject
//! synthesized state into a `LiteSVM` instance via `set_account`.
//!
//! Layouts pinned against:
//! - `raydium-io/raydium-clmm/programs/amm/src/states/{config,oracle}.rs`
//!   (mainnet ELF captured at `tests/fixtures/raydium_clmm.so`)
//! - `solana-program-library/token/program/src/state.rs` for SPL.

#![allow(dead_code)] // some helpers are scenario-specific; not all tests use all

use super::raydium::{AMM_CONFIG_DISC, OBSERVATION_STATE_DISC, TICK_ARRAY_STATE_DISC};

// ---- AmmConfig ----
//
// Byte layout (Anchor `#[account]`, Borsh; 117 bytes total):
//   8   discriminator
//   1   bump
//   2   index
//   32  owner
//   4   protocol_fee_rate
//   4   trade_fee_rate
//   2   tick_spacing
//   4   fund_fee_rate
//   4   padding_u32
//   32  fund_owner
//   24  padding [u64; 3]

pub const AMM_CONFIG_LEN: usize = 117;

pub fn ammconfig_bytes(
    bump: u8,
    index: u16,
    owner: &[u8; 32],
    protocol_fee_rate: u32,
    trade_fee_rate: u32,
    tick_spacing: u16,
    fund_fee_rate: u32,
    fund_owner: &[u8; 32],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(AMM_CONFIG_LEN);
    out.extend_from_slice(&AMM_CONFIG_DISC);
    out.push(bump);
    out.extend_from_slice(&index.to_le_bytes());
    out.extend_from_slice(owner);
    out.extend_from_slice(&protocol_fee_rate.to_le_bytes());
    out.extend_from_slice(&trade_fee_rate.to_le_bytes());
    out.extend_from_slice(&tick_spacing.to_le_bytes());
    out.extend_from_slice(&fund_fee_rate.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // padding_u32
    out.extend_from_slice(fund_owner);
    out.extend_from_slice(&[0u8; 24]); // padding [u64; 3]
    debug_assert_eq!(out.len(), AMM_CONFIG_LEN);
    out
}

// ---- ObservationState ----
//
// Byte layout (Anchor `#[account(zero_copy(unsafe))] #[repr(C, packed)]`):
//   8     discriminator
//   1     initialized (bool)
//   8     recent_epoch
//   2     observation_index
//   32    pool_id
//   4400  observations[100] of Observation { u32 + i64 + [u64;4] = 44 each }
//   32    padding [u64; 4]
//   ----
//   4483 bytes total

pub const OBSERVATION_LEN: usize = 8 + 1 + 8 + 2 + 32 + 44 * 100 + 32;

/// Produces a zero-initialized `ObservationState` with only the discriminator
/// and `pool_id` set. Raydium writes oracle observations during a swap but
/// only reads its own state; an uninitialized buffer should accept writes.
pub fn observation_state_bytes(pool_id: &[u8; 32]) -> Vec<u8> {
    let mut out = vec![0u8; OBSERVATION_LEN];
    out[0..8].copy_from_slice(&OBSERVATION_STATE_DISC);
    // initialized: bool = false (already 0)
    // recent_epoch: u64 = 0
    // observation_index: u16 = 0
    // pool_id at offset 8 + 1 + 8 + 2 = 19
    out[19..51].copy_from_slice(pool_id);
    debug_assert_eq!(out.len(), OBSERVATION_LEN);
    out
}

// ---- Empty TickArrayState ----
//
// 10240 bytes:
//   8       discriminator
//   32      pool_id
//   4       start_tick_index
//   168*60  ticks (all zeroed = uninitialized)
//   1       initialized_tick_count = 0
//   8       recent_epoch
//   107     padding

pub const TICK_ARRAY_LEN: usize = 10240;

/// Empty tick-array account at the given start index. The on-chain swap
/// requires the FIRST tick-array passed to be the one containing
/// `pool.tick_current`, even if it has no initialized ticks. Captured
/// fixtures only include arrays the swap actually crossed, so we
/// synthesize empties for the leading array(s) when needed.
pub fn empty_tick_array_bytes(pool_id: &[u8; 32], start_tick_index: i32) -> Vec<u8> {
    let mut out = vec![0u8; TICK_ARRAY_LEN];
    out[0..8].copy_from_slice(&TICK_ARRAY_STATE_DISC);
    out[8..40].copy_from_slice(pool_id);
    out[40..44].copy_from_slice(&start_tick_index.to_le_bytes());
    // initialized_tick_count at offset 8 + 32 + 4 + 168*60 = 10124, leave 0.
    debug_assert_eq!(out.len(), TICK_ARRAY_LEN);
    out
}

// ---- SPL Token Mint ----
//
// 82 bytes:
//   36  COption<Pubkey> mint_authority
//   8   u64 supply
//   1   u8 decimals
//   1   bool is_initialized
//   36  COption<Pubkey> freeze_authority

pub const SPL_MINT_LEN: usize = 82;

pub fn spl_mint_bytes(mint_authority: Option<&[u8; 32]>, decimals: u8, supply: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(SPL_MINT_LEN);
    write_coption_pubkey(&mut out, mint_authority);
    out.extend_from_slice(&supply.to_le_bytes());
    out.push(decimals);
    out.push(1); // is_initialized
    write_coption_pubkey(&mut out, None); // freeze_authority
    debug_assert_eq!(out.len(), SPL_MINT_LEN);
    out
}

// ---- SPL Token Account ----
//
// 165 bytes:
//   32  Pubkey mint
//   32  Pubkey owner
//   8   u64 amount
//   36  COption<Pubkey> delegate
//   1   AccountState (1 = Initialized)
//   12  COption<u64> is_native
//   8   u64 delegated_amount
//   36  COption<Pubkey> close_authority

pub const SPL_TOKEN_ACCOUNT_LEN: usize = 165;

pub fn spl_token_account_bytes(mint: &[u8; 32], owner: &[u8; 32], amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(SPL_TOKEN_ACCOUNT_LEN);
    out.extend_from_slice(mint);
    out.extend_from_slice(owner);
    out.extend_from_slice(&amount.to_le_bytes());
    write_coption_pubkey(&mut out, None); // delegate
    out.push(1); // AccountState::Initialized
                 // is_native: COption<u64> = None (4 bytes 0 + 8 bytes pad)
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&0u64.to_le_bytes()); // delegated_amount
    write_coption_pubkey(&mut out, None); // close_authority
    debug_assert_eq!(out.len(), SPL_TOKEN_ACCOUNT_LEN);
    out
}

fn write_coption_pubkey(out: &mut Vec<u8>, pk: Option<&[u8; 32]>) {
    match pk {
        None => {
            out.extend_from_slice(&[0u8; 4]);
            out.extend_from_slice(&[0u8; 32]);
        }
        Some(p) => {
            out.extend_from_slice(&[1, 0, 0, 0]);
            out.extend_from_slice(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ammconfig_size_matches_upstream() {
        let bytes = ammconfig_bytes(255, 0, &[0u8; 32], 0, 100, 1, 0, &[0u8; 32]);
        assert_eq!(bytes.len(), AMM_CONFIG_LEN);
        assert_eq!(&bytes[..8], &AMM_CONFIG_DISC);
    }

    #[test]
    fn observation_state_size_matches_upstream() {
        let bytes = observation_state_bytes(&[0u8; 32]);
        assert_eq!(bytes.len(), OBSERVATION_LEN);
        assert_eq!(&bytes[..8], &OBSERVATION_STATE_DISC);
    }

    #[test]
    fn spl_mint_round_trip_offsets() {
        let auth = [7u8; 32];
        let bytes = spl_mint_bytes(Some(&auth), 9, 1_000_000_000);
        assert_eq!(bytes.len(), 82);
        // mint_authority is COption Some + 32 bytes = [1,0,0,0] + auth
        assert_eq!(&bytes[0..4], &[1, 0, 0, 0]);
        assert_eq!(&bytes[4..36], &auth);
        // decimals at offset 44
        assert_eq!(bytes[44], 9);
        // is_initialized at offset 45
        assert_eq!(bytes[45], 1);
    }

    #[test]
    fn spl_token_account_round_trip_offsets() {
        let mint = [3u8; 32];
        let owner = [5u8; 32];
        let bytes = spl_token_account_bytes(&mint, &owner, 1_000_000);
        assert_eq!(bytes.len(), 165);
        assert_eq!(&bytes[0..32], &mint);
        assert_eq!(&bytes[32..64], &owner);
        // amount at offset 64
        assert_eq!(
            u64::from_le_bytes(bytes[64..72].try_into().unwrap()),
            1_000_000
        );
        // AccountState at offset 108
        assert_eq!(bytes[108], 1); // Initialized
    }
}
