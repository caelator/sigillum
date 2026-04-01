---
name: cluster-42
description: "Skill for the Cluster_42 area of sigillum. 7 symbols across 1 files."
---

# Cluster_42

7 symbols | 1 files | Cohesion: 100%

## When to Use

- Working with code in `crates/`
- Understanding how from_env work
- Modifying cluster_42-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-daemon/src/policy.rs` | default, from_env, from_pairs, from_overrides, runtime_policy_defaults_match_expected_baseline (+2) |

## Entry Points

Start here when exploring this area:

- **`from_env`** (Function) — `crates/sigillum-daemon/src/policy.rs:86`

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `from_env` | Function | `crates/sigillum-daemon/src/policy.rs` | 86 |
| `default` | Function | `crates/sigillum-daemon/src/policy.rs` | 80 |
| `from_pairs` | Function | `crates/sigillum-daemon/src/policy.rs` | 90 |
| `from_overrides` | Function | `crates/sigillum-daemon/src/policy.rs` | 134 |
| `runtime_policy_defaults_match_expected_baseline` | Function | `crates/sigillum-daemon/src/policy.rs` | 234 |
| `runtime_policy_sanitizes_invalid_overrides` | Function | `crates/sigillum-daemon/src/policy.rs` | 249 |
| `runtime_policy_clamps_requested_limits_and_retry_backoff` | Function | `crates/sigillum-daemon/src/policy.rs` | 274 |

## How to Explore

1. `gitnexus_context({name: "from_env"})` — see callers and callees
2. `gitnexus_query({query: "cluster_42"})` — find related execution flows
3. Read key files listed above for implementation details
