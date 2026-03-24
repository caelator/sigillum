# Sigillum — Production Readiness

**Date:** March 24, 2026  
**Current Verdict:** The documented local-first single-host baseline is green; internet-facing and remote-platform scope is not the target

## Summary

Sigillum now meets its current full-workspace release baseline for the
documented local-first scope. That baseline covers the local daemon, client,
core, CLI, and the `sigillum-gateway` sidecar:

- the workspace needs to stay green on build, tests, fmt, clippy, and audit
- the current 2026-03-24 verification pass completed cleanly across those workspace gates
- the local daemon uses bearer session-token auth over local HTTP
- the gateway is part of the workspace release bar, but remains a local-sidecar
  preview surface in this phase
- the passphrase unlock path, daemon state recovery, and gateway/payment flows
  are all part of the same readiness story

That does **not** mean the project is aiming toward a remote, multi-tenant, or
internet-facing deployment. The green bar here is explicitly the single-machine
operator model described in the README and deployment guide, because that is the
intended product boundary.

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
  - no fuzzing campaign across the gateway/UI boundary
  - no long-duration soak or chaos testing evidence yet
- Broader operator polish still remains after the new daemon-backed CLI surface.

## Structural Readiness Gates

Sigillum should only be treated as release-ready for a given scope when all of
these are true:

1. The workspace is green on build, tests, fmt, clippy, and audit.
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

1. keep the full-workspace release gate enforced in CI
2. expand higher-assurance testing around long-running recovery, gateway delivery, and browser/widget behavior
3. close the remaining CLI/operator gaps for wallet/send flows and polish
4. keep documentation and audits anchored to the local-on-your-computer boundary
5. then begin `eth-xpub`
