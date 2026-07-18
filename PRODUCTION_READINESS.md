# Sigillum — Production Readiness

**Date:** June 4, 2026 (updated July 18, 2026)

**Current Verdict:** RC5 is a valid, checksum-verified draft candidate for
protected-main commit `7e047438f6305ef1cedecdf4790e1b0e1d7e1e6e`, not a
published or final release. Remote annotated tag object `c726ba9` peels to that
commit; release workflow `29248938476` passed all six jobs and produced the
draft's checksum plus five payload assets. Standard/chaos F4 and doctor
receipts bind to the same SHA. RC5 still lacks F6 public-testnet receipts,
clean-install desktop evidence, UI sign-off, and a complete sanitized evidence
bundle.

The operator-surface feature branch now changes the product after RC5. Its
committed keyboard/accessibility checkpoint is `29426df`, which contains
protected `origin/main` through merge `3b647f8`; interaction/browser checkpoint
`c435611` adds the green palette and has 225/225 UI tests, 15/15 axe scenarios,
12/12 screenshots, and a green real-daemon browser gate, plus focused coverage
for token-aware retirement and reconnection of authenticated SSE streams.
Current implementation checkpoint `8ea6f8e` restores the architecture,
formatting, and strict-clippy gates, and `7042178` is its documentation
successor. A warm-up full-gate process at `7042178` ended without a recoverable
output/exit receipt and is not a pass; a clean full gate remains pending at the
eventual documentation-truth commit. RC5 evidence is therefore historical
baseline for this feature line; after protected-main integration the next
eligible candidate is RC6 and all release evidence must bind to RC6's exact
peeled SHA. There is no final `v1.0.0` tag and no published GitHub Release.
Earlier RC2–RC4 tags remain immutable failed-contract receipts. The supported
boundary remains local-first, single-host, and not internet-facing.

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
- RC5 historical evidence is internally consistent: workflow
  `29248938476` passed, its five payload assets match `SHA256SUMS`, and
  standard/chaos F4 plus doctor receipts name `7e04743`. It does not certify
  product changes after that SHA.
- The remaining same-candidate evidence for RC6 is:
  - a clean-tree `./scripts/check-release.sh`, protected-main CI, successful
    draft release workflow, and independently checksum-verified assets at the
    exact RC6 SHA
  - standard and chaos soak receipts plus doctor on every supported host at
    the exact RC6 SHA (F4)
  - five public-testnet transactions for the four core execution families:
    native sweep, ERC-20 sweep, revoke, plus both the `fund_gas` and dependent
    sweep legs of gas top-up on Ethereum Sepolia (`11155111`) and Base Sepolia
    (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`) (F6)
  - a checksum-verified `.dmg` clean install reaching unlock without a
    developer toolchain, plus operator UI walkthrough/sign-off
  - one sanitized external evidence bundle containing the same-RC operator
    receipts; H2 binds its filename and SHA-256 digest into the immutable final
    tag, verifies the uploaded copy before publication, and H3 records the
    public linkage in the tracked audit
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

The active execution plan for the 1.0 release lives in
[docs/release-1.0-plan.md](./docs/release-1.0-plan.md). Its phases (A-H, W1-W8,
F, G, H) are the plan of record. The active feature companion and exact
continuation state are
[docs/operator-surface-and-privacy-plan.md](./docs/operator-surface-and-privacy-plan.md)
and [docs/execution-handoff.md](./docs/execution-handoff.md). The earlier structural roadmap in
[docs/catchup-plan.md](./docs/catchup-plan.md) remains a background reference;
its phases 1–3 are absorbed into the 1.0 plan.

## Short-Term Recommendation

The next work should still avoid speculative new product scope first. The right
immediate move is:

1. commit the current documentation-truth correction on top of documentation
   checkpoint `7042178` and implementation checkpoint `8ea6f8e`
2. at that resulting commit, make the next local validation step a clean-tree
   `./scripts/check-release.sh` run; the unrecoverable `7042178` warm-up and the
   green focused checks do not substitute for the complete gate
3. merge through protected `main` with required Ubuntu and macOS contexts green
4. create immutable annotated RC6 and independently verify its draft assets
5. collect RC6-bound doctor, standard/chaos soak, public-testnet F6,
   clean-install desktop, and UI sign-off evidence
6. build and validate the external evidence bundle; only then request the H2
   final-tag/publish decision
7. keep non-EVM chains, swap execution, fiat/NFT valuation, and remote or hosted
   modes in their documented post-1.0 scope
