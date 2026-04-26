"""Discover the canonical Raydium CLMM (concentrated-liquidity) SOL/USDC pool.

Analogous to ../../lvr/find_pool.py but with poolType=concentrated. Writes
metadata to tests/fixtures/clmm_pool_metadata.json so the fixture fetcher
and replay tests can pick it up.
"""
from __future__ import annotations
import json
import sys
from pathlib import Path

import requests

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURES_DIR = REPO_ROOT / "tests" / "fixtures"
FIXTURES_DIR.mkdir(parents=True, exist_ok=True)
POOL_METADATA_PATH = FIXTURES_DIR / "clmm_pool_metadata.json"

SOL_MINT = "So11111111111111111111111111111111111111112"
USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"

RAYDIUM_INFO_API = "https://api-v3.raydium.io/pools/info/mint"
RAYDIUM_KEY_API = "https://api-v3.raydium.io/pools/key/ids"

# Raydium CLMM program ID (mainnet).
EXPECTED_PROGRAM_ID = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"


def main() -> int:
    params = {
        "mint1": SOL_MINT,
        "mint2": USDC_MINT,
        "poolType": "concentrated",
        "poolSortField": "liquidity",
        "sortType": "desc",
        "pageSize": 5,
        "page": 1,
    }
    resp = requests.get(RAYDIUM_INFO_API, params=params, timeout=20)
    resp.raise_for_status()
    pools = resp.json().get("data", {}).get("data", [])
    if not pools:
        print("no concentrated SOL/USDC pools returned", file=sys.stderr)
        return 1
    p = pools[0]

    if p["programId"] != EXPECTED_PROGRAM_ID:
        print(
            f"warning: top pool program {p['programId']} != expected {EXPECTED_PROGRAM_ID}",
            file=sys.stderr,
        )

    keys_resp = requests.get(RAYDIUM_KEY_API, params={"ids": p["id"]}, timeout=20)
    keys_resp.raise_for_status()
    keys = (keys_resp.json().get("data") or [])
    if not keys:
        print(f"no key data for pool {p['id']}", file=sys.stderr)
        return 1
    k = keys[0]

    metadata = {
        "pool_address": p["id"],
        "program_id": p["programId"],
        "type": p["type"],
        "tick_spacing": k.get("config", {}).get("tickSpacing"),
        "fee_rate": p["feeRate"],
        "mint_a": {
            "address": p["mintA"]["address"],
            "symbol": p["mintA"]["symbol"],
            "decimals": p["mintA"]["decimals"],
        },
        "mint_b": {
            "address": p["mintB"]["address"],
            "symbol": p["mintB"]["symbol"],
            "decimals": p["mintB"]["decimals"],
        },
        "vault_a": k["vault"]["A"],
        "vault_b": k["vault"]["B"],
        "amm_config": k.get("config", {}).get("id"),
        "snapshot_price": p["price"],
        "snapshot_tvl_usd": p["tvl"],
    }
    POOL_METADATA_PATH.write_text(json.dumps(metadata, indent=2))
    print(f"pool:         {metadata['pool_address']}")
    print(f"program:      {metadata['program_id']}")
    print(f"pair:         {metadata['mint_a']['symbol']}/{metadata['mint_b']['symbol']}")
    print(f"tick_spacing: {metadata['tick_spacing']}")
    print(f"fee_rate:     {metadata['fee_rate']*100:.4f}%")
    print(f"tvl:          ${metadata['snapshot_tvl_usd']:,.0f}")
    print(f"price:        ${metadata['snapshot_price']:.4f}")
    print(f"vaultA:       {metadata['vault_a']}")
    print(f"vaultB:       {metadata['vault_b']}")
    print(f"amm_config:   {metadata['amm_config']}")
    print(f"wrote {POOL_METADATA_PATH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
