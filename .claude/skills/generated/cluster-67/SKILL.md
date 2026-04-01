---
name: cluster-67
description: "Skill for the Cluster_67 area of sigillum. 50 symbols across 1 files."
---

# Cluster_67

50 symbols | 1 files | Cohesion: 100%

## When to Use

- Working with code in `crates/`
- Understanding how roundtrip_test, test_key_value_request_roundtrip, test_key_value_request_none_value work
- Modifying cluster_67-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-api/src/request.rs` | roundtrip_test, test_key_value_request_roundtrip, test_key_value_request_none_value, test_key_only_request_roundtrip, test_passphrase_request_roundtrip (+45) |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `roundtrip_test` | Function | `crates/sigillum-api/src/request.rs` | 659 |
| `test_key_value_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 668 |
| `test_key_value_request_none_value` | Function | `crates/sigillum-api/src/request.rs` | 677 |
| `test_key_only_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 686 |
| `test_passphrase_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 694 |
| `test_snapshot_restore_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 702 |
| `test_setup_reset_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 711 |
| `test_compartment_definition_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 719 |
| `test_fido2_setup_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 729 |
| `test_fido2_setup_request_without_pin_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 744 |
| `test_fido2_set_pin_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 759 |
| `test_fido2_register_request_without_pin_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 767 |
| `test_fido2_unlock_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 778 |
| `test_fido2_unlock_request_without_pins_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 787 |
| `test_fido2_remove_request_without_pin_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 796 |
| `test_compartment_init_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 806 |
| `test_secrets_push_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 817 |
| `test_transit_encrypt_request_roundtrip` | Function | `crates/sigillum-api/src/request.rs` | 829 |
| `test_transit_decrypt_request_with_aad` | Function | `crates/sigillum-api/src/request.rs` | 839 |
| `test_transit_decrypt_request_no_aad` | Function | `crates/sigillum-api/src/request.rs` | 850 |

## How to Explore

1. `gitnexus_context({name: "roundtrip_test"})` — see callers and callees
2. `gitnexus_query({query: "cluster_67"})` — find related execution flows
3. Read key files listed above for implementation details
