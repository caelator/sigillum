---
name: classif
description: "Skill for the Classif area of sigillum. 7 symbols across 1 files."
---

# Classif

7 symbols | 1 files | Cohesion: 80%

## When to Use

- Working with code in `crates/`
- Understanding how classify_ctap_error, classifies_pin_auth_blocked_errors, classifies_pin_not_set_errors work
- Modifying classif-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-fido2/src/hid.rs` | classify_ctap_error, classifies_pin_auth_blocked_errors, classifies_pin_not_set_errors, classifies_pin_required_errors, classifies_no_matching_credential_errors (+2) |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `classify_ctap_error` | Function | `crates/sigillum-fido2/src/hid.rs` | 17 |
| `classifies_pin_auth_blocked_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 330 |
| `classifies_pin_not_set_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 339 |
| `classifies_pin_required_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 348 |
| `classifies_no_matching_credential_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 357 |
| `classifies_pin_blocked_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 366 |
| `classifies_incorrect_pin_errors` | Function | `crates/sigillum-fido2/src/hid.rs` | 375 |

## How to Explore

1. `gitnexus_context({name: "classify_ctap_error"})` — see callers and callees
2. `gitnexus_query({query: "classif"})` — find related execution flows
3. Read key files listed above for implementation details
