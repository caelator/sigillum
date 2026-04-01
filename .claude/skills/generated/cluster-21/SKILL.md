---
name: cluster-21
description: "Skill for the Cluster_21 area of sigillum. 9 symbols across 2 files."
---

# Cluster_21

9 symbols | 2 files | Cohesion: 75%

## When to Use

- Working with code in `crates/`
- Understanding how new, status, is_enabled work
- Modifying cluster_21-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-fido2/src/lib.rs` | new, status, is_enabled, test_manager, status_empty (+3) |
| `crates/sigillum-fido2/src/config.rs` | is_fido2_enabled |

## Entry Points

Start here when exploring this area:

- **`new`** (Function) — `crates/sigillum-fido2/src/lib.rs:143`
- **`status`** (Function) — `crates/sigillum-fido2/src/lib.rs:189`
- **`is_enabled`** (Function) — `crates/sigillum-fido2/src/lib.rs:210`
- **`is_fido2_enabled`** (Function) — `crates/sigillum-fido2/src/config.rs:102`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `new` | Function | `crates/sigillum-fido2/src/lib.rs` | 143 |
| `status` | Function | `crates/sigillum-fido2/src/lib.rs` | 189 |
| `is_enabled` | Function | `crates/sigillum-fido2/src/lib.rs` | 210 |
| `is_fido2_enabled` | Function | `crates/sigillum-fido2/src/config.rs` | 102 |
| `test_manager` | Function | `crates/sigillum-fido2/src/lib.rs` | 845 |
| `status_empty` | Function | `crates/sigillum-fido2/src/lib.rs` | 852 |
| `list_keys_empty` | Function | `crates/sigillum-fido2/src/lib.rs` | 860 |
| `status_with_keys` | Function | `crates/sigillum-fido2/src/lib.rs` | 934 |
| `malformed_config_surfaces_as_error` | Function | `crates/sigillum-fido2/src/lib.rs` | 955 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Tests | 3 calls |

## How to Explore

1. `gitnexus_context({name: "new"})` — see callers and callees
2. `gitnexus_query({query: "cluster_21"})` — find related execution flows
3. Read key files listed above for implementation details
