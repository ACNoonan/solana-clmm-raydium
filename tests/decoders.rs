//! Sanity tests for the test-only PoolState / TickArrayState decoders.
//! Loads any fixture in tests/fixtures/ and asserts the decoded fields are
//! plausible — confirms our byte-offset layout matches upstream.

mod support;

use base64::Engine;
use serde_json::Value;
use std::path::PathBuf;
use support::decode::{PoolState, TickArrayState, TICK_ARRAY_DATA_LEN};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture() -> Option<Value> {
    let dir = fixtures_dir();
    for entry in std::fs::read_dir(&dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("swap_") && name.ends_with(".json") {
            let bytes = std::fs::read(entry.path()).ok()?;
            return serde_json::from_slice(&bytes).ok();
        }
    }
    None
}

fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .expect("valid base64")
}

#[test]
fn pool_state_decodes_from_fixture() {
    let Some(fx) = load_fixture() else {
        // No fixtures yet — skip rather than fail. CI/dev workflow: run
        // scripts/fetch_fixtures.py to generate fixtures, then re-run.
        eprintln!("[skip] no fixtures in tests/fixtures/");
        return;
    };
    let pool_b64 = fx["pool_b64"].as_str().expect("pool_b64");
    let bytes = b64(pool_b64);
    let pool = PoolState::from_bytes(&bytes).expect("decode pool");

    // Sanity: SOL/USDC CLMM at ~$86 → tick_current near ln(86)/ln(1.0001)
    // = ln(86) / ln(1.0001) ≈ 4.454/0.0001 ≈ 44_543. Allow wide bounds.
    assert!(
        (-200_000..=200_000).contains(&pool.tick_current),
        "implausible tick_current = {}",
        pool.tick_current
    );
    // Tick spacing for CLMM SOL/USDC 4bps tier we picked: 1.
    assert_eq!(pool.tick_spacing, 1, "tick_spacing should be 1");
    // Liquidity must be > 0 for an active pool.
    assert!(pool.liquidity > 0, "liquidity should be > 0");
    // sqrt_price must be inside the canonical CLMM domain
    use solana_clmm_raydium::{MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};
    assert!(pool.sqrt_price_x64 >= MIN_SQRT_PRICE_X64);
    assert!(pool.sqrt_price_x64 < MAX_SQRT_PRICE_X64);

    // sqrt_price <-> tick_current must match what the on-chain math says.
    let sp_from_tick = solana_clmm_raydium::get_sqrt_price_at_tick(pool.tick_current).unwrap();
    let tick_from_sp = solana_clmm_raydium::get_tick_at_sqrt_price(pool.sqrt_price_x64).unwrap();
    // sqrt_price moves continuously between ticks; tick_from_sp == tick_current
    // when no swap has updated tick_current to a non-canonical position. Should
    // hold here since we read straight from the on-chain account.
    assert_eq!(
        tick_from_sp, pool.tick_current,
        "tick_current ({}) does not match what get_tick_at_sqrt_price computes ({}) from sqrt_price={}",
        pool.tick_current, tick_from_sp, pool.sqrt_price_x64
    );
    // sp_from_tick is the sqrt price at the lower bound of the current tick;
    // pool.sqrt_price_x64 is between sp_from_tick and sp_from_tick(tick+1).
    let sp_next = solana_clmm_raydium::get_sqrt_price_at_tick(pool.tick_current + 1).unwrap();
    assert!(
        pool.sqrt_price_x64 >= sp_from_tick && pool.sqrt_price_x64 < sp_next,
        "sqrt_price {} not in [{}, {}) for tick {}",
        pool.sqrt_price_x64,
        sp_from_tick,
        sp_next,
        pool.tick_current
    );

    eprintln!(
        "pool_state: tick_current={} sqrt_price_x64={} liquidity={} tick_spacing={}",
        pool.tick_current, pool.sqrt_price_x64, pool.liquidity, pool.tick_spacing
    );
}

#[test]
fn tick_array_decodes_from_fixture() {
    let Some(fx) = load_fixture() else {
        eprintln!("[skip] no fixtures in tests/fixtures/");
        return;
    };
    let pool_addr_str = fx["pool_address"].as_str().unwrap();
    // Decode pool address (base58) for pool_id verification — quick & dirty
    // base58 alphabet decode using a lookup table.
    let pool_id_bytes = base58_decode(pool_addr_str);
    assert_eq!(pool_id_bytes.len(), 32, "pool address must be 32 bytes");

    let arrays = fx["tick_arrays"].as_array().expect("tick_arrays");
    assert!(!arrays.is_empty(), "fixture has no tick arrays");

    for ta_json in arrays {
        let data_b64 = ta_json["data_b64"].as_str().unwrap();
        let bytes = b64(data_b64);
        assert_eq!(
            bytes.len(),
            TICK_ARRAY_DATA_LEN,
            "tick-array account size {} != expected {}",
            bytes.len(),
            TICK_ARRAY_DATA_LEN
        );
        let ta = TickArrayState::from_bytes(&bytes).expect("decode tick array");
        assert_eq!(
            ta.pool_id,
            pool_id_bytes.as_slice(),
            "tick-array pool_id mismatch"
        );

        // start_tick_index must be a multiple of tick_count_in_array(spacing=1) = 60.
        assert_eq!(
            ta.start_tick_index % 60,
            0,
            "start_tick_index {} not aligned to 60",
            ta.start_tick_index
        );

        // Ticks within the array must be in [start, start+60) and ascending where
        // initialized. Initialized ticks have liquidity_gross > 0.
        let mut prev: Option<i32> = None;
        for t in ta.ticks.iter().filter(|t| t.is_initialized()) {
            assert!(
                t.tick >= ta.start_tick_index && t.tick < ta.start_tick_index + 60,
                "init tick {} out of array range [{}, {})",
                t.tick,
                ta.start_tick_index,
                ta.start_tick_index + 60
            );
            if let Some(p) = prev {
                assert!(t.tick > p, "ticks not ascending: {} after {}", t.tick, p);
            }
            prev = Some(t.tick);
        }
    }
    eprintln!("decoded {} tick arrays cleanly", arrays.len());
}

// ---- minimal base58 for pool address verification ----
fn base58_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut idx = [255u8; 128];
    for (i, &c) in ALPHABET.iter().enumerate() {
        idx[c as usize] = i as u8;
    }
    let mut n: num_bigint_helper::BigInt = num_bigint_helper::BigInt::ZERO;
    for &c in s.as_bytes() {
        let v = idx[c as usize];
        assert!(v != 255, "invalid base58 char");
        n = n.mul58().add(v);
    }
    let mut bytes = n.into_be_bytes();
    let leading_ones = s.bytes().take_while(|&c| c == b'1').count();
    let mut out = vec![0u8; leading_ones];
    out.append(&mut bytes);
    out
}

// Tiny base58 helper inline to avoid pulling num-bigint as a dev-dep.
mod num_bigint_helper {
    /// Just enough big-int (bytes, base 256, big-endian) for base58 decode.
    #[derive(Default, Clone)]
    pub struct BigInt(Vec<u8>);
    impl BigInt {
        pub const ZERO: Self = Self(Vec::new());
        pub fn mul58(mut self) -> Self {
            let mut carry: u32 = 0;
            for b in self.0.iter_mut().rev() {
                let v = (*b as u32) * 58 + carry;
                *b = (v & 0xff) as u8;
                carry = v >> 8;
            }
            while carry > 0 {
                self.0.insert(0, (carry & 0xff) as u8);
                carry >>= 8;
            }
            self
        }
        pub fn add(mut self, x: u8) -> Self {
            let mut carry: u32 = x as u32;
            for b in self.0.iter_mut().rev() {
                let v = *b as u32 + carry;
                *b = (v & 0xff) as u8;
                carry = v >> 8;
                if carry == 0 {
                    break;
                }
            }
            while carry > 0 {
                self.0.insert(0, (carry & 0xff) as u8);
                carry >>= 8;
            }
            self
        }
        pub fn into_be_bytes(self) -> Vec<u8> {
            self.0
        }
    }
}
