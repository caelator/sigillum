---
name: cluster-11
description: "Skill for the Cluster_11 area of sigillum. 7 symbols across 1 files."
---

# Cluster_11

7 symbols | 1 files | Cohesion: 82%

## When to Use

- Working with code in `crates/`
- Understanding how derive_sigillum_ethereum_xpub_receive_branch work
- Modifying cluster_11-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-core/src/ethereum_xpub.rs` | derive_sigillum_ethereum_xpub_receive_branch, derive_receive_branch_xprv, derive_account_xprv, hardened_child, receive_branch_child (+2) |

## Entry Points

Start here when exploring this area:

- **`derive_sigillum_ethereum_xpub_receive_branch`** (Function) — `crates/sigillum-core/src/ethereum_xpub.rs:41`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `derive_sigillum_ethereum_xpub_receive_branch` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 41 |
| `derive_receive_branch_xprv` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 83 |
| `derive_account_xprv` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 93 |
| `hardened_child` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 132 |
| `receive_branch_child` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 136 |
| `export_receive_branch_returns_xpub` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 155 |
| `xpub_requires_receive_branch_depth` | Function | `crates/sigillum-core/src/ethereum_xpub.rs` | 176 |

## How to Explore

1. `gitnexus_context({name: "derive_sigillum_ethereum_xpub_receive_branch"})` — see callers and callees
2. `gitnexus_query({query: "cluster_11"})` — find related execution flows
3. Read key files listed above for implementation details
