---
name: support
description: "Skill for the Support area of sigillum. 20 symbols across 4 files."
---

# Support

20 symbols | 4 files | Cohesion: 83%

## When to Use

- Working with code in `crates/`
- Understanding how sec_headers, err, ok_json work
- Modifying support-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-gateway/tests/support/mod.rs` | export_meta_address, generate_address, create_native_deposit, create_erc20_deposit, delete_deposit (+9) |
| `crates/sigillum-daemon/src/routes/mod.rs` | sec_headers, err, ok_json |
| `crates/sigillum-gateway/src/db.rs` | connect, test_pool |
| `crates/sigillum-gateway/src/error.rs` | into_response |

## Entry Points

Start here when exploring this area:

- **`sec_headers`** (Function) — `crates/sigillum-daemon/src/routes/mod.rs:74`
- **`err`** (Function) — `crates/sigillum-daemon/src/routes/mod.rs:81`
- **`ok_json`** (Function) — `crates/sigillum-daemon/src/routes/mod.rs:93`
- **`connect`** (Function) — `crates/sigillum-gateway/src/db.rs:67`
- **`sqlite_row_count`** (Function) — `crates/sigillum-gateway/tests/support/mod.rs:263`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `sec_headers` | Function | `crates/sigillum-daemon/src/routes/mod.rs` | 74 |
| `err` | Function | `crates/sigillum-daemon/src/routes/mod.rs` | 81 |
| `ok_json` | Function | `crates/sigillum-daemon/src/routes/mod.rs` | 93 |
| `connect` | Function | `crates/sigillum-gateway/src/db.rs` | 67 |
| `sqlite_row_count` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 263 |
| `sqlite_payment_by_idempotency` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 278 |
| `install_payment_insert_failure_trigger` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 297 |
| `spawn` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 76 |
| `into_response` | Function | `crates/sigillum-gateway/src/error.rs` | 29 |
| `export_meta_address` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 378 |
| `generate_address` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 408 |
| `create_native_deposit` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 424 |
| `create_erc20_deposit` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 449 |
| `delete_deposit` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 474 |
| `test_pool` | Function | `crates/sigillum-gateway/src/db.rs` | 316 |
| `pick_free_port` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 321 |
| `list_wallet_profiles` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 338 |
| `default_wallet_profiles` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 528 |
| `list_provider_profiles` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 362 |
| `default_provider_profiles` | Function | `crates/sigillum-gateway/tests/support/mod.rs` | 540 |

## Execution Flows

| Flow | Type | Steps |
|------|------|-------|
| `Eth_xpub_export → Common_security_headers` | cross_community | 5 |
| `Eth_stealth_export → Common_security_headers` | cross_community | 5 |
| `Eth_stealth_check → Common_security_headers` | cross_community | 5 |
| `Eth_stealth_sign → Common_security_headers` | cross_community | 5 |
| `Eth_stealth_sign_transfer → Common_security_headers` | cross_community | 5 |
| `Eth_stealth_sign_erc20_transfer → Common_security_headers` | cross_community | 5 |
| `Transit_encrypt → Common_security_headers` | cross_community | 5 |
| `Transit_decrypt → Common_security_headers` | cross_community | 5 |
| `Transit_hmac → Common_security_headers` | cross_community | 5 |
| `Get_api_key → Common_security_headers` | cross_community | 5 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Routes | 1 calls |

## How to Explore

1. `gitnexus_context({name: "sec_headers"})` — see callers and callees
2. `gitnexus_query({query: "support"})` — find related execution flows
3. Read key files listed above for implementation details
