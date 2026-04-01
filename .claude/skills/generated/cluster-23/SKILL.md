---
name: cluster-23
description: "Skill for the Cluster_23 area of sigillum. 13 symbols across 2 files."
---

# Cluster_23

13 symbols | 2 files | Cohesion: 77%

## When to Use

- Working with code in `crates/`
- Understanding how set_new_pin, make_credential, make_credential_with_hmac work
- Modifying cluster_23-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-fido2/src/hid.rs` | normalize_pin, with_fido_timeout, open_device, get_single_attached_device, make_credential_with_device (+7) |
| `crates/sigillum-fido2/src/crypto.rs` | application_salt |

## Entry Points

Start here when exploring this area:

- **`set_new_pin`** (Function) — `crates/sigillum-fido2/src/hid.rs:221`
- **`make_credential`** (Function) — `crates/sigillum-fido2/src/hid.rs:259`
- **`make_credential_with_hmac`** (Function) — `crates/sigillum-fido2/src/hid.rs:265`
- **`get_hmac_secret`** (Function) — `crates/sigillum-fido2/src/hid.rs:290`
- **`application_salt`** (Function) — `crates/sigillum-fido2/src/crypto.rs:48`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `set_new_pin` | Function | `crates/sigillum-fido2/src/hid.rs` | 221 |
| `make_credential` | Function | `crates/sigillum-fido2/src/hid.rs` | 259 |
| `make_credential_with_hmac` | Function | `crates/sigillum-fido2/src/hid.rs` | 265 |
| `get_hmac_secret` | Function | `crates/sigillum-fido2/src/hid.rs` | 290 |
| `application_salt` | Function | `crates/sigillum-fido2/src/crypto.rs` | 48 |
| `normalize_pin` | Function | `crates/sigillum-fido2/src/hid.rs` | 39 |
| `with_fido_timeout` | Function | `crates/sigillum-fido2/src/hid.rs` | 44 |
| `open_device` | Function | `crates/sigillum-fido2/src/hid.rs` | 85 |
| `get_single_attached_device` | Function | `crates/sigillum-fido2/src/hid.rs` | 92 |
| `make_credential_with_device` | Function | `crates/sigillum-fido2/src/hid.rs` | 103 |
| `get_hmac_secret_with_device` | Function | `crates/sigillum-fido2/src/hid.rs` | 133 |
| `select_registration_device` | Function | `crates/sigillum-fido2/src/hid.rs` | 173 |
| `short_new_pin_is_rejected_before_hid_access` | Function | `crates/sigillum-fido2/src/hid.rs` | 384 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Classif | 3 calls |
| Tests | 2 calls |
| Routes | 1 calls |

## How to Explore

1. `gitnexus_context({name: "set_new_pin"})` — see callers and callees
2. `gitnexus_query({query: "cluster_23"})` — find related execution flows
3. Read key files listed above for implementation details
