# Execution Handoff — Operator Surface & Privacy Plan

> [!CAUTION]
> Retired historical record. This handoff was superseded by the integrated
> hardening work and must not be used as current implementation or release
> authority. The only current release authorities are
> [`release-1.0-plan.md`](./release-1.0-plan.md) and
> [`execution-runbook-1.0.md`](./execution-runbook-1.0.md). RC6 now exists only
> as immutable failed-workflow evidence; no production-ready release exists.

**Checkpoint date:** 2026-07-18

**Current implementation checkpoint:**
`8ea6f8eb66c044f94ba5c1ad76ae4477ae03be63` (`8ea6f8e`)

**Audited documentation/release-gate checkpoint:**
`fc1e93b1aa2cf9524b2b99fd04342863ba6b2b1d` (`fc1e93b`)

**Latest failed exact-HEAD gate checkpoint:**
`8c654fc854bbc03696a51fc8dd8f8342765211af` (`8c654fc`)

**Historical accessibility checkpoint:** `29426df`
(`Harden keyboard and accessibility gates`)

**Branch:** `codex/operator-surface-privacy`

**Protected-main baseline:** `7e047438f6305ef1cedecdf4790e1b0e1d7e1e6e`
(`origin/main`, merged into this branch by `3b647f8`)

**Historical companion:** `operator-surface-and-privacy-plan.md`

This handoff was the zero-context continuation point for its dated branch. It
is preserved for provenance only and is not executable current guidance.

**Current implementation truth:** `c435611` commits the command palette,
regenerated bundles, real-daemon browser-smoke migration, and reviewed
session/focus race hardening. `8ea6f8e` then repairs the architecture and
formatting debt exposed by the first clean release-gate attempt, without
relaxing any existing parent-file cap. It splits the affected Rust and authored
CSS owners, restores strict clippy, regenerates the embedded stylesheet, and
updates the architecture contract to the live UI core/destination layout. Grok
4.5 reported `CONVERGED: YES` for both the plan and final implementation.
`fc1e93b` synchronizes the current documentation and passes the complete
clean-tree release gate in 919 seconds with every recorded return code zero.
That receipt proves `fc1e93b` only. The exact-HEAD rerun at documentation
successor `8c654fc` preserved a real failure in the scheduler integration test:
an unlocked one-second tick could legitimately cross its final pause check
after the test read a queued job but before the pause handler set the in-memory
latch. Production pause/drain/broadcast code is byte-identical across the two
commits; the current handoff fixes only the test fixture by establishing pause
before scheduler startup. This HEAD still requires an external clean-gate
receipt before protected-main integration and RC6.

## 1. Mission and non-negotiable boundaries

Finish the operator-surface and privacy plan without weakening Sigillum's
fail-closed authorization, execution, payment, evidence, or release contracts.
The branch now contains the privacy/backend work and the five-destination
operator console, including the interaction and browser-smoke work through
`c435611` and the release-gate architecture repair through `8ea6f8e`.
`fc1e93b` records that repair and passes the complete local source gate.
The current test-only successor repairs the scheduler fixture race exposed at
`8c654fc`. Remaining work is this HEAD's external exact-HEAD gate receipt,
operator visual sign-off, protected-main integration/CI, and same-SHA RC6
evidence.

- Work only in the designated worktree and branch. Do not push, tag, publish,
  or change GitHub settings unless the operator explicitly authorizes it.
- `crates/sigillum-daemon/ui/src/app.js` and `src/styles.css` are checked-in
  build artifacts. Any UI source change requires `npm run build` and a review
  of the regenerated bundles.
- Never convert mock, local, historical, or different-SHA evidence into a
  release claim. Preserve failed receipts.
- Run release gates only from a clean checkout and do not overlap them with
  another agent changing or building the same tree.
- RC5 is historical evidence for protected `main` at `7e04743`; this feature
  branch changes the product after that SHA. Its next eligible candidate is
  RC6 after protected-main merge.

## 2. Implemented state — do not redo

Phases 0–3 are complete. Phase 4 has the five-controller console, hardened
state reconciliation, receiving parity, modal semantics, and keyboard and
accessibility foundations.

### Backend and privacy

- Structured error codes and field validation; real asynchronous operations
  with cooperative discovery cancellation/resume; passive SSE and status reads;
  pagination/filter/sort; a guarded background scheduler; canonical
  `/api/chains*` routes.
- Standards-compatible ERC-5564 compressed-point hashing with dual decode for
  pre-switch records, external vectors, watch-only detection, persisted scan
  cursors, single-key meta-addresses, and payer-gas metadata.
- Provider-partitioned HD scanning, forget/prune cascades, xpub exposure gates,
  default-on cross-party linkage blocking, common-funder findings, and the
  scheduler-driven one-time receive lifecycle.
- Explicit gas-top-up caps. Enabling gas top-ups without a nonblank valid
  `max_gas_topup_wei_hex` fails validation; missing or corrupt stored caps keep
  runtime gas-topup policy disabled.
- Receiving balance identity is `(wallet_family, wallet_profile, chain_id,
  address)`, not address alone. Overview selects the freshest matching
  observation deterministically. `ReceivingItem` exposes additive optional
  `balance_last_checked_at_unix`.
- Counterparty destination updates use patch semantics: omitted retains the
  stored destination; explicit blank clears it.

### Operator console

- Core composition in `ui/src/core/`: observable store, keyed DOM rendering,
  hash router and legacy adapter, typed API methods plus temporary thin local
  request wrappers, SSE reconciliation, keyboard helpers, and live runtime.
- Five destination controllers in `ui/src/destinations/`: Overview, Move,
  Receiving, Portfolio, and Vault. Controllers take over designated legacy
  hosts, subscribe to live state, and restore stashed content on unmount.
- All five destinations consume relevant SSE-backed status, operation, queue,
  sync, or resync state. Snapshot data is authoritative for live operations;
  bounded list requests enrich terminal history. Generation/revision guards
  reject stale completions, and passive polling is the fallback.
- Session-token revoke/rotation retires the authenticated SSE generation and
  reconnects only with the current authorization. A revoked browser token
  cannot leave its previously opened event stream alive.
- Each session-aware request captures the bearer token it actually sent. A
  late `401` clears browser authorization only if that token is still current,
  so an older response cannot revoke a newly reauthenticated session. Same-tab
  token clear applies the locked shell and disables palette eligibility
  synchronously instead of waiting for the five-second refresh loop.
- Receiving owns four-source reconciliation, party edit/delete, allocations,
  deposits, balance refresh, and safe optimistic tag updates. Failed writes
  roll back; a successful write followed by failed refresh retains the committed
  value and reports degraded freshness.
- The shared modal coordinator enforces one active modal, DOM-order focus
  trapping, Escape/backdrop cancellation, and connected-only focus restore.
  Existing and dynamically appended background siblings are inert while a
  modal is active, and escaped programmatic focus is redirected into the
  dialog. FIDO removal distinguishes cancellation from an explicitly submitted
  blank value; cancel/Escape/backdrop issue zero mutation requests.
- Keyed workspace and compartment navigation preserves focused nodes. Locked
  autofocus occurs once per transition and yields to modal/operator focus.
  Delayed FIDO detection is mode-guarded so a response started in locked mode
  cannot re-show unlock controls after authentication. Setup and
  reauthentication Enter behavior, semantic landmarks, headings, file labels,
  lists, and tables are covered by tests.
- At `c435611`, the command palette has an exact seven-command allowlist:
  navigation to the five destinations, workspace refresh, and self-check. It
  is unlocked-only, refuses to replace another modal, closes before command
  execution/error reporting, and fails closed if the workspace locks while it
  is open. Its dialog/combobox/listbox semantics, filtering, wrapped keyboard
  navigation, dismissal, and focus restoration have focused tests.
- The screenshot harness is strict and stateful: allowlisted routes return real
  envelopes, unknown routes fail the run, and server contract tests cover the
  mock boundary.

### Release-gate architecture repair

- The first clean `./scripts/check-release.sh` attempt at `2ecb3f7` preserved a
  real failure: `check-architecture.sh` stopped on `audit_log.rs` at 1909 lines
  against its 1800-line cap. Once that first breach was repaired, the guard
  exposed additional feature-branch growth in daemon, API, CLI, client, and
  authored CSS files. No existing parent cap was raised.
- `8ea6f8e` assigns that growth to explicit child owners while keeping public
  Rust paths and JSON contracts stable. Queue replay/gates/outcome/broadcast
  order remains in the processing loop; only fresh payload dispatch moved.
  Inventory cancellation/checkpoint/finalization and AppState lock semantics
  remain byte-equivalent inherent methods in child modules.
- The architecture script now requires and budgets every new Rust/CSS owner,
  enforces the complete authored stylesheet import order, and points at the
  live `ui/src/core` and destination controllers instead of three dead files
  removed by `cd4cf72`.
- Workspace formatting drift and strict-clippy findings were repaired rather
  than excluded from the release gate. The only lint allowance is scoped to
  the Axum list-query boundary and documents why returning the final structured
  `Response` is intentional.

### Commit sequence after the earlier console checkpoint

- `a83302d` — hardened screenshot-harness contracts
- `7195818`, `9244e12` — receiving observation and refresh identity
- `0a4e88d` — explicit gas-top-up cap requirement
- `07d074b`, `a9a3c5e` — operator state, policy, and risk-contract safety
- `1f685c7` — receiving operator parity and per-item freshness
- `3b647f8` — merged protected `origin/main` at `7e04743`
- `8034eb6` — modal and FIDO cancellation hardening
- `29426df` — keyboard, focus, semantic accessibility, and pinned axe gate
- `c435611` — completed command palette, browser-smoke migration, session/SSE
  race corrections, modal/FIDO focus guards, and regenerated bundles; focused
  verification is green
- `2ecb3f7` — refreshed this execution handoff before the first clean full-gate
  attempt; that attempt failed at the architecture cap and is not a pass
- `8ea6f8e` — restored architecture/format/clippy gates, split the over-cap
  owners, regenerated the stylesheet bundle, and passed focused verification
- `7042178` — documented the architecture repair and focused verification
- `fc1e93b` — synchronized release checkpoint documentation and passed the
  complete clean-tree release gate with an auditable external receipt
- `8c654fc` — recorded the passing predecessor receipt; its exact-HEAD gate
  failed in the scheduler integration test and is preserved as negative
  evidence

## 3. Verification by checkpoint

Verified from the clean `29426df` tree:

- `cd crates/sigillum-daemon/ui && npm test`: **215/215 passed**.
- `./scripts/check-ui-accessibility.sh`: **14/14 scenarios passed** with
  axe-core `4.12.1`, **0 violations and 0 incomplete results**.
- Twelve populated mock-data screenshots exist across the standard and
  1024-pixel walkthrough sets.
- Route inventory: **142 route registrations and 143 method endpoints**;
  `/api/treasury/parties` is the sole GET+POST registration.

Verified at implementation checkpoint `c435611`:

- UI tests: **225/225 passed**; TypeScript typecheck and Vite build passed.
- `./scripts/check-ui-accessibility.sh`: **15/15 scenarios passed** with
  axe-core `4.12.1`, **0 violations and 0 incomplete results**.
- Screenshot walkthrough: **12/12 passed**.
- `./scripts/check-browser-smoke.sh`: passed end to end against an isolated
  real daemon. Its cold-build startup timeout now defaults to 120 seconds and
  is configurable.

Verified at implementation checkpoint `8ea6f8e`:

- `./scripts/check-architecture.sh`, `git diff --check`, and
  `cargo fmt --all -- --check`: passed.
- `cargo check --workspace --all-targets --locked`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test -p sigillum-api -p sigillum-client -p sigillum-cli
  -p sigillum-daemon --all-targets --locked`: passed, including API/client/CLI
  contracts, **495 daemon unit tests**, and every daemon integration target.
- UI tests: **225/225 passed**; TypeScript typecheck and Vite build passed; the
  checked-in `styles.css` bundle was regenerated.
- Screenshot-server contracts: **5/5 passed**; accessibility: **15/15** with
  axe-core `4.12.1`, **0 violations and 0 incomplete results**; screenshot
  walkthrough: **12/12**; isolated real-daemon browser smoke: passed.
- GitNexus impact review covered the CRITICAL AppState callers and HIGH scan/
  queue surfaces; the index was refreshed at `8ea6f8e`. Cursor CLI Grok 4.5
  final implementation review reported `CONVERGED: YES` with no stop-ship
  finding.

The warm-up `./scripts/check-release.sh` process at documentation checkpoint
`7042178` reached `cargo deny`, but its output and exit receipt were not
recoverable. It is intentionally preserved as non-evidence and does not prove
the complete gate passed.

The auditable clean gate at exact SHA
`fc1e93b1aa2cf9524b2b99fd04342863ba6b2b1d` started at
`2026-07-18T17:15:16Z`, ended at `2026-07-18T17:30:35Z`, and ran for 919 seconds.
`gate_rc=0`, `tee_rc=0`, `filter_rc=0`, and `overall_rc=0`. The operator-local
external receipt ID is `20260718T171516Z-fc1e93b1aa2c`; the log SHA-256 is
`efb8e2240949d32f0f53ff8ee028d1016724df849e038b361b9fed78f23dab94`.

The exact-HEAD clean gate at `8c654fc854bbc03696a51fc8dd8f8342765211af`
started at `2026-07-18T17:36:36Z` and failed after 532 seconds in
`scheduler_skips_everything_when_locked_and_when_execution_paused`:
`gate_rc=101`, `tee_rc=0`, `filter_rc=0`, and `overall_rc=101`. The
operator-local external receipt ID is `20260718T173636Z-8c654fc854bb`; the log
SHA-256 is
`7544419aaa610e91a6bd45ad93cdf5cc416c061a3d8df0427110d6f03aa46b61`.
The job advanced from `queued` to `sent`; the scheduler had crossed its final
pause check before the handler set the in-memory latch. Ten subsequent pre-fix
whole-binary runs passed, confirming a timing-sensitive fixture race rather
than a production-code regression.

The current test-only repair starts the daemon without the scheduler, enqueues
and pauses through the real HTTP API, locks, then starts the scheduler. It
proves `skipped_locked`, advances unlocked paused ticks with zero job attempts,
and broadcasts exactly once only after the real resume endpoint. Cursor CLI
Grok 4.5 High reported `CONVERGED: YES`. Focused evidence is green: scheduler
**3/3**, scheduler-disabled **1/1**, the existing mid-drain pause regression
**1/1**, and **20/20** repeated exact repaired cases. Production scheduler,
pause, drain, broadcast, and persistence code is unchanged.

The focused checks at `8ea6f8e` are source-slice evidence; the auditable
`fc1e93b` run proves the complete local source gate. Neither proves the
same-candidate operator/release receipts below:

- RC6-bound standard/chaos F4, doctor, or F7 upgrade-path rerun evidence;
- clean-install desktop behavior or operator visual sign-off;
- public-testnet F6 execution receipts;
- a same-candidate sanitized release-evidence bundle.

## 4. Exact continuation order

1. Preserve both the passing `fc1e93b` receipt and failed `8c654fc` receipt.
   Run the clean gate at this test-fix HEAD and retain the external exact-SHA
   receipt before protected-main integration; do not edit the commit solely to
   embed its own receipt.
2. Review the 12 screenshots manually; automated mock rendering is not
   operator visual sign-off or runtime proof.
3. Review and merge through protected `main`, with required Ubuntu and macOS CI
   contexts green. Re-index GitNexus if implementation changes after
   `8ea6f8e`.
4. Create the next immutable annotated candidate, `v1.0.0-rc.6`, only from the
   protected-main commit. Verify the six-job draft release and asset checksums.
5. Bind F4 standard/chaos, F6 public-testnet receipts, F7 upgrade-path tests,
   desktop clean-install, doctor, and UI sign-off to the exact RC6 peeled SHA.
   Build and validate the external sanitized evidence archive.
6. Only after every H1 receipt agrees may the operator make the explicit H2
   final-tag/publish decision. No final `v1.0.0` tag or published release exists
   at this handoff.

## 5. Known product and proof gaps

- Compartment add is available in Vault; compartment remove remains API-only
  pending a destructive typed-confirm UI.
- No per-job queue-cancel endpoint exists; operation-level cancellation is the
  current control.
- Vault idle countdown is explicitly a browser-tab estimate because the daemon
  does not expose last-activity time.
- Snapshot restore file handling belongs in the real browser smoke because the
  fake DOM does not implement browser `File` behavior.
- The core API is not yet literally the only request path: controllers retain
  thin, session-aware wrappers for methods not yet promoted into `core/api.ts`.
- Mock screenshots and axe runs prove the checked-in frontend renders against
  strict mock envelopes. They do not prove daemon authentication, RPC/provider
  behavior, signing, broadcast, persistence, or release packaging.

## 6. Release truth and operator gates

Remote `v1.0.0-rc.5` is an annotated tag object
`c726ba913ace7f5ca64987454b1352ffdd9c8f77`, peeled to protected-main commit
`7e047438f6305ef1cedecdf4790e1b0e1d7e1e6e`. GitHub Actions run
`29248938476` passed all six jobs. Its GitHub Release is still an unpublished
draft with six assets; the five payload assets independently match
`SHA256SUMS`.

RC5 standard F4, chaos F4, and doctor receipts bind correctly to `7e04743`.
They remain useful historical evidence, but they cannot certify code added on
this branch. RC5 has no qualifying F6 receipt set, desktop clean-install
receipt, UI sign-off, or complete external evidence bundle. There is no final
`v1.0.0` tag and no published GitHub Release.

Branch protection was observed on 2026-07-18 with strict required contexts
`rust (ubuntu-24.04)` and `rust (macos-15)`, admin enforcement, and force-push
and deletion disabled. Final and RC tag rulesets were also observed active.
Those settings are mutable external state and must be rechecked before release.

## 7. Verification commands

```bash
cd crates/sigillum-daemon/ui
npm ci --ignore-scripts
npm test
npm run typecheck
npm run build

cd ../../..
node scripts/ui-screenshots/server.test.mjs
node scripts/ui-screenshots/drive.mjs
./scripts/check-ui-accessibility.sh
./scripts/check-browser-smoke.sh
./scripts/check-release.sh
```

Run the full release gate only from a clean, stable checkout. The evidence store
is outside the repository at
`/Users/xx/Documents/ReleaseEvidence/Sigillum/`; never commit its binary or
sensitive contents.
