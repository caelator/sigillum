---
name: routes
description: "Skill for the Routes area of sigillum. 248 symbols across 30 files."
---

# Routes

248 symbols | 30 files | Cohesion: 88%

## When to Use

- Working with code in `crates/`
- Understanding how new, with_http_client, session_token work
- Modifying routes-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-client/src/lib.rs` | new, with_http_client, session_token, set_session_token, clear_session_token (+76) |
| `crates/sigillum-cli/src/daemon_api.rs` | cmd_api, cmd_api_profiles, cmd_api_deposits, cmd_api_queue, cmd_api_maintenance (+22) |
| `crates/sigillum-daemon/src/routes/mod.rs` | service_response, bearer_token, validated, api_router, api_routes (+14) |
| `crates/sigillum-gateway/src/routes/payments.rs` | resolve_effective_chain_id, idempotency_matches, created_payment_response, existing_payment_response, is_unique_constraint (+6) |
| `crates/sigillum-daemon/src/routes/profiles.rs` | evm_provider_profiles_list, evm_provider_profiles_upsert, evm_provider_profiles_delete, eth_stealth_wallet_profiles_list, eth_stealth_wallet_profiles_upsert (+6) |
| `crates/sigillum-gateway/src/webhooks.rs` | sign_payload, normalize_pem, deterministic_delivery_id, sign_besatas_payload, build_project_webhook_request (+6) |
| `crates/sigillum-daemon/src/routes/secrets.rs` | list_api_keys, get_api_key, set_api_key, delete_api_key, list_secrets (+4) |
| `crates/sigillum-daemon/src/routes/wallets.rs` | eth_xpub_export, eth_xpub_derive, eth_stealth_export, eth_stealth_generate, eth_stealth_check (+3) |
| `crates/sigillum-daemon/src/routes/fido2.rs` | fido2_status, fido2_set_pin, fido2_list, fido2_setup, fido2_register (+2) |
| `crates/sigillum-gateway/src/db.rs` | find_project_by_id, clear_webhook_retry, find_payment_by_idempotency_key, insert_project, find_payment_by_id (+2) |

## Entry Points

Start here when exploring this area:

- **`new`** (Function) — `crates/sigillum-client/src/lib.rs:122`
- **`with_http_client`** (Function) — `crates/sigillum-client/src/lib.rs:135`
- **`session_token`** (Function) — `crates/sigillum-client/src/lib.rs:146`
- **`set_session_token`** (Function) — `crates/sigillum-client/src/lib.rs:153`
- **`clear_session_token`** (Function) — `crates/sigillum-client/src/lib.rs:157`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `new` | Function | `crates/sigillum-client/src/lib.rs` | 122 |
| `with_http_client` | Function | `crates/sigillum-client/src/lib.rs` | 135 |
| `session_token` | Function | `crates/sigillum-client/src/lib.rs` | 146 |
| `set_session_token` | Function | `crates/sigillum-client/src/lib.rs` | 153 |
| `clear_session_token` | Function | `crates/sigillum-client/src/lib.rs` | 157 |
| `status` | Function | `crates/sigillum-client/src/lib.rs` | 164 |
| `unlock_with_passphrase` | Function | `crates/sigillum-client/src/lib.rs` | 169 |
| `lock` | Function | `crates/sigillum-client/src/lib.rs` | 181 |
| `revoke_session` | Function | `crates/sigillum-client/src/lib.rs` | 188 |
| `list_compartments` | Function | `crates/sigillum-client/src/lib.rs` | 199 |
| `switch_compartment` | Function | `crates/sigillum-client/src/lib.rs` | 207 |
| `list_api_keys` | Function | `crates/sigillum-client/src/lib.rs` | 219 |
| `get_api_key` | Function | `crates/sigillum-client/src/lib.rs` | 224 |
| `set_api_key` | Function | `crates/sigillum-client/src/lib.rs` | 234 |
| `delete_api_key` | Function | `crates/sigillum-client/src/lib.rs` | 245 |
| `list_secrets` | Function | `crates/sigillum-client/src/lib.rs` | 255 |
| `get_secret` | Function | `crates/sigillum-client/src/lib.rs` | 260 |
| `set_secret` | Function | `crates/sigillum-client/src/lib.rs` | 270 |
| `delete_secret` | Function | `crates/sigillum-client/src/lib.rs` | 281 |
| `export_snapshot` | Function | `crates/sigillum-client/src/lib.rs` | 294 |

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
| Service | 74 calls |
| Tests | 9 calls |
| Support | 4 calls |
| Cluster_78 | 3 calls |
| Cluster_6 | 2 calls |
| Cluster_69 | 1 calls |
| Cluster_71 | 1 calls |
| Cluster_70 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "new"})` — see callers and callees
2. `gitnexus_query({query: "routes"})` — find related execution flows
3. Read key files listed above for implementation details
