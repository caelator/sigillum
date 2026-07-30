# Sigillum — Production Readiness

**Date:** June 4, 2026 (updated July 30, 2026)

**Current Verdict:** Sigillum is not production-ready and has no published
stable release. `v1.0.0-rc.2` through `v1.0.0-rc.4` are immutable failure
receipts. `v1.0.0-rc.5` at protected-main commit `7e04743` is an unpublished
historical draft only: its successful workflow and checksums prove that older
commit, not the integrated hardening line. No RC6 exists.

The next eligible candidate is `v1.0.0-rc.6` only after the exact integrated
HEAD passes `./scripts/check-release.sh` from a clean tree, completes the
required independent review, lands through protected `main`, and passes
required CI. C7 remains partial: the five-destination operator console is
implemented, but its RC6-bound operator walkthrough and sign-off are pending.
All release evidence must bind to the same RC6 peeled SHA. The fail-closed
target-host evidence boundary is macOS 15.x on Apple Silicon (`aarch64`) only;
macOS 26.5.2/arm64 is not an eligible release-evidence host. There is no final
`v1.0.0` tag and no published GitHub Release.

## Summary

Historical source receipts re-proved the full-workspace baseline for their
exact commits. The integrated hardening HEAD must re-prove that baseline across
the local daemon, client, core, CLI, the `sigillum-gateway` sidecar, and the
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

The current hardening checkout implements the following single-machine
capabilities, subject to the fresh source, CI, RC, and operator gates below:

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
- ScopeLift-compatible ERC-5564 scheme-1 shared-point hashing with legacy
  pre-release payment recovery
- post-wait session/compartment revalidation and lock-latch admission before
  transaction broadcast
- serialized first-run admission rechecks for snapshot restore, compartment
  initialization, and FIDO2 setup
- forced idle locking that keeps broadcast admission closed and re-zeroizes
  after already-admitted work drains
- lock-latch queue holds that stop the drain and preserve exact-byte recovery
  authority without terminalizing an unsubmitted or ambiguous job
- cancelable, resumable discovery jobs with terminal failure and restart states
- cross-process FIDO2 writer exclusion and causal recovery receipts bound to
  the exact resulting configuration state

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
- RC5 is retained as historical evidence for `7e04743`; none of its workflow,
  artifact, soak, doctor, or UI evidence certifies the integrated hardening
  changes.
- The remaining same-candidate evidence for RC6 is:
  - an exact-integrated-HEAD clean-tree `./scripts/check-release.sh`,
    independent review, protected merge, required CI, successful RC6 draft
    workflow, and independently checksum-verified assets at the exact RC6 SHA
  - schema-v2 standard 3600-second and chaos 600-second soak receipts plus
    `sigillum doctor` on the same eligible macOS 15.x/aarch64 host at the exact
    RC6 SHA (F4)
  - five public-testnet transactions for the four core execution families:
    native sweep, ERC-20 sweep, revoke, plus both the `fund_gas` and dependent
    sweep legs of gas top-up on Ethereum Sepolia (`11155111`) and Base Sepolia
    (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`) (F6)
  - F7 upgrade-path verification at RC6
  - a checksum-verified RC6 `.dmg` clean install reaching unlock without a
    developer toolchain, plus the RC6 five-destination operator UI
    walkthrough/sign-off (C7)
  - one sanitized external evidence bundle containing the same-RC operator
    receipts; H2 binds its filename and SHA-256 digest into the immutable final
    tag, verifies the uploaded copy before publication, and H3 records the
    public linkage in the tracked audit
  - an explicit H2 operator decision, recorded immediately before invoking the
    final-tag ceremony, that authorizes tag creation and conditional
    publication only if every post-tag verification passes
  - exact-byte final promotion: rerun the source-verification legs, skip
    artifact rebuilds, copy the five qualified RC payload bytes under final
    names, verify byte identity and tag-normalized digests, regenerate
    `SHA256SUMS`, attach the evidence bundle as the seventh asset, and verify
    the final draft before publication
- RC releases must be draft, unpublished, and `prerelease=true`; the final
  draft and published release must be `prerelease=false`.
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
   runtime smoke, pinned 15-scenario axe-core accessibility coverage, and no
   tracked-tree mutation. Mock accessibility/screenshot evidence does not
   replace the real-daemon browser smoke.
2. The API, daemon route, client surface, and docs all match.
3. The feature has an operator surface or an explicit API-only decision.
4. Persistence and restart behavior are explicit and tested.
5. The README, readiness doc, and roadmap describe the code that actually ships.

Gate 3 (operator surface or explicit API-only decision) is tracked
route-by-route in
[docs/operator-surface-parity.md](./docs/operator-surface-parity.md).

## Current Plan Of Record

The current release authorities are
[docs/release-1.0-plan.md](./docs/release-1.0-plan.md) and
[docs/execution-runbook-1.0.md](./docs/execution-runbook-1.0.md). The plan
defines scope and acceptance; the runbook defines current execution order,
release mechanics, and operator gates. Historical plans and handoffs are not
current authority.

## Short-Term Recommendation

The next work should still avoid speculative new product scope first. The right
immediate move is:

1. run the full clean gate and independent review against the exact integrated
   HEAD
2. merge through protected `main` with required Ubuntu and macOS CI green
3. create immutable annotated RC6; require its GitHub Release to remain an
   unpublished prerelease draft and independently verify all six assets
4. collect RC6-bound F7, schema-v2 F4/doctor evidence on macOS 15.x/aarch64,
   five funded F6 transactions, clean-machine install/unlock, and C7 sign-off
5. validate the complete evidence bundle and exact-byte final draft; only then
   request the explicit H2 final-tag/publish decision
6. keep non-EVM chains, swap execution, fiat/NFT valuation, and remote or hosted
   modes in their documented post-1.0 scope
