//! Raydium CLMM on-chain constants used by the litesvm differential test.
//!
//! Derived from Anchor naming conventions:
//!
//! - **Account discriminators:** `sha256("account:<TypeName>")[..8]`
//! - **Instruction discriminators:** `sha256("global:<ix_name>")[..8]`
//!
//! These match `raydium-io/raydium-clmm` Anchor codegen as of the program
//! ELF captured at `tests/fixtures/raydium_clmm.so`. Verified by
//! `python3 -c "import hashlib; print(hashlib.sha256(b'account:PoolState').digest()[:8].hex())"`.

#![allow(dead_code)] // referenced by tests that may not run on every binary

use solana_pubkey::Pubkey;

/// Mainnet Raydium CLMM program id
/// (`CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`).
pub const PROGRAM_ID: Pubkey = solana_pubkey::pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");

// ---- Anchor account discriminators ----

pub const POOL_STATE_DISC: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const AMM_CONFIG_DISC: [u8; 8] = [218, 244, 33, 104, 203, 203, 43, 111];
pub const TICK_ARRAY_STATE_DISC: [u8; 8] = [192, 155, 85, 205, 49, 249, 129, 42];
pub const OBSERVATION_STATE_DISC: [u8; 8] = [122, 174, 197, 53, 129, 9, 165, 132];

// ---- Anchor instruction discriminators ----

pub const IX_SWAP_DISC: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
pub const IX_SWAP_V2_DISC: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
