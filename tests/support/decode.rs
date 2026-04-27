//! Tests-only decoders for Raydium CLMM `PoolState` and `TickArrayState`.
//!
//! Mirrors layouts from
//! `raydium-io/raydium-clmm/programs/amm/src/states/{pool,tick_array}.rs`.
//! Both upstream structs are `#[repr(C, packed)]`, so we read field-by-field
//! at fixed byte offsets rather than fighting bytemuck's alignment rules.
//!
//! Only fields the replay test needs are extracted; the rest are skipped.

const ANCHOR_DISC: usize = 8;
const PUBKEY: usize = 32;
const REWARD_NUM: usize = 3;
const REWARD_INFO_LEN: usize = 1 + 8 + 8 + 8 + 16 + 8 + 8 + 32 + 32 + 32 + 16; // 169

// ---- PoolState offsets (upstream packed layout, includes Anchor 8-byte disc) ----
//
// 8                  discriminator
// + 1                bump
// + 32 * 7           amm_config, owner, mint_0, mint_1, vault_0, vault_1, observation_key
// + 1 + 1            mint_decimals_0, _1
// + 2                tick_spacing
// + 16               liquidity
// + 16               sqrt_price_x64
// + 4                tick_current
// + 2 + 2            padding3, padding4
// + 16 + 16          fee_growth_global_{0,1}_x64
// + 8 + 8            protocol_fees_token_{0,1}
// + 16 * 4           swap_in/out_amount_token_{0,1}
// + 1 + 7            status, [u8;7] padding
// + 169 * 3          reward_infos
// + 8 * 16           tick_array_bitmap

// Pubkey field offsets (used by litesvm test to synthesize matching accounts).
const OFF_AMM_CONFIG: usize = ANCHOR_DISC + 1; // 9
const OFF_MINT_0: usize = ANCHOR_DISC + 1 + PUBKEY * 2; // 73
const OFF_MINT_1: usize = OFF_MINT_0 + PUBKEY; // 105
const OFF_VAULT_0: usize = OFF_MINT_1 + PUBKEY; // 137
const OFF_VAULT_1: usize = OFF_VAULT_0 + PUBKEY; // 169
const OFF_OBSERVATION_KEY: usize = OFF_VAULT_1 + PUBKEY; // 201
const OFF_MINT_DECIMALS_0: usize = ANCHOR_DISC + 1 + PUBKEY * 7; // 233
const OFF_MINT_DECIMALS_1: usize = OFF_MINT_DECIMALS_0 + 1; // 234
const OFF_TICK_SPACING: usize = ANCHOR_DISC + 1 + PUBKEY * 7 + 2; // 235
const OFF_LIQUIDITY: usize = OFF_TICK_SPACING + 2; // 237
const OFF_SQRT_PRICE: usize = OFF_LIQUIDITY + 16; // 253
const OFF_TICK_CURRENT: usize = OFF_SQRT_PRICE + 16; // 269
const OFF_TICK_ARRAY_BITMAP: usize = OFF_TICK_CURRENT
    + 4   // tick_current
    + 4   // padding3 + padding4
    + 32  // fee_growth_global_{0,1}_x64
    + 16  // protocol_fees_{0,1}
    + 64  // 4 swap amounts (u128)
    + 8   // status + 7-byte padding
    + REWARD_INFO_LEN * REWARD_NUM; // 904

const POOL_MIN_LEN: usize = OFF_TICK_ARRAY_BITMAP + 8 * 16; // 1032

#[allow(dead_code)] // fields read by replay.rs; cargo dead-code analysis can't see across test binaries cleanly
#[derive(Debug, Clone)]
pub struct PoolState {
    // Pubkeys (used by litesvm_diff to synthesize matching support accounts).
    pub amm_config: [u8; 32],
    pub mint_0: [u8; 32],
    pub mint_1: [u8; 32],
    pub vault_0: [u8; 32],
    pub vault_1: [u8; 32],
    pub observation_key: [u8; 32],
    // Decimals (need matching SPL mint accounts).
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    // Swap-math fields.
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub tick_array_bitmap: [u64; 16],
}

impl PoolState {
    pub fn from_bytes(d: &[u8]) -> Result<Self, String> {
        if d.len() < POOL_MIN_LEN {
            return Err(format!(
                "PoolState bytes too short: {} < {}",
                d.len(),
                POOL_MIN_LEN
            ));
        }
        let mut bitmap = [0u64; 16];
        for (i, slot) in bitmap.iter_mut().enumerate() {
            let off = OFF_TICK_ARRAY_BITMAP + i * 8;
            *slot = u64::from_le_bytes(d[off..off + 8].try_into().unwrap());
        }
        let pk = |off: usize| -> [u8; 32] { d[off..off + PUBKEY].try_into().unwrap() };
        Ok(Self {
            amm_config: pk(OFF_AMM_CONFIG),
            mint_0: pk(OFF_MINT_0),
            mint_1: pk(OFF_MINT_1),
            vault_0: pk(OFF_VAULT_0),
            vault_1: pk(OFF_VAULT_1),
            observation_key: pk(OFF_OBSERVATION_KEY),
            mint_decimals_0: d[OFF_MINT_DECIMALS_0],
            mint_decimals_1: d[OFF_MINT_DECIMALS_1],
            tick_spacing: u16::from_le_bytes(
                d[OFF_TICK_SPACING..OFF_TICK_SPACING + 2]
                    .try_into()
                    .unwrap(),
            ),
            liquidity: u128::from_le_bytes(
                d[OFF_LIQUIDITY..OFF_LIQUIDITY + 16].try_into().unwrap(),
            ),
            sqrt_price_x64: u128::from_le_bytes(
                d[OFF_SQRT_PRICE..OFF_SQRT_PRICE + 16].try_into().unwrap(),
            ),
            tick_current: i32::from_le_bytes(
                d[OFF_TICK_CURRENT..OFF_TICK_CURRENT + 4]
                    .try_into()
                    .unwrap(),
            ),
            tick_array_bitmap: bitmap,
        })
    }
}

// ---- TickArrayState ----
//
// 8                            discriminator
// + 32                         pool_id
// + 4                          start_tick_index
// + TICK_LEN * 60              ticks
// + 1                          initialized_tick_count
// + 8                          recent_epoch
// + 107                        padding

const TICK_LEN: usize = 4              // tick: i32
    + 16           // liquidity_net: i128
    + 16           // liquidity_gross: u128
    + 16 + 16      // fee_growth_outside_{0,1}_x64
    + 16 * REWARD_NUM // reward_growths_outside_x64
    + 4 * 13; // padding [u32; 13]
              // = 168

pub const TICK_ARRAY_SIZE_USIZE: usize = 60;
#[allow(dead_code)] // used by decoders.rs; per-test-binary dead-code analysis flags otherwise
pub const TICK_ARRAY_DATA_LEN: usize =
    ANCHOR_DISC + PUBKEY + 4 + TICK_LEN * TICK_ARRAY_SIZE_USIZE + 1 + 8 + 107;
// = 8 + 32 + 4 + 10080 + 116 = 10240

const OFF_TARRAY_POOL_ID: usize = ANCHOR_DISC;
const OFF_TARRAY_START_TICK: usize = ANCHOR_DISC + PUBKEY; // 40
const OFF_TARRAY_TICKS: usize = ANCHOR_DISC + PUBKEY + 4; // 44

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct TickState {
    pub tick: i32,
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
}

impl TickState {
    pub fn from_bytes(d: &[u8]) -> Self {
        Self {
            tick: i32::from_le_bytes(d[0..4].try_into().unwrap()),
            liquidity_net: i128::from_le_bytes(d[4..20].try_into().unwrap()),
            liquidity_gross: u128::from_le_bytes(d[20..36].try_into().unwrap()),
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.liquidity_gross != 0
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TickArrayState {
    pub pool_id: [u8; 32],
    pub start_tick_index: i32,
    pub ticks: [TickState; TICK_ARRAY_SIZE_USIZE],
}

impl TickArrayState {
    pub fn from_bytes(d: &[u8]) -> Result<Self, String> {
        if d.len() < OFF_TARRAY_TICKS + TICK_LEN * TICK_ARRAY_SIZE_USIZE {
            return Err(format!(
                "TickArrayState bytes too short: {} < {}",
                d.len(),
                OFF_TARRAY_TICKS + TICK_LEN * TICK_ARRAY_SIZE_USIZE
            ));
        }
        let mut pool_id = [0u8; 32];
        pool_id.copy_from_slice(&d[OFF_TARRAY_POOL_ID..OFF_TARRAY_POOL_ID + PUBKEY]);
        let start_tick_index = i32::from_le_bytes(
            d[OFF_TARRAY_START_TICK..OFF_TARRAY_START_TICK + 4]
                .try_into()
                .unwrap(),
        );
        let mut ticks = [TickState {
            tick: 0,
            liquidity_net: 0,
            liquidity_gross: 0,
        }; TICK_ARRAY_SIZE_USIZE];
        for (i, slot) in ticks.iter_mut().enumerate() {
            let off = OFF_TARRAY_TICKS + i * TICK_LEN;
            *slot = TickState::from_bytes(&d[off..off + TICK_LEN]);
        }
        Ok(Self {
            pool_id,
            start_tick_index,
            ticks,
        })
    }
}
