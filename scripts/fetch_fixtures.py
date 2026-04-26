"""Capture Raydium CLMM swap fixtures for solana-clmm-raydium replay tests.

Strategy: live monitoring. Snapshot pool + all of its tick-array accounts NOW
(via getProgramAccounts filtered on pool_id at offset 8). Then wait for the
*next* CLMM swap that touches the pool with NO intervening swaps. That snapshot
is the pre-state of that swap. Save fixture, advance, repeat.

Pre-state guarantee comes from:
  1. Reading head_sig = newest signature for the pool BEFORE snapshotting.
  2. After snapshot, polling getSignaturesForAddress(pool, until=head_sig) for
     new sigs. The sigs returned are *strictly after* head_sig, so the snapshot
     captures the state right before all of them.
  3. Taking only the chronologically-earliest of the new sigs and verifying its
     tx contains exactly one CLMM swap on our pool.

Output: tests/fixtures/swap_<sig_short>.json per swap. Each fixture is
self-contained (pool bytes + all tick-array bytes + observed swap result).

Run from repo root:
    .venv/bin/python solana-clmm-raydium/scripts/fetch_fixtures.py --count 30
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import struct
import sys
import time
from pathlib import Path

import requests
from dotenv import load_dotenv

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURES_DIR = REPO_ROOT / "tests" / "fixtures"
FIXTURES_DIR.mkdir(parents=True, exist_ok=True)
POOL_METADATA_PATH = FIXTURES_DIR / "clmm_pool_metadata.json"

CLMM_PROGRAM_ID = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"
# Empirical tick-array data size from raydium-clmm states/tick_array.rs.
# Verified on first run via getAccountInfo size.
TICK_ARRAY_DATA_SIZE = 10240
# Anchor instruction discriminators: first 8 bytes of sha256("global:<name>").
SWAP_DISCRIMINATOR = hashlib.sha256(b"global:swap").digest()[:8]
SWAP_V2_DISCRIMINATOR = hashlib.sha256(b"global:swap_v2").digest()[:8]

RPC_TIMEOUT_S = 30
POLL_INTERVAL_S = 2.0
MAX_WAIT_PER_FIXTURE_S = 180  # if no swap in 3 min, we'll skip and re-snapshot

# ---- Solana base58 (stdlib only) ----
B58_ALPHABET = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
B58_INDEX = {c: i for i, c in enumerate(B58_ALPHABET)}


def b58decode(s: str) -> bytes:
    n = 0
    for c in s.encode():
        n = n * 58 + B58_INDEX[c]
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    leading_ones = len(s) - len(s.lstrip("1"))
    return b"\x00" * leading_ones + body


# ---- RPC ----
load_dotenv()
HELIUS_API_KEY = os.environ.get("HELIUS_API_KEY")
if not HELIUS_API_KEY:
    print("HELIUS_API_KEY not set in env (.env or shell)", file=sys.stderr)
    sys.exit(2)
RPC_URL = f"https://mainnet.helius-rpc.com/?api-key={HELIUS_API_KEY}"


class RpcError(Exception):
    pass


def _rpc(method: str, params: list, *, timeout: float = RPC_TIMEOUT_S) -> dict | list | None:
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    try:
        r = requests.post(RPC_URL, json=body, timeout=timeout)
    except requests.RequestException as e:
        raise RpcError(f"network: {type(e).__name__}") from None
    if r.status_code == 429:
        raise RpcError("rate-limited (429)")
    if not r.ok:
        raise RpcError(f"http {r.status_code}")
    j = r.json()
    if "error" in j:
        raise RpcError(f"rpc-error: {j['error'].get('message', '?')[:160]}")
    return j.get("result")


# ---- snapshots ----
def snapshot_pool_and_arrays(pool_address: str) -> dict:
    """Returns {snapshot_slot, pool_b64, tick_arrays: [{address, data_b64}]}."""
    pool_resp = _rpc(
        "getAccountInfo",
        [pool_address, {"encoding": "base64", "commitment": "confirmed"}],
    )
    if not pool_resp or not pool_resp.get("value"):
        raise RpcError(f"pool account not found: {pool_address}")
    snapshot_slot = pool_resp["context"]["slot"]
    pool_b64 = pool_resp["value"]["data"][0]

    arrays_resp = _rpc(
        "getProgramAccounts",
        [
            CLMM_PROGRAM_ID,
            {
                "encoding": "base64",
                "commitment": "confirmed",
                "filters": [
                    {"dataSize": TICK_ARRAY_DATA_SIZE},
                    {"memcmp": {"offset": 8, "bytes": pool_address}},
                ],
            },
        ],
    )
    tick_arrays = [
        {"address": item["pubkey"], "data_b64": item["account"]["data"][0]}
        for item in (arrays_resp or [])
    ]
    return {
        "snapshot_slot": snapshot_slot,
        "pool_b64": pool_b64,
        "tick_arrays": tick_arrays,
    }


# ---- swap detection / parsing ----
def parse_swap_ix(ix_data_b58: str) -> dict | None:
    """Returns parsed swap args if `ix_data_b58` is an Anchor swap/swap_v2 ix,
    else None. Raydium swap args:
        amount: u64
        other_amount_threshold: u64
        sqrt_price_limit_x64: u128
        is_base_input: bool
    """
    try:
        raw = b58decode(ix_data_b58)
    except Exception:
        return None
    if len(raw) < 8 + 33:
        return None
    disc = raw[:8]
    if disc == SWAP_V2_DISCRIMINATOR:
        kind = "swap_v2"
    elif disc == SWAP_DISCRIMINATOR:
        kind = "swap"
    else:
        return None
    args = raw[8:]
    amount, threshold = struct.unpack("<QQ", args[:16])
    sqrt_price_limit_x64 = int.from_bytes(args[16:32], "little")
    is_base_input = bool(args[32])
    return {
        "kind": kind,
        "amount": amount,
        "other_amount_threshold": threshold,
        "sqrt_price_limit_x64": sqrt_price_limit_x64,
        "is_base_input": is_base_input,
    }


def find_clmm_swap_for_pool(tx: dict, pool_address: str) -> dict | None:
    """Walk all instructions (top-level + inner). Returns the first swap that
    targets pool_address, with parsed args + the ix's accounts list. None if no
    such swap (e.g. the sig touched the pool via a non-swap call)."""
    if not tx or not tx.get("transaction"):
        return None
    msg = tx["transaction"]["message"]
    accounts = [a["pubkey"] if isinstance(a, dict) else a for a in msg["accountKeys"]]
    if pool_address not in accounts:
        return None

    def walk(ixs: list) -> dict | None:
        for ix in ixs:
            program = ix.get("programId")
            if program != CLMM_PROGRAM_ID:
                continue
            data = ix.get("data")
            if not data:
                continue
            parsed = parse_swap_ix(data)
            if parsed is None:
                continue
            ix_accounts = ix.get("accounts", [])
            if pool_address not in ix_accounts:
                continue
            parsed["ix_accounts"] = ix_accounts
            return parsed
        return None

    found = walk(msg.get("instructions", []))
    if found:
        return found
    for inner in tx.get("meta", {}).get("innerInstructions", []) or []:
        found = walk(inner.get("instructions", []))
        if found:
            return found
    return None


def tx_touches_pool_via_clmm(tx: dict, pool_address: str) -> bool:
    """True if `tx` has ANY CLMM instruction whose accounts list contains
    `pool_address`. Used to detect intermediate state-mutating txs (open/close
    position, increase/decrease liquidity, etc.) that would invalidate our
    pre-swap snapshot. Conservative — flags any CLMM ix on the pool."""
    if not tx or not tx.get("transaction"):
        return False
    msg = tx["transaction"]["message"]
    accounts = [a["pubkey"] if isinstance(a, dict) else a for a in msg["accountKeys"]]
    if pool_address not in accounts:
        return False

    def any_clmm_ix(ixs: list) -> bool:
        for ix in ixs:
            if ix.get("programId") != CLMM_PROGRAM_ID:
                continue
            ix_accounts = ix.get("accounts", [])
            if pool_address in ix_accounts:
                return True
        return False

    if any_clmm_ix(msg.get("instructions", [])):
        return True
    for inner in tx.get("meta", {}).get("innerInstructions", []) or []:
        if any_clmm_ix(inner.get("instructions", [])):
            return True
    return False


def vault_deltas(tx: dict, vault_a: str, vault_b: str) -> tuple[int, int] | None:
    """Returns (delta_a, delta_b) as raw token amounts (post - pre).
    Sign convention: positive = into pool, negative = out of pool."""
    meta = tx.get("meta") or {}
    pre = {tb["accountIndex"]: tb for tb in meta.get("preTokenBalances", []) or []}
    post = {tb["accountIndex"]: tb for tb in meta.get("postTokenBalances", []) or []}
    accounts = [a["pubkey"] if isinstance(a, dict) else a
                for a in tx["transaction"]["message"]["accountKeys"]]

    def get_delta(vault: str) -> int | None:
        try:
            idx = accounts.index(vault)
        except ValueError:
            return None
        pre_amt = int(pre[idx]["uiTokenAmount"]["amount"]) if idx in pre else 0
        post_amt = int(post[idx]["uiTokenAmount"]["amount"]) if idx in post else 0
        return post_amt - pre_amt

    da = get_delta(vault_a)
    db = get_delta(vault_b)
    if da is None or db is None:
        return None
    return da, db


# ---- main loop ----
def gather_one_fixture(meta: dict, head_sig: str | None) -> tuple[dict, str] | None:
    """Snapshot, then wait for the next CLMM swap on this pool. Return
    (fixture, new_head_sig) or None if we timed out / had to re-snapshot."""
    pool = meta["pool_address"]
    vault_a = meta["vault_a"]
    vault_b = meta["vault_b"]

    if head_sig is None:
        # First call: read newest sig before snapshotting so we have a baseline.
        sigs = _rpc(
            "getSignaturesForAddress",
            [pool, {"limit": 1, "commitment": "confirmed"}],
        ) or []
        head_sig = sigs[0]["signature"] if sigs else None

    snap = snapshot_pool_and_arrays(pool)
    print(
        f"  snapshot @ slot {snap['snapshot_slot']} "
        f"({len(snap['tick_arrays'])} tick arrays, pool head sig: {(head_sig or '')[:16]}…)",
        flush=True,
    )

    deadline = time.monotonic() + MAX_WAIT_PER_FIXTURE_S
    while time.monotonic() < deadline:
        # Paginate full window of sigs back to head_sig (or snapshot_slot if no
        # head_sig). For high-volume pools the window can have hundreds of sigs;
        # missing any of them means a possible state-mutating tx slips past the
        # staleness check.
        new_sigs = []
        before = None
        while True:
            params = [pool, {"limit": 1000, "commitment": "confirmed"}]
            if head_sig:
                params[1]["until"] = head_sig
            if before:
                params[1]["before"] = before
            batch = _rpc("getSignaturesForAddress", params) or []
            if not batch:
                break
            new_sigs.extend(batch)
            if len(batch) < 1000:
                break
            before = batch[-1]["signature"]
        if not new_sigs:
            time.sleep(POLL_INTERVAL_S)
            continue
        # Sigs come newest-first. Walk oldest-to-newest; first CLMM swap found
        # is the earliest one after our snapshot. Reject it if ANY intermediate
        # tx (between head_sig and the chosen swap) modified pool state — our
        # snapshot would be stale otherwise.
        ordered = list(reversed(new_sigs))
        chose_swap = False
        for i, sig_meta in enumerate(ordered):
            if sig_meta.get("err") is not None:
                continue
            sig = sig_meta["signature"]
            tx = _rpc(
                "getTransaction",
                [
                    sig,
                    {
                        "encoding": "jsonParsed",
                        "commitment": "confirmed",
                        "maxSupportedTransactionVersion": 0,
                    },
                ],
            )
            swap = find_clmm_swap_for_pool(tx, pool) if tx else None
            if not swap:
                continue
            # Validate pre-swap snapshot freshness: any sig BEFORE this one in
            # ordered[] that touches the pool via CLMM invalidates our snapshot.
            stale = False
            for j in range(i):
                prev = ordered[j]
                if prev.get("err") is not None:
                    continue
                prev_tx = _rpc(
                    "getTransaction",
                    [
                        prev["signature"],
                        {
                            "encoding": "jsonParsed",
                            "commitment": "confirmed",
                            "maxSupportedTransactionVersion": 0,
                        },
                    ],
                )
                if tx_touches_pool_via_clmm(prev_tx, pool):
                    stale = True
                    break
            if stale:
                # Snapshot is stale — abort, caller will re-snapshot.
                return None
            # Snapshot slot must be strictly BEFORE the chosen swap. If our
            # getAccountInfo race read the pool at or after the swap's slot,
            # the snapshot reflects post-swap state.
            swap_slot = tx.get("slot")
            if swap_slot is None or swap_slot <= snap["snapshot_slot"]:
                return None
            deltas = vault_deltas(tx, vault_a, vault_b)
            if deltas is None:
                continue
            da, db = deltas
            if da == 0 and db == 0:
                continue
            chose_swap = True
            zero_for_one = da > 0  # token A in (positive delta_a) → 0->1
            observed_in = abs(da) if zero_for_one else abs(db)
            observed_out = abs(db) if zero_for_one else abs(da)
            # Filter snapshotted tick arrays to only those the swap's
            # instruction explicitly references. Cuts fixture size by ~99%.
            ix_acct_set = set(swap["ix_accounts"])
            touched_arrays = [
                ta for ta in snap["tick_arrays"] if ta["address"] in ix_acct_set
            ]
            fixture = {
                "pool_address": pool,
                "snapshot_slot": snap["snapshot_slot"],
                "pool_b64": snap["pool_b64"],
                "tick_arrays": touched_arrays,
                "swap": {
                    "signature": sig,
                    "slot": tx["slot"],
                    "block_time": tx.get("blockTime"),
                    "kind": swap["kind"],
                    "amount": swap["amount"],
                    "other_amount_threshold": swap["other_amount_threshold"],
                    "sqrt_price_limit_x64": str(swap["sqrt_price_limit_x64"]),
                    "is_base_input": swap["is_base_input"],
                    "zero_for_one": zero_for_one,
                    "observed_amount_in": observed_in,
                    "observed_amount_out": observed_out,
                    "vault_delta_a": da,
                    "vault_delta_b": db,
                },
            }
            return fixture, sig
        # All new sigs were non-swaps OR the snapshot was stale; advance head
        # and keep waiting. (If we returned None above for staleness, the
        # caller re-snapshots.)
        if chose_swap:
            return None
        head_sig = new_sigs[0]["signature"]
        time.sleep(POLL_INTERVAL_S)
    return None


def fixture_path(sig: str) -> Path:
    return FIXTURES_DIR / f"swap_{sig[:16]}.json"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=10, help="number of fixtures to gather")
    args = ap.parse_args()

    if not POOL_METADATA_PATH.exists():
        print(
            f"missing {POOL_METADATA_PATH} — run scripts/find_clmm_pool.py first",
            file=sys.stderr,
        )
        return 1
    meta = json.loads(POOL_METADATA_PATH.read_text())
    print(f"pool {meta['pool_address']} ({meta['mint_a']['symbol']}/{meta['mint_b']['symbol']})")

    head_sig = None
    n_collected = 0
    n_failed = 0
    while n_collected < args.count:
        print(f"[{n_collected + 1}/{args.count}] ", end="", flush=True)
        try:
            res = gather_one_fixture(meta, head_sig)
        except RpcError as e:
            print(f"  rpc error: {e} — retrying in 5s", flush=True)
            time.sleep(5)
            continue
        if res is None:
            print("  timeout, re-snapshotting and retrying", flush=True)
            n_failed += 1
            if n_failed >= 5:
                print("aborting after 5 consecutive timeouts", file=sys.stderr)
                return 1
            continue
        n_failed = 0
        fixture, head_sig = res
        path = fixture_path(fixture["swap"]["signature"])
        path.write_text(json.dumps(fixture, indent=2))
        n_collected += 1
        sw = fixture["swap"]
        side = f"{sw['observed_amount_in']:>12} -> {sw['observed_amount_out']:>12}"
        dir_ = "0→1" if sw["zero_for_one"] else "1→0"
        print(f"  ✓ {sw['signature'][:12]}…  {dir_}  {side}", flush=True)
    print(f"wrote {n_collected} fixtures to {FIXTURES_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
