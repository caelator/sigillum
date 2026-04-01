---
name: tests
description: "Skill for the Tests area of sigillum. 159 symbols across 15 files."
---

# Tests

159 symbols | 15 files | Cohesion: 76%

## When to Use

- Working with code in `crates/`
- Understanding how base_url, counts, wait_until_ready work
- Modifying tests-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/tests/integration.rs` | test_generate_dummy_shards, test_app, get_request, test_get_status_locked_no_token, test_get_status_invalid_token (+31) |
| `crates/sigillum-cli/tests/cli_smoke.rs` | sigillum_bin, run, help_exits_zero_with_usage_text, help_flag_exits_zero, version_shows_version_info (+12) |
| `crates/sigillum-daemon/tests/daemon_service.rs` | spawn_daemon, spawn_mock_evm_provider, rpc_response, rpc_handler, post_json (+11) |
| `crates/sigillum-gateway/tests/gateway_integration.rs` | project_request, payment_request, create_project, create_payment, default_stub_config (+9) |
| `crates/sigillum-fido2/src/crypto.rs` | split_master_key, reconstruct_master_key, shamir_split_reconstruct_1_of_1, shamir_split_reconstruct_2_of_3, shamir_split_reconstruct_3_of_5 (+8) |
| `crates/sigillum-fido2/src/lib.rs` | load, save, normalize_pin, pin_for_round, save_config_raw (+7) |
| `crates/sigillum-gateway/tests/gateway_tests.rs` | sign_payload, verify_signature, sign_payload_deterministic, sign_payload_different_secrets, sign_payload_different_payloads (+7) |
| `crates/sigillum-daemon/tests/concurrent_sessions.rs` | spawn_daemon, post_json, get, setup_daemon, concurrent_api_key_writes_do_not_corrupt (+5) |
| `crates/sigillum-fido2/src/config.rs` | generate_dummy_shards, dummy_shards_correct_length, load_config, config_roundtrip, load_missing_returns_default (+4) |
| `crates/sigillum-daemon/tests/crash_recovery.rs` | spawn_daemon, post_json, get, setup_with_data, snapshot_export_and_restore_preserves_api_keys (+4) |

## Entry Points

Start here when exploring this area:

- **`base_url`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:128`
- **`counts`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:132`
- **`wait_until_ready`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:213`
- **`url`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:233`
- **`request_json`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:241`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `base_url` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 128 |
| `counts` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 132 |
| `wait_until_ready` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 213 |
| `url` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 233 |
| `request_json` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 241 |
| `get` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 255 |
| `as_str` | Function | `crates/sigillum-daemon/src/audit_log.rs` | 107 |
| `save_config_raw` | Function | `crates/sigillum-fido2/src/lib.rs` | 182 |
| `list_keys` | Function | `crates/sigillum-fido2/src/lib.rs` | 197 |
| `register_key` | Function | `crates/sigillum-fido2/src/lib.rs` | 340 |
| `remove_key` | Function | `crates/sigillum-fido2/src/lib.rs` | 492 |
| `register_key_poison` | Function | `crates/sigillum-fido2/src/lib.rs` | 592 |
| `authenticate_cascading` | Function | `crates/sigillum-fido2/src/lib.rs` | 691 |
| `generate_dummy_shards` | Function | `crates/sigillum-fido2/src/config.rs` | 141 |
| `fido2_list_keys` | Function | `crates/sigillum-daemon/src/service/fido2.rs` | 197 |
| `split_master_key` | Function | `crates/sigillum-fido2/src/crypto.rs` | 215 |
| `reconstruct_master_key` | Function | `crates/sigillum-fido2/src/crypto.rs` | 248 |
| `encrypt_shard_tagged` | Function | `crates/sigillum-fido2/src/crypto.rs` | 102 |
| `decrypt_shard_tagged` | Function | `crates/sigillum-fido2/src/crypto.rs` | 121 |
| `encrypt_compartment_meta` | Function | `crates/sigillum-fido2/src/crypto.rs` | 148 |

## Execution Flows

| Flow | Type | Steps |
|------|------|-------|
| `Main → Sigillum_bin` | cross_community | 5 |
| `Main → As_str` | cross_community | 4 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Routes | 6 calls |
| Cluster_23 | 5 calls |
| Cluster_24 | 4 calls |
| Service | 3 calls |
| Cluster_70 | 2 calls |
| Support | 2 calls |
| Cluster_21 | 1 calls |
| Cluster_0 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "base_url"})` — see callers and callees
2. `gitnexus_query({query: "tests"})` — find related execution flows
3. Read key files listed above for implementation details
