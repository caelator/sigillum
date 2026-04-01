---
name: cluster-15
description: "Skill for the Cluster_15 area of sigillum. 7 symbols across 1 files."
---

# Cluster_15

7 symbols | 1 files | Cohesion: 88%

## When to Use

- Working with code in `crates/`
- Understanding how rlp_encode_u64, rlp_encode_quantity, rlp_encode_bytes work
- Modifying cluster_15-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-core/src/ethereum_stealth.rs` | rlp_encode_u64, rlp_encode_quantity, rlp_encode_bytes, trim_leading_zeroes, minimal_be_bytes_from_u64 (+2) |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `rlp_encode_u64` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 734 |
| `rlp_encode_quantity` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 739 |
| `rlp_encode_bytes` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 765 |
| `trim_leading_zeroes` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 784 |
| `minimal_be_bytes_from_u64` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 792 |
| `minimal_be_bytes_from_usize` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 805 |
| `encode_quantity_hex` | Function | `crates/sigillum-core/src/ethereum_stealth.rs` | 813 |

## How to Explore

1. `gitnexus_context({name: "rlp_encode_u64"})` — see callers and callees
2. `gitnexus_query({query: "cluster_15"})` — find related execution flows
3. Read key files listed above for implementation details
