---
name: cluster-6
description: "Skill for the Cluster_6 area of sigillum. 33 symbols across 2 files."
---

# Cluster_6

33 symbols | 2 files | Cohesion: 81%

## When to Use

- Working with code in `crates/`
- Understanding how new, vault_exists, verify_master_key work
- Modifying cluster_6-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-core/src/file_vault.rs` | new, vault_exists, verify_master_key, with_master_key, tier2_path (+26) |
| `crates/sigillum-core/src/traits.rs` | get_secret, list_secrets |

## Entry Points

Start here when exploring this area:

- **`new`** (Function) — `crates/sigillum-core/src/file_vault.rs:84`
- **`vault_exists`** (Function) — `crates/sigillum-core/src/file_vault.rs:98`
- **`verify_master_key`** (Function) — `crates/sigillum-core/src/file_vault.rs:104`
- **`with_master_key`** (Function) — `crates/sigillum-core/src/file_vault.rs:112`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `new` | Function | `crates/sigillum-core/src/file_vault.rs` | 84 |
| `vault_exists` | Function | `crates/sigillum-core/src/file_vault.rs` | 98 |
| `verify_master_key` | Function | `crates/sigillum-core/src/file_vault.rs` | 104 |
| `with_master_key` | Function | `crates/sigillum-core/src/file_vault.rs` | 112 |
| `get_secret` | Function | `crates/sigillum-core/src/traits.rs` | 55 |
| `list_secrets` | Function | `crates/sigillum-core/src/traits.rs` | 60 |
| `tier2_path` | Function | `crates/sigillum-core/src/file_vault.rs` | 126 |
| `load_store` | Function | `crates/sigillum-core/src/file_vault.rs` | 164 |
| `save_store` | Function | `crates/sigillum-core/src/file_vault.rs` | 187 |
| `read_secret` | Function | `crates/sigillum-core/src/file_vault.rs` | 248 |
| `set_secret` | Function | `crates/sigillum-core/src/file_vault.rs` | 258 |
| `delete_secret` | Function | `crates/sigillum-core/src/file_vault.rs` | 268 |
| `read_secrets` | Function | `crates/sigillum-core/src/file_vault.rs` | 278 |
| `load_master_key` | Function | `crates/sigillum-core/src/file_vault.rs` | 308 |
| `zeroize_master_key` | Function | `crates/sigillum-core/src/file_vault.rs` | 313 |
| `initialize` | Function | `crates/sigillum-core/src/file_vault.rs` | 318 |
| `test_vault` | Function | `crates/sigillum-core/src/file_vault.rs` | 334 |
| `test_key` | Function | `crates/sigillum-core/src/file_vault.rs` | 344 |
| `tier1_get_missing_returns_none` | Function | `crates/sigillum-core/src/file_vault.rs` | 361 |
| `tier1_malformed_store_blocks_write_and_preserves_data` | Function | `crates/sigillum-core/src/file_vault.rs` | 393 |

## Execution Flows

| Flow | Type | Steps |
|------|------|-------|
| `Get_secret → Tier2_path` | cross_community | 5 |
| `Fido2_remove → With_master_key` | cross_community | 5 |
| `Fido2_remove → New` | cross_community | 5 |
| `Create_eth_stealth_native_deposit → With_master_key` | cross_community | 5 |
| `Create_eth_stealth_native_deposit → New` | cross_community | 5 |
| `Eth_stealth_export → With_master_key` | cross_community | 4 |
| `Eth_stealth_export → New` | cross_community | 4 |
| `Eth_stealth_check → With_master_key` | cross_community | 4 |
| `Eth_stealth_check → New` | cross_community | 4 |
| `Eth_stealth_sign → With_master_key` | cross_community | 4 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Service | 1 calls |
| Cluster_0 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "new"})` — see callers and callees
2. `gitnexus_query({query: "cluster_6"})` — find related execution flows
3. Read key files listed above for implementation details
