---
name: service
description: "Skill for the Service area of sigillum. 368 symbols across 35 files."
---

# Service

368 symbols | 35 files | Cohesion: 76%

## When to Use

- Working with code in `crates/`
- Understanding how meta_address, setup_dummy_directories, generate_dummy_file work
- Modifying service-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/src/state.rs` | token_matches, http_client, session_key_for, default_active_compartment_id, active_compartment_id_for (+34) |
| `crates/sigillum-daemon/src/service/evm.rs` | evm_nonce, evm_broadcast, eth_stealth_send_transfer, eth_stealth_send_erc20_transfer, resolve_provider_rpc_client (+33) |
| `crates/sigillum-cli/src/main.rs` | main, print_usage, discover_unlocked_compartments, require_unlocked_compartments, cmd_setup (+33) |
| `crates/sigillum-daemon/src/service/recovery.rs` | recover_snapshot_operation, snapshot_temp_path, snapshot_placeholder_dir, recover_runtime_state, startup_recovery_finalizes_snapshot_journal_after_filesystem_recovery (+17) |
| `crates/sigillum-daemon/src/service/queue.rs` | list_queue_jobs, enqueue_job, process_queue, queue_error_classification_distinguishes_retryable_failures, process_queue_state (+15) |
| `crates/sigillum-daemon/src/service/fido2.rs` | map_other_fido2_message, map_fido2_service_error, optional_pin, fido2_status, fido2_set_pin (+14) |
| `crates/sigillum-daemon/src/service/profiles.rs` | list_evm_provider_profiles, upsert_evm_provider_profile, delete_evm_provider_profile, list_eth_stealth_wallet_profiles, upsert_eth_stealth_wallet_profile (+13) |
| `crates/sigillum-daemon/src/service/mod.rs` | require_session, optional_session, record_audit, begin_operation, with_active_vault (+8) |
| `crates/sigillum-daemon/src/service/helpers.rs` | decode_hex, decode_optional_hex, decode_fixed_hex, decode_optional_view_tag, map_wallet_error (+8) |
| `crates/sigillum-daemon/src/service/deposits.rs` | list_eth_stealth_deposits, persist_new_deposit, delete_eth_stealth_deposit, refresh_eth_stealth_deposits, enqueue_eth_stealth_deposit_sweep (+8) |

## Entry Points

Start here when exploring this area:

- **`meta_address`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:147`
- **`setup_dummy_directories`** (Function) — `crates/sigillum-fido2/src/lib.rs:270`
- **`generate_dummy_file`** (Function) — `crates/sigillum-fido2/src/crypto.rs:190`
- **`http_client`** (Function) — `crates/sigillum-daemon/src/state.rs:213`
- **`default_active_compartment_id`** (Function) — `crates/sigillum-daemon/src/state.rs:233`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `meta_address` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 147 |
| `setup_dummy_directories` | Function | `crates/sigillum-fido2/src/lib.rs` | 270 |
| `generate_dummy_file` | Function | `crates/sigillum-fido2/src/crypto.rs` | 190 |
| `http_client` | Function | `crates/sigillum-daemon/src/state.rs` | 213 |
| `default_active_compartment_id` | Function | `crates/sigillum-daemon/src/state.rs` | 233 |
| `active_compartment_id_for` | Function | `crates/sigillum-daemon/src/state.rs` | 243 |
| `compartment_dir` | Function | `crates/sigillum-daemon/src/state.rs` | 255 |
| `operation_guard` | Function | `crates/sigillum-daemon/src/state.rs` | 259 |
| `salt_path` | Function | `crates/sigillum-daemon/src/state.rs` | 263 |
| `wrapped_key_path` | Function | `crates/sigillum-daemon/src/state.rs` | 267 |
| `ensure_vault` | Function | `crates/sigillum-daemon/src/state.rs` | 284 |
| `with_active_vault_for` | Function | `crates/sigillum-daemon/src/state.rs` | 299 |
| `unlock_compartment` | Function | `crates/sigillum-daemon/src/state.rs` | 321 |
| `unlock_multiple` | Function | `crates/sigillum-daemon/src/state.rs` | 332 |
| `check_unlock_throttle` | Function | `crates/sigillum-daemon/src/state.rs` | 348 |
| `record_unlock_failure` | Function | `crates/sigillum-daemon/src/state.rs` | 365 |
| `reset_unlock_throttle` | Function | `crates/sigillum-daemon/src/state.rs` | 372 |
| `create_session` | Function | `crates/sigillum-daemon/src/state.rs` | 384 |
| `switch_active_for` | Function | `crates/sigillum-daemon/src/state.rs` | 423 |
| `unlocked_compartments` | Function | `crates/sigillum-daemon/src/state.rs` | 437 |

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
| Tests | 39 calls |
| Cluster_6 | 29 calls |
| Cluster_41 | 20 calls |
| Cluster_13 | 15 calls |
| Routes | 7 calls |
| Cluster_46 | 7 calls |
| Cluster_19 | 6 calls |
| Cluster_0 | 5 calls |

## How to Explore

1. `gitnexus_context({name: "meta_address"})` — see callers and callees
2. `gitnexus_query({query: "service"})` — find related execution flows
3. Read key files listed above for implementation details
