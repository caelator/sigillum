---
name: cluster-8
description: "Skill for the Cluster_8 area of sigillum. 8 symbols across 1 files."
---

# Cluster_8

8 symbols | 1 files | Cohesion: 64%

## When to Use

- Working with code in `crates/`
- Understanding how export_encrypted_snapshot, restore_encrypted_snapshot, inspect_encrypted_snapshot work
- Modifying cluster_8-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-core/src/snapshot.rs` | export_encrypted_snapshot, restore_encrypted_snapshot, inspect_encrypted_snapshot, decode_encrypted_snapshot, decode_fixed_hex (+3) |

## Entry Points

Start here when exploring this area:

- **`export_encrypted_snapshot`** (Function) — `crates/sigillum-core/src/snapshot.rs:89`
- **`restore_encrypted_snapshot`** (Function) — `crates/sigillum-core/src/snapshot.rs:149`
- **`inspect_encrypted_snapshot`** (Function) — `crates/sigillum-core/src/snapshot.rs:162`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `export_encrypted_snapshot` | Function | `crates/sigillum-core/src/snapshot.rs` | 89 |
| `restore_encrypted_snapshot` | Function | `crates/sigillum-core/src/snapshot.rs` | 149 |
| `inspect_encrypted_snapshot` | Function | `crates/sigillum-core/src/snapshot.rs` | 162 |
| `decode_encrypted_snapshot` | Function | `crates/sigillum-core/src/snapshot.rs` | 320 |
| `decode_fixed_hex` | Function | `crates/sigillum-core/src/snapshot.rs` | 402 |
| `snapshot_roundtrip_restores_tree` | Function | `crates/sigillum-core/src/snapshot.rs` | 425 |
| `snapshot_restore_rejects_wrong_passphrase` | Function | `crates/sigillum-core/src/snapshot.rs` | 461 |
| `inspect_snapshot_returns_summary_without_restoring` | Function | `crates/sigillum-core/src/snapshot.rs` | 473 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Service | 2 calls |
| Cluster_3 | 2 calls |

## How to Explore

1. `gitnexus_context({name: "export_encrypted_snapshot"})` — see callers and callees
2. `gitnexus_query({query: "cluster_8"})` — find related execution flows
3. Read key files listed above for implementation details
