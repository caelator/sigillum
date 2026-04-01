---
name: cluster-0
description: "Skill for the Cluster_0 area of sigillum. 9 symbols across 2 files."
---

# Cluster_0

9 symbols | 2 files | Cohesion: 65%

## When to Use

- Working with code in `crates/`
- Understanding how atomic_write, backup_path work
- Modifying cluster_0-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/src/json_store.rs` | load_with_parser, save_json_bytes, backup_path, read_backup, sync_backup (+1) |
| `crates/sigillum-core/src/utils.rs` | atomic_write, atomic_write_roundtrip, atomic_write_sets_permissions |

## Entry Points

Start here when exploring this area:

- **`atomic_write`** (Function) — `crates/sigillum-core/src/utils.rs:33`
- **`backup_path`** (Function) — `crates/sigillum-daemon/src/json_store.rs:219`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `atomic_write` | Function | `crates/sigillum-core/src/utils.rs` | 33 |
| `backup_path` | Function | `crates/sigillum-daemon/src/json_store.rs` | 219 |
| `atomic_write_roundtrip` | Function | `crates/sigillum-core/src/utils.rs` | 183 |
| `atomic_write_sets_permissions` | Function | `crates/sigillum-core/src/utils.rs` | 192 |
| `load_with_parser` | Function | `crates/sigillum-daemon/src/json_store.rs` | 170 |
| `save_json_bytes` | Function | `crates/sigillum-daemon/src/json_store.rs` | 210 |
| `read_backup` | Function | `crates/sigillum-daemon/src/json_store.rs` | 228 |
| `sync_backup` | Function | `crates/sigillum-daemon/src/json_store.rs` | 274 |
| `quarantine_corrupt_file` | Function | `crates/sigillum-daemon/src/json_store.rs` | 291 |

## Execution Flows

| Flow | Type | Steps |
|------|------|-------|
| `Eth_xpub_export → Backup_path` | cross_community | 7 |
| `Process_jobs → Backup_path` | cross_community | 7 |
| `Evm_provider_profiles_delete → Backup_path` | cross_community | 7 |
| `Eth_stealth_wallet_profiles_delete → Backup_path` | cross_community | 7 |
| `Eth_xpub_wallet_profiles_delete → Backup_path` | cross_community | 7 |
| `Run_maintenance → Backup_path` | cross_community | 7 |
| `Create_eth_stealth_erc20_deposit → Backup_path` | cross_community | 7 |
| `Delete_eth_stealth_deposit → Backup_path` | cross_community | 7 |
| `Refresh_eth_stealth_deposits → Backup_path` | cross_community | 7 |
| `Enqueue_eth_stealth_deposit_sweep → Backup_path` | cross_community | 7 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Static | 1 calls |
| Cluster_49 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "atomic_write"})` — see callers and callees
2. `gitnexus_query({query: "cluster_0"})` — find related execution flows
3. Read key files listed above for implementation details
