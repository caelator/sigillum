# Sigillum — Production Readiness

**Date:** June 4, 2026 (updated July 10, 2026)
**Current Verdict:** the earlier Sigillum 1.0 RC receipts are historical and do
not certify the current hardening line. Capability authorization, payment
truth, queue broadcast durability/pause behavior, no-HID compilation, and
release governance must pass the updated source gate together before a new RC.
The supported boundary remains local-first, single-host, and not
internet-facing; remote-platform scope is explicitly unsupported.

## Summary

Sigillum's earlier RC met the full-workspace baseline for the documented
local-first scope. The current hardening checkout must re-prove that baseline
for the local daemon, client, core, CLI, the `sigillum-gateway` sidecar, and the
shipped EVM wallet-management product:

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
  preview surface in this phase; payment creation is disabled by default and
  balance observations are not supported payment confirmations
- the passphrase unlock path, daemon state recovery, and gateway/payment flows
  are all part of the same readiness story
- `sigillum doctor` is the operator preflight for local host readiness
- EVM roadmap phases 1–9 ship multi-chain discovery, inventory, and risk
  analysis; the chain registry; consolidation planning; DeFi exit adapters;
  gas top-ups; and hot-wallet overflow/refill treasury automation
- consolidation-plan execution ships as a policy-gated, fail-closed opt-in
  that defaults off

The current evidence and remaining caveats are tracked in
[docs/production-readiness-audit.md](./docs/production-readiness-audit.md).

That does **not** mean the project is aiming toward a remote, multi-tenant, or
internet-facing deployment. The green bar here is explicitly the single-machine
operator model described in the README and deployment guide, because that is the
intended product boundary.

The release scope is **Sigillum 1.0**, the local-first wallet-management
workstation. It includes the local daemon, vault, CLI, session model,
local-sidecar gateway preview, and the EVM wallet-management product described
in [docs/wallet-management-roadmap.md](./docs/wallet-management-roadmap.md)
phases 1–9. Wallet management is complete for EVM except swap execution, which
is deferred per D-13. Consolidation-plan execution is in scope and ships as a
policy-gated, fail-closed opt-in that defaults off. Non-EVM chains (roadmap
phase 10), fiat/NFT valuation (D-16), and remote or hosted operation remain
future work; the local-first, single-host, not-internet-facing boundary is
unchanged.

Gateway payment creation requires the explicit
`GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS=1` opt-in. It emits observations rather
than finality-backed confirmations, exposes the latest balance observation
timestamp, and has no privileged third-party invoice-signing callback.

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
- crash-safe queue submission: every family durably records exact signed bytes
  and hash as `prepared`, persists `submitted_unknown` before RPC, and recovers
  by receipt lookup or exact-byte resubmission without re-signing
- bounded outbound HTTP behavior across the daemon, client, and webhook delivery paths
- embedded daemon UI actions routed through delegated handlers under a nonce-based CSP
- webhook delivery that re-resolves and pins HTTPS targets at send time to reduce SSRF drift
- `sigillum-gateway` as a disabled-by-default local-sidecar payment observation
  preview, not a supported 1.0 payment processor
- BIP-39-backed 8-word default passphrase generation and RustCrypto TOTP HMACs

## What Is Not Yet Product-Complete

The main remaining limits are scope and release-candidate assurance evidence,
not the shipped local-first wallet-management baseline:

- Deployment remains intentionally local-only. There is no supported remote
  service boundary, remote event streaming, multi-host coordination model, or
  claim that the gateway is an internet-ready boundary.
- D-4 defines the assurance claim as the source-verified local-first release
  gate. No external penetration test has been performed, and the release does
  not claim one. Automated local adversarial/fuzz coverage through
  `scripts/check-adversarial.sh` is not an independent security audit.
- The remaining RC-time evidence is:
  - standard and chaos doctor/soak receipts on every supported host at the new
    RC SHA; the earlier `mac-server` receipt is historical baseline evidence,
    not evidence for the current hardening candidate (F4)
  - public-testnet execution receipts for the four core execution families:
    native sweep, ERC-20 sweep, revoke, and gas top-up (F6)
- Within wallet management, only non-EVM chains (roadmap phase 10), swap
  execution (D-13), and fiat/NFT valuation (D-16) are deferred.

### Execution-path residual risk (D-17)

With allow_plan_execution and the relevant per-family execution gates enabled,
a stolen session token on the local machine can move funds. The shipped
mitigations (typed confirmation at enqueue, per-family fail-closed policy gates,
gate-flip audit events carrying session fingerprints, the `execution_paused`
kill switch with a lock-free pre-broadcast latch, and policy re-reads at both
enqueue and queue-drain time) detect and
bound that misuse; they do not prevent it. FIDO2 tap-to-execute is the named
post-1.0 hardening candidate. Operators who do not accept this risk leave the
execution gates off (their default).

The execution-path security review (F5) dispositions are complete and recorded
in the audit's
[Execution-path security review (F5)](./docs/production-readiness-audit.md#execution-path-security-review-f5),
including the N1 cross-plan-linkage residual and the F1 policy-cap hardening.

## Structural Readiness Gates

Sigillum should only be treated as release-ready for a given scope when all of
these are true:

1. `./scripts/check-release.sh` passes from a clean checkout with the pinned
   Rust toolchain, committed daemon UI assets, locked dependency resolution,
   default and no-HID FIDO2 coverage, adversarial/fuzz checks, local daemon
   runtime smoke, and no tracked-tree mutation.
2. The API, daemon route, client surface, and docs all match.
3. The feature has an operator surface or an explicit API-only decision.
4. Persistence and restart behavior are explicit and tested.
5. The README, readiness doc, and roadmap describe the code that actually ships.

Gate 3 (operator surface or explicit API-only decision) is tracked
route-by-route in
[docs/operator-surface-parity.md](./docs/operator-surface-parity.md).

## Current Plan Of Record

The active execution plan for the 1.0 release lives in
[docs/release-1.0-plan.md](./docs/release-1.0-plan.md). Its phases (A-H, W1-W8,
F, G, H) are the plan of record. The earlier structural roadmap in
[docs/catchup-plan.md](./docs/catchup-plan.md) remains a background reference;
its phases 1–3 are absorbed into the 1.0 plan.

## Short-Term Recommendation

The next work should still avoid speculative new product scope first. The right
immediate move is:

1. keep `./scripts/check-release.sh` enforced in CI across Ubuntu and macOS
2. keep `./scripts/check-adversarial.sh` green and expand it when new API,
   gateway, or UI boundary surfaces are added
3. collect the remaining per-host doctor/soak receipts beyond `mac-server` (F4)
4. collect public-testnet execution receipts for native sweep, ERC-20 sweep,
   revoke, and gas top-up (F6)
5. keep documentation and audits anchored to the local-on-your-computer boundary
6. keep non-EVM chains, swap execution, fiat/NFT valuation, and remote or hosted
   modes in their documented post-1.0 scope
