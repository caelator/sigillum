---
name: cluster-13
description: "Skill for the Cluster_13 area of sigillum. 34 symbols across 1 files."
---

# Cluster_13

34 symbols | 1 files | Cohesion: 86%

## When to Use

- Working with code in `crates/`
- Understanding how derive_sigillum_ethereum_stealth_wallet, generate_ethereum_stealth_address, check_ethereum_stealth_address work
- Modifying cluster_13-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-core/src/ethereum_stealth.rs` | derive_sigillum_ethereum_stealth_wallet, generate_ethereum_stealth_address, check_ethereum_stealth_address, derive_verified_stealth_key, sign_ethereum_stealth_digest (+29) |

## Entry Points

Start here when exploring this area:

- **`derive_sigillum_ethereum_stealth_wallet`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:165`
- **`generate_ethereum_stealth_address`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:215`
- **`check_ethereum_stealth_address`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:256`
- **`sign_ethereum_stealth_digest`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:350`
- **`sign_ethereum_stealth_native_transfer`** (Function) — `crates/sigillum-core/src/ethereum_stealth.rs:397`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `derive_sigillum_ethereum_stealth_wallet` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 165 |
| `generate_ethereum_stealth_address` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 215 |
| `check_ethereum_stealth_address` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 256 |
| `sign_ethereum_stealth_digest` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 350 |
| `sign_ethereum_stealth_native_transfer` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 397 |
| `sign_ethereum_stealth_erc20_transfer` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 439 |
| `derive_verified_stealth_key` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 305 |
| `derive_wallet_secret_key` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 470 |
| `sign_ethereum_stealth_eip1559_transaction` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 497 |
| `parse_meta_address` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 543 |
| `parse_public_key_hex` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 575 |
| `ephemeral_private_key_to_secret` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 606 |
| `hashed_shared_secret` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 624 |
| `hashed_shared_secret_for_recipient` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 632 |
| `derive_stealth_public_key` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 639 |
| `derive_stealth_private_key` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 652 |
| `encode_recoverable_signature` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 663 |
| `encode_erc20_transfer_data` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 670 |
| `encode_eip1559_signing_payload` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 683 |
| `encode_eip1559_signed_payload` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 703 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Cluster_15 | 2 calls |
| Service | 2 calls |
| Tests | 1 calls |

## How to Explore

1. `gitnexus_context({name: "derive_sigillum_ethereum_stealth_wallet"})` — see callers and callees
2. `gitnexus_query({query: "cluster_13"})` — find related execution flows
3. Read key files listed above for implementation details
