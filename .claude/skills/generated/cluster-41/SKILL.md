---
name: cluster-41
description: "Skill for the Cluster_41 area of sigillum. 11 symbols across 1 files."
---

# Cluster_41

11 symbols | 1 files | Cohesion: 62%

## When to Use

- Working with code in `crates/`
- Understanding how load_profiles, save_profiles, profiles_path work
- Modifying cluster_41-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/src/profiles.rs` | load_profiles, save_profiles, profiles_path, load_returns_default_when_no_file, save_and_load_roundtrip (+6) |

## Entry Points

Start here when exploring this area:

- **`load_profiles`** (Function) — `crates/sigillum-daemon/src/profiles.rs:27`
- **`save_profiles`** (Function) — `crates/sigillum-daemon/src/profiles.rs:32`
- **`profiles_path`** (Function) — `crates/sigillum-daemon/src/profiles.rs:40`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `load_profiles` | Function | `crates/sigillum-daemon/src/profiles.rs` | 27 |
| `save_profiles` | Function | `crates/sigillum-daemon/src/profiles.rs` | 32 |
| `profiles_path` | Function | `crates/sigillum-daemon/src/profiles.rs` | 40 |
| `load_returns_default_when_no_file` | Function | `crates/sigillum-daemon/src/profiles.rs` | 52 |
| `save_and_load_roundtrip` | Function | `crates/sigillum-daemon/src/profiles.rs` | 61 |
| `corrupted_json_returns_error` | Function | `crates/sigillum-daemon/src/profiles.rs` | 89 |
| `save_creates_parent_directories` | Function | `crates/sigillum-daemon/src/profiles.rs` | 100 |
| `restores_from_backup_when_live_file_is_missing` | Function | `crates/sigillum-daemon/src/profiles.rs` | 109 |
| `corrupt_live_file_is_quarantined_and_restored_from_backup` | Function | `crates/sigillum-daemon/src/profiles.rs` | 135 |
| `save_writes_versioned_schema_envelope` | Function | `crates/sigillum-daemon/src/profiles.rs` | 178 |
| `legacy_unwrapped_profiles_still_load` | Function | `crates/sigillum-daemon/src/profiles.rs` | 194 |

## Execution Flows

| Flow | Type | Steps |
|------|------|-------|
| `Eth_xpub_export → Backup_path` | cross_community | 7 |
| `Evm_provider_profiles_delete → Backup_path` | cross_community | 7 |
| `Eth_stealth_wallet_profiles_delete → Backup_path` | cross_community | 7 |
| `Eth_xpub_wallet_profiles_delete → Backup_path` | cross_community | 7 |
| `Create_eth_stealth_erc20_deposit → Backup_path` | cross_community | 7 |
| `Evm_provider_profiles_delete → Quarantine_corrupt_file` | cross_community | 6 |
| `Eth_stealth_wallet_profiles_delete → Quarantine_corrupt_file` | cross_community | 6 |
| `Eth_xpub_wallet_profiles_delete → Quarantine_corrupt_file` | cross_community | 6 |
| `Eth_xpub_export → Profiles_path` | cross_community | 5 |
| `Eth_stealth_send_with_profile → Profiles_path` | cross_community | 5 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Cluster_50 | 1 calls |
| Cluster_51 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "load_profiles"})` — see callers and callees
2. `gitnexus_query({query: "cluster_41"})` — find related execution flows
3. Read key files listed above for implementation details
