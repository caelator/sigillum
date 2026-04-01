---
name: cluster-19
description: "Skill for the Cluster_19 area of sigillum. 9 symbols across 1 files."
---

# Cluster_19

9 symbols | 1 files | Cohesion: 74%

## When to Use

- Working with code in `crates/`
- Understanding how find_unlocked_vault, cmd_set, cmd_get work
- Modifying cluster_19-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-cli/src/main.rs` | find_unlocked_vault, cmd_set, cmd_get, cmd_delete, cmd_set_api (+4) |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `find_unlocked_vault` | Function | `crates/sigillum-cli/src/main.rs` | 782 |
| `cmd_set` | Function | `crates/sigillum-cli/src/main.rs` | 795 |
| `cmd_get` | Function | `crates/sigillum-cli/src/main.rs` | 807 |
| `cmd_delete` | Function | `crates/sigillum-cli/src/main.rs` | 824 |
| `cmd_set_api` | Function | `crates/sigillum-cli/src/main.rs` | 876 |
| `cmd_get_api` | Function | `crates/sigillum-cli/src/main.rs` | 888 |
| `cmd_delete_api` | Function | `crates/sigillum-cli/src/main.rs` | 905 |
| `require_arg` | Function | `crates/sigillum-cli/src/main.rs` | 1341 |
| `prompt_secret` | Function | `crates/sigillum-cli/src/main.rs` | 1349 |

## Connected Areas

| Area | Connections |
|------|-------------|
| Service | 3 calls |
| Cluster_6 | 1 calls |

## How to Explore

1. `gitnexus_context({name: "find_unlocked_vault"})` — see callers and callees
2. `gitnexus_query({query: "cluster_19"})` — find related execution flows
3. Read key files listed above for implementation details
