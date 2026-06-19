# Sigillum — Production Readiness

**Date:** June 4, 2026 (updated June 19, 2026)
**Current Verdict:** The target is local-first single-host readiness; internet-facing and remote-platform scope is explicitly unsupported

## Summary

Sigillum now meets its current full-workspace release baseline for the
documented local-first scope. That baseline covers the local daemon, client,
core, CLI, and the `sigillum-gateway` sidecar:

- the workspace needs to stay green on the executable `./scripts/check-release.sh`
  gate, including metadata, architecture guardrails, daemon UI checks, tests,
  adversarial/fuzz checks, fmt, clippy, runtime smoke, npm/Rust audits, and deny
- clean-clone reproducibility depends on the committed `Cargo.lock`, Rust
  `1.88.0`, crates.io dependencies, and no local vendored SQLite patch
- Rust `1.88.0` supersedes the earlier `1.85.0` target because the current
  RustSec gates require the fixed `time` crate and a `cargo-deny` release that
  both have Rust `1.88` MSRVs
- the local daemon uses bearer session-token auth over local HTTP
- daemon startup creates or repairs its local base directory to `0700` on Unix
  before opening runtime state
- the gateway is part of the workspace release bar, but remains a local-sidecar
  preview surface in this phase
- the passphrase unlock path, daemon state recovery, and gateway/payment flows
  are all part of the same readiness story
- `sigillum doctor` is the operator preflight for local host readiness

The current evidence and remaining caveats are tracked in
[docs/production-readiness-audit.md](./docs/production-readiness-audit.md).

That does **not** mean the project is aiming toward a remote, multi-tenant, or
internet-facing deployment. The green bar here is explicitly the single-machine
operator model described in the README and deployment guide, because that is the
intended product boundary.

The release scope is **Sigillum Local-First Operator Console v1**. That scope
includes the local daemon, vault, CLI, session model, current wallet/inventory
slices, and local-sidecar gateway preview. It explicitly does not include the
full wallet-management roadmap: deeper seed/xpub gap-limit discovery, rich
NFT/DeFi/airdrop inventory, non-EVM chains, automated consolidation execution,
and internet-facing or hosted wallet operations remain future product work.

## What Is Ready

The current repository is in good shape for controlled single-machine use:

- file-backed vault core and wrapped-key lifecycle
- passphrase and FIDO2-based unlock flows
- local daemon session model and compartment switching
- shared API/client contract layer
- local snapshot, audit, transit, and Ethereum stealth flows
- provider-backed stealth deposit and sweep orchestration
- durable atomic persistence for vault and daemon state, including temp-file
  `fsync`, parent-directory sync where supported, backup restore, and corrupt-file quarantine
- startup recovery that reconciles pending operations, queue state, deposit state,
  and emits recovery telemetry for diagnostics
- bounded outbound HTTP behavior across the daemon, client, and webhook delivery paths
- embedded daemon UI actions routed through delegated handlers under a nonce-based CSP
- webhook delivery that re-resolves and pins HTTPS targets at send time to reduce SSRF drift
- `sigillum-gateway` as a local-sidecar payment preview surface
- BIP-39-backed 8-word default passphrase generation and RustCrypto TOTP HMACs

## What Is Not Yet Product-Complete

The main remaining limits are scope and assurance, not the current local-first baseline:

- CLI and broader operator ergonomics still lag the daemon/API surface in some
  areas, but the gap is smaller now that the daemon-backed CLI covers
  status/session operations plus provider profiles, stealth wallet profiles,
  deposits, queue inspection, and maintenance runs.
- The deployment story is intentionally local-only.
  - no polished remote service boundary
  - no remote event streaming or multi-host coordination model
  - no claim that the gateway is an internet-ready boundary because that is not the intended direction
- The current bar is based on code audit plus workspace verification gates.
  - no external penetration test
  - local adversarial/fuzz coverage is automated through
    `scripts/check-adversarial.sh`, but it is not an independent security audit
  - a 3600-second target-host daemon/gateway soak passed on `mac-server`, but
    broader host coverage and chaos testing are still future assurance work
- Broader operator polish still remains after the new daemon-backed CLI surface.
- The comprehensive wallet-management workstation remains a roadmap track.
  Current wallet and inventory slices are inside the local-first release
  boundary, but broader discovery, NFT/DeFi/airdrop, non-EVM, and automated
  consolidation claims are deferred.

## Structural Readiness Gates

Sigillum should only be treated as release-ready for a given scope when all of
these are true:

1. `./scripts/check-release.sh` passes from a clean checkout with the pinned
   Rust toolchain, committed daemon UI assets, adversarial/fuzz checks, and
   local daemon runtime smoke.
2. The API, daemon route, client surface, and docs all match.
3. The feature has an operator surface or an explicit API-only decision.
4. Persistence and restart behavior are explicit and tested.
5. The README, readiness doc, and roadmap describe the code that actually ships.

## Current Plan Of Record

The structural roadmap lives in
[docs/catchup-plan.md](./docs/catchup-plan.md).

The current execution order is:

1. Structural enforcement
  - CI
  - doc synchronization
2. Local operator-surface parity
3. Gateway correctness and local-sidecar parity
4. Automation and recovery hardening
5. `eth-xpub` project-wallet expansion
6. Remote/platform decision

## Short-Term Recommendation

The next work should still avoid speculative new product scope first. The right
immediate move is:

1. keep `./scripts/check-release.sh` enforced in CI across Ubuntu and macOS
2. keep `./scripts/check-adversarial.sh` green and expand it when new API,
   gateway, or UI boundary surfaces are added
3. expand higher-assurance testing beyond the current `mac-server` long soak,
   especially recovery, gateway delivery, browser/widget behavior, and chaos runs
4. close the remaining CLI/operator gaps for wallet/send flows and polish
5. keep documentation and audits anchored to the local-on-your-computer boundary
6. then begin `eth-xpub`
