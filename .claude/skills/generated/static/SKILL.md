---
name: static
description: "Skill for the Static area of sigillum. 13 symbols across 2 files."
---

# Static

13 symbols | 2 files | Cohesion: 92%

## When to Use

- Working with code in `crates/`
- Understanding how sync_parent_dir, open, close work
- Modifying static-related functionality

## Key Files

| File | Symbols |
|------|---------|
| `crates/sigillum-gateway/static/widget.js` | open, close, clearPollState, createModalShell, showModal (+7) |
| `crates/sigillum-core/src/utils.rs` | sync_parent_dir |

## Key Symbols

| Symbol | Type | File | Line |
|--------|------|------|------|
| `sync_parent_dir` | Function | `crates/sigillum-core/src/utils.rs` | 58 |
| `clearPollState` | Function | `crates/sigillum-gateway/static/widget.js` | 72 |
| `createModalShell` | Function | `crates/sigillum-gateway/static/widget.js` | 79 |
| `showModal` | Function | `crates/sigillum-gateway/static/widget.js` | 101 |
| `showPaymentDetails` | Function | `crates/sigillum-gateway/static/widget.js` | 122 |
| `setStatus` | Function | `crates/sigillum-gateway/static/widget.js` | 201 |
| `formatExpiry` | Function | `crates/sigillum-gateway/static/widget.js` | 214 |
| `schedulePoll` | Function | `crates/sigillum-gateway/static/widget.js` | 221 |
| `pollPayment` | Function | `crates/sigillum-gateway/static/widget.js` | 229 |
| `formatAmount` | Function | `crates/sigillum-gateway/static/widget.js` | 267 |
| `ensureStyles` | Function | `crates/sigillum-gateway/static/widget.js` | 277 |
| `open` | Method | `crates/sigillum-gateway/static/widget.js` | 28 |
| `close` | Method | `crates/sigillum-gateway/static/widget.js` | 64 |

## How to Explore

1. `gitnexus_context({name: "sync_parent_dir"})` — see callers and callees
2. `gitnexus_query({query: "static"})` — find related execution flows
3. Read key files listed above for implementation details
