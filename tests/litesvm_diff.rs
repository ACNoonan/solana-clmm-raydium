//! Litesvm differential test.
//!
//! Loads the on-chain Raydium CLMM program ELF into an in-process VM,
//! injects synthesized supporting accounts (mints, vaults, AmmConfig,
//! ObservationState) plus the captured pool / tick-array bytes from a
//! mainnet fixture, and runs the same `swap_v2` instruction the captured
//! transaction did. Compares the user's vault deltas against (a) our
//! `compute_swap_full` output and (b) the captured `observed_amount_*`
//! to assert all three implementations agree.
//!
//! See the audit (`docs/audits/v0.1.0-external-review.md` §4.8) and
//! `MeteoraAg/dlmm-sdk` `commons/tests/integration/test_swap.rs` for the
//! pattern.

mod support;

use std::path::PathBuf;

use base64::Engine;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::{pubkey, Pubkey};
use solana_signer::Signer;
use solana_transaction::Transaction;

use solana_clmm_raydium::{compute_swap_full, InitializedTick, SwapPool};

use support::decode::{PoolState, TickArrayState};
use support::encode::{
    ammconfig_bytes, empty_tick_array_bytes, observation_state_bytes, spl_mint_bytes,
    spl_token_account_bytes, AMM_CONFIG_LEN, OBSERVATION_LEN, SPL_MINT_LEN, SPL_TOKEN_ACCOUNT_LEN,
    TICK_ARRAY_LEN,
};
use support::raydium::{IX_SWAP_V2_DISC, PROGRAM_ID};

// ---- Solana program IDs we wire up ----

const SPL_TOKEN_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SPL_TOKEN_2022_ID: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const MEMO_ID: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

fn program_elf_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raydium_clmm.so")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .expect("valid base64")
}

fn b58(s: &str) -> Pubkey {
    Pubkey::try_from(s).expect("valid base58 pubkey")
}

// ---- Smoke test: ELF loads ----

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
        "ELF should be ~1.7MB; got {} bytes",
        acct.data.len()
    );
}

// ---- Differential test ----

/// All `swap_v2` fixtures, sorted by `amount` ascending.
fn all_swap_v2_fixtures() -> Vec<(String, serde_json::Value)> {
    let dir = fixtures_dir();
    let mut candidates: Vec<(String, serde_json::Value)> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("swap_") && n.ends_with(".json")
        })
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let fx: serde_json::Value =
                serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap();
            (name, fx)
        })
        .filter(|(_, fx)| fx["swap"]["kind"] == "swap_v2")
        .collect();
    candidates.sort_by_key(|(_, fx)| fx["swap"]["amount"].as_u64().unwrap_or(u64::MAX));
    candidates
}

fn rent_exempt(svm: &LiteSVM, data_len: usize) -> u64 {
    svm.minimum_balance_for_rent_exemption(data_len)
}

fn make_account(data: Vec<u8>, owner: Pubkey, lamports: u64) -> Account {
    Account {
        lamports,
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    }
}

#[test]
fn differential_swap_v2_byte_exact_all_fixtures() {
    let fixtures = all_swap_v2_fixtures();
    assert!(!fixtures.is_empty(), "no swap_v2 fixtures found");
    eprintln!(
        "running litesvm differential on {} swap_v2 fixtures",
        fixtures.len()
    );
    for (name, fx) in &fixtures {
        eprintln!(
            "\n=== fixture: {name} (amount={}) ===",
            fx["swap"]["amount"]
        );
        run_one_fixture(fx);
    }
    eprintln!(
        "\n✓ {} swap_v2 fixtures replayed byte-exact through litesvm",
        fixtures.len()
    );
}

fn run_one_fixture(fx: &serde_json::Value) {
    let pool_bytes = b64(fx["pool_b64"].as_str().unwrap());
    let pool = PoolState::from_bytes(&pool_bytes).expect("decode pool");

    let pool_pubkey = b58(fx["pool_address"].as_str().unwrap());
    let amm_config_pk = Pubkey::new_from_array(pool.amm_config);
    let mint_0_pk = Pubkey::new_from_array(pool.mint_0);
    let mint_1_pk = Pubkey::new_from_array(pool.mint_1);
    let vault_0_pk = Pubkey::new_from_array(pool.vault_0);
    let vault_1_pk = Pubkey::new_from_array(pool.vault_1);
    let observation_pk = Pubkey::new_from_array(pool.observation_key);

    let swap = &fx["swap"];
    let amount: u64 = swap["amount"].as_u64().unwrap();
    let other_threshold: u64 = swap["other_amount_threshold"].as_u64().unwrap();
    let sqrt_limit: u128 = swap["sqrt_price_limit_x64"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let is_base_input = swap["is_base_input"].as_bool().unwrap();
    let zero_for_one = swap["zero_for_one"].as_bool().unwrap();
    let observed_in: u64 = swap["observed_amount_in"].as_u64().unwrap();
    let observed_out: u64 = swap["observed_amount_out"].as_u64().unwrap();

    // Pool metadata (fee_rate in pips).
    let meta_path = fixtures_dir().join("clmm_pool_metadata.json");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta_path).unwrap()).unwrap();
    let fee_rate_pips: u32 = (meta["fee_rate"].as_f64().unwrap() * 1_000_000.0).round() as u32;

    // ---- compute_swap_full as the reference oracle ----
    let arrays: Vec<TickArrayState> = fx["tick_arrays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ta| TickArrayState::from_bytes(&b64(ta["data_b64"].as_str().unwrap())).unwrap())
        .collect();
    let ticks: Vec<InitializedTick> = arrays
        .iter()
        .flat_map(|ta| {
            ta.ticks.iter().filter_map(|t| {
                t.is_initialized().then_some(InitializedTick {
                    tick: t.tick,
                    liquidity_net: t.liquidity_net,
                })
            })
        })
        .collect();
    let mut ticks = ticks;
    ticks.sort_by_key(|t| t.tick);

    let swap_pool = SwapPool {
        sqrt_price_x64: pool.sqrt_price_x64,
        liquidity: pool.liquidity,
        tick_current: pool.tick_current,
        tick_spacing: pool.tick_spacing,
        fee_rate_pips,
    };
    let our = compute_swap_full(
        &swap_pool,
        &ticks,
        amount,
        sqrt_limit,
        is_base_input,
        zero_for_one,
    )
    .expect("our compute_swap_full");

    eprintln!(
        "compute_swap_full: in={} out={}",
        our.amount_in, our.amount_out
    );
    eprintln!(
        "captured observed:  in={} out={}",
        observed_in, observed_out
    );
    assert_eq!(
        our.amount_in, observed_in,
        "extracted math vs mainnet replay"
    );
    assert_eq!(
        our.amount_out, observed_out,
        "extracted math vs mainnet replay"
    );

    // ---- LiteSVM setup ----
    let mut svm = LiteSVM::new();
    svm.add_program_from_file(PROGRAM_ID, program_elf_path())
        .expect("ELF loads");

    // Warp the clock to the captured swap's block_time so the program's
    // `block_timestamp > pool.open_time` guard passes. (LiteSVM defaults
    // to slot 0 / unix_timestamp 0.)
    let captured_block_time = swap["block_time"].as_u64().unwrap() as i64;
    let mut clock = svm.get_sysvar::<solana_clock::Clock>();
    clock.unix_timestamp = captured_block_time;
    svm.set_sysvar::<solana_clock::Clock>(&clock);

    // Pool + captured tick array bytes verbatim.
    let tick_array_pk = b58(fx["tick_arrays"][0]["address"].as_str().unwrap());
    let tick_array_bytes = b64(fx["tick_arrays"][0]["data_b64"].as_str().unwrap());
    svm.set_account(
        pool_pubkey,
        make_account(
            pool_bytes.clone(),
            PROGRAM_ID,
            rent_exempt(&svm, pool_bytes.len()),
        ),
    )
    .unwrap();
    svm.set_account(
        tick_array_pk,
        make_account(
            tick_array_bytes.clone(),
            PROGRAM_ID,
            rent_exempt(&svm, tick_array_bytes.len()),
        ),
    )
    .unwrap();

    // The on-chain swap expects the FIRST tick array passed to be the one
    // containing `pool.tick_current`. Captured fixtures only include arrays
    // the swap physically crossed; synthesize an empty array at the
    // current-tick array start with the pool_id pinned.
    let array_width = (pool.tick_spacing as i32) * 60; // 60 ticks per array
    let current_array_start = {
        let t = pool.tick_current;
        if t < 0 && t % array_width != 0 {
            (t / array_width - 1) * array_width
        } else {
            (t / array_width) * array_width
        }
    };
    eprintln!(
        "tick_current={} → current array start={} (captured array start={})",
        pool.tick_current,
        current_array_start,
        i32::from_le_bytes(tick_array_bytes[40..44].try_into().unwrap())
    );

    let empty_first_array_pk = if current_array_start
        != i32::from_le_bytes(tick_array_bytes[40..44].try_into().unwrap())
    {
        let pk = Pubkey::new_unique();
        let data = empty_tick_array_bytes(&pool_pubkey.to_bytes(), current_array_start);
        svm.set_account(
            pk,
            make_account(data, PROGRAM_ID, rent_exempt(&svm, TICK_ARRAY_LEN)),
        )
        .unwrap();
        Some(pk)
    } else {
        None
    };

    // AmmConfig with metadata's trade_fee_rate; protocol/fund=0 (don't affect vault deltas).
    let amm_config_data = ammconfig_bytes(
        0,
        0,
        &[0; 32],
        0,
        fee_rate_pips,
        pool.tick_spacing,
        0,
        &[0; 32],
    );
    svm.set_account(
        amm_config_pk,
        make_account(
            amm_config_data,
            PROGRAM_ID,
            rent_exempt(&svm, AMM_CONFIG_LEN),
        ),
    )
    .unwrap();

    // ObservationState — zeroed buffer with discriminator + pool_id.
    let obs_data = observation_state_bytes(&pool_pubkey.to_bytes());
    svm.set_account(
        observation_pk,
        make_account(obs_data, PROGRAM_ID, rent_exempt(&svm, OBSERVATION_LEN)),
    )
    .unwrap();

    // SPL mints — decimals from PoolState.
    let mint_0_data = spl_mint_bytes(None, pool.mint_decimals_0, u64::MAX / 4);
    let mint_1_data = spl_mint_bytes(None, pool.mint_decimals_1, u64::MAX / 4);
    svm.set_account(
        mint_0_pk,
        make_account(mint_0_data, SPL_TOKEN_ID, rent_exempt(&svm, SPL_MINT_LEN)),
    )
    .unwrap();
    svm.set_account(
        mint_1_pk,
        make_account(mint_1_data, SPL_TOKEN_ID, rent_exempt(&svm, SPL_MINT_LEN)),
    )
    .unwrap();

    // SPL vaults — owner = pool_state, large balance.
    let vault_balance: u64 = u64::MAX / 4;
    let vault_0_data =
        spl_token_account_bytes(&pool.mint_0, &pool_pubkey.to_bytes(), vault_balance);
    let vault_1_data =
        spl_token_account_bytes(&pool.mint_1, &pool_pubkey.to_bytes(), vault_balance);
    svm.set_account(
        vault_0_pk,
        make_account(
            vault_0_data,
            SPL_TOKEN_ID,
            rent_exempt(&svm, SPL_TOKEN_ACCOUNT_LEN),
        ),
    )
    .unwrap();
    svm.set_account(
        vault_1_pk,
        make_account(
            vault_1_data,
            SPL_TOKEN_ID,
            rent_exempt(&svm, SPL_TOKEN_ACCOUNT_LEN),
        ),
    )
    .unwrap();

    // User wallet + token accounts.
    let user = Keypair::new();
    svm.airdrop(&user.pubkey(), 1_000_000_000).unwrap();
    let user_input_ata = Pubkey::new_unique();
    let user_output_ata = Pubkey::new_unique();
    let user_input_balance: u64 = amount.saturating_add(amount); // headroom

    // input/output mint depends on zero_for_one direction.
    let (input_mint, output_mint, input_vault, output_vault) = if zero_for_one {
        (mint_0_pk, mint_1_pk, vault_0_pk, vault_1_pk)
    } else {
        (mint_1_pk, mint_0_pk, vault_1_pk, vault_0_pk)
    };
    let input_mint_arr: [u8; 32] = input_mint.to_bytes();
    let output_mint_arr: [u8; 32] = output_mint.to_bytes();
    let user_arr: [u8; 32] = user.pubkey().to_bytes();
    svm.set_account(
        user_input_ata,
        make_account(
            spl_token_account_bytes(&input_mint_arr, &user_arr, user_input_balance),
            SPL_TOKEN_ID,
            rent_exempt(&svm, SPL_TOKEN_ACCOUNT_LEN),
        ),
    )
    .unwrap();
    svm.set_account(
        user_output_ata,
        make_account(
            spl_token_account_bytes(&output_mint_arr, &user_arr, 0),
            SPL_TOKEN_ID,
            rent_exempt(&svm, SPL_TOKEN_ACCOUNT_LEN),
        ),
    )
    .unwrap();

    // ---- Build swap_v2 instruction ----
    let mut data = Vec::with_capacity(8 + 8 + 8 + 16 + 1);
    data.extend_from_slice(&IX_SWAP_V2_DISC);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_limit.to_le_bytes());
    data.push(is_base_input as u8);

    let mut accounts = vec![
        AccountMeta::new_readonly(user.pubkey(), true), // payer (signer)
        AccountMeta::new_readonly(amm_config_pk, false),
        AccountMeta::new(pool_pubkey, false),
        AccountMeta::new(user_input_ata, false),
        AccountMeta::new(user_output_ata, false),
        AccountMeta::new(input_vault, false),
        AccountMeta::new(output_vault, false),
        AccountMeta::new(observation_pk, false),
        AccountMeta::new_readonly(SPL_TOKEN_ID, false),
        AccountMeta::new_readonly(SPL_TOKEN_2022_ID, false),
        AccountMeta::new_readonly(MEMO_ID, false),
        AccountMeta::new_readonly(input_mint, false),
        AccountMeta::new_readonly(output_mint, false),
    ];
    // remaining_accounts: tick array(s) — first must be the array containing
    // pool.tick_current, then any subsequent arrays the swap can cross.
    if let Some(pk) = empty_first_array_pk {
        accounts.push(AccountMeta::new(pk, false));
    }
    accounts.push(AccountMeta::new(tick_array_pk, false));

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data,
    };
    let tx = Transaction::new(
        &[&user],
        Message::new(&[ix], Some(&user.pubkey())),
        svm.latest_blockhash(),
    );
    let result = svm.send_transaction(tx);
    match result {
        Ok(meta) => {
            eprintln!("on-chain swap succeeded; logs:");
            for l in &meta.logs {
                eprintln!("  {l}");
            }
        }
        Err(e) => {
            eprintln!("on-chain swap failed: {e:?}");
            panic!("swap_v2 rejected by program — see logs above");
        }
    }

    // ---- Compare deltas ----
    let read_balance = |pk: Pubkey| -> u64 {
        let a = svm.get_account(&pk).unwrap();
        u64::from_le_bytes(a.data[64..72].try_into().unwrap())
    };
    let user_in_after = read_balance(user_input_ata);
    let user_out_after = read_balance(user_output_ata);
    let actual_in = user_input_balance - user_in_after;
    let actual_out = user_out_after; // started at 0
    eprintln!("on-chain deltas: in={} out={}", actual_in, actual_out,);
    assert_eq!(actual_in, observed_in, "litesvm vs mainnet replay (in)");
    assert_eq!(actual_out, observed_out, "litesvm vs mainnet replay (out)");
}
