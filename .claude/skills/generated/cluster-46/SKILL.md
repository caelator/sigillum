---
name: cluster-46
description: "Skill for the Cluster_46 area of sigillum. 8 symbols across 1 files."
---

# Cluster_46

8 symbols | 1 files | Cohesion: 69%

## When to Use

- Working with code in `crates/`
- Understanding how kind, compartment_add, list_pending_operations work
- Modifying cluster_46-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/src/operations.rs` | kind, compartment_add, list_pending_operations, clear_pending_operation, operations_dir (+3) |

## Entry Points

Start here when exploring this area:

- **`kind`** (Function) — `crates/sigillum-daemon/src/operations.rs:114`
- **`compartment_add`** (Function) — `crates/sigillum-daemon/src/operations.rs:130`
- **`list_pending_operations`** (Function) — `crates/sigillum-daemon/src/operations.rs:291`
- **`clear_pending_operation`** (Function) — `crates/sigillum-daemon/src/operations.rs:313`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `kind` | Function | `crates/sigillum-daemon/src/operations.rs` | 114 |
| `compartment_add` | Function | `crates/sigillum-daemon/src/operations.rs` | 130 |
| `list_pending_operations` | Function | `crates/sigillum-daemon/src/operations.rs` | 291 |
| `clear_pending_operation` | Function | `crates/sigillum-daemon/src/operations.rs` | 313 |
| `operations_dir` | Function | `crates/sigillum-daemon/src/operations.rs` | 328 |
| `save_writes_versioned_schema_envelope` | Function | `crates/sigillum-daemon/src/operations.rs` | 438 |
| `legacy_unwrapped_records_still_load` | Function | `crates/sigillum-daemon/src/operations.rs` | 462 |
| `unsupported_legacy_kind_returns_error` | Function | `crates/sigillum-daemon/src/operations.rs` | 497 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Cluster_49 | 1 calls |
| Service | 1 calls |

## How to Explore

1. `gitnexus_context({name: "kind"})` — see callers and callees
2. `gitnexus_query({query: "cluster_46"})` — find related execution flows
3. Read key files listed above for implementation details
