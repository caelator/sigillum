---
name: cluster-66
description: "Skill for the Cluster_66 area of sigillum. 64 symbols across 1 files."
---

# Cluster_66

64 symbols | 1 files | Cohesion: 100%

## When to Use

- Working with code in `crates/`
- Understanding how roundtrip_test, test_error_response_roundtrip, test_active_compartment_roundtrip work
- Modifying cluster_66-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-api/src/response.rs` | roundtrip_test, test_error_response_roundtrip, test_active_compartment_roundtrip, test_active_compartment_no_secret_count, test_unlocked_compartment_roundtrip (+59) |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `roundtrip_test` | Function | `crates/sigillum-api/src/response.rs` | 724 |
| `test_error_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 733 |
| `test_active_compartment_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 741 |
| `test_active_compartment_no_secret_count` | Function | `crates/sigillum-api/src/response.rs` | 752 |
| `test_unlocked_compartment_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 763 |
| `test_status_response_full` | Function | `crates/sigillum-api/src/response.rs` | 774 |
| `test_status_response_locked` | Function | `crates/sigillum-api/src/response.rs` | 799 |
| `test_lock_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 811 |
| `test_unlock_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 820 |
| `test_generic_status_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 838 |
| `test_key_list_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 846 |
| `test_key_value_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 854 |
| `test_key_mutation_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 863 |
| `test_push_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 873 |
| `test_compartment_info_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 884 |
| `test_compartment_list_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 896 |
| `test_fido2_setup_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 919 |
| `test_transit_encrypt_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 932 |
| `test_transit_decrypt_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 942 |
| `test_transit_hmac_response_roundtrip` | Function | `crates/sigillum-api/src/response.rs` | 951 |

## How to Explore

1. `gitnexus_context({name: "roundtrip_test"})` — see callers and callees
2. `gitnexus_query({query: "cluster_66"})` — find related execution flows
3. Read key files listed above for implementation details
