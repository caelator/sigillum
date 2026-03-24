# Catch-Up Plan

## Goal

Bring the full Sigillum workspace, including `sigillum-gateway`, up to one
consistent level of maturity.

For this plan, a feature is only considered **fully developed** when it has:

- a green workspace build and test baseline
- stable storage and migration behavior
- transport coverage in `sigillum-api`, daemon routes, and client methods
- a supported operator surface: UI, CLI, or an explicit "API-only" decision
- validation, auth, audit, and recovery behavior
- docs that match the code

This is a parity plan for the existing local-first system first. Remote and
multi-host work is deliberately sequenced after the local platform is coherent.

## Current Baseline

Status after this catch-up pass:

- The workspace is back to a fully green release baseline as of 2026-03-24.
- `sigillum-core`, `sigillum-daemon`, `sigillum-client`, `sigillum-cli`, and
  `sigillum-gateway` all remain in the production-readiness scope.
- The non-vault JSON stores for profiles, deposits, and queue state use atomic
  writes, sidecar backups, and automatic restore/quarantine behavior for
  missing or corrupted live files.
- The next release gate is keeping build, test, fmt, clippy, and audit aligned
  across the whole workspace.

The remaining maturity mismatches are now product-surface, CLI, gateway, and
longer-running recovery gaps, not a broken baseline build:

- The daemon/API/client surface is now much closer to the embedded UI, but CLI
  parity is still catching up for deeper wallet/send flows and broader operator
  polish.
- The gateway is part of the workspace release bar, but it should stay a
  local-sidecar preview surface until a separate remote boundary is designed.
- Queue-policy recovery and maintenance observability still need a fuller Phase
  2 pass.
- Repo-wide formatting and linting must be enforced in CI alongside tests and
  audit.

The documented product gaps are also clear:

- The architecture is intentionally local-first and does not yet provide a
  polished remote client/server story, remote event streaming, remote audit
  aggregation, or deep on-chain indexing.

## Development Standard

All backlog items below should be executed to the same finish line:

1. design and schema are explicit
2. daemon service behavior is implemented
3. client and SDK parity is present
4. persistence is durable and backward-compatible
5. tests cover happy path, auth failure, validation failure, and recovery
6. docs and operator-facing screens are updated in the same phase

## Structural Workstreams

The catch-up plan is now organized around five structural workstreams:

- `W0 Baseline Enforcement`
  - workspace build, tests, clippy, audit, CI, formatting discipline, readiness docs
- `W1 Operator Surface Parity`
  - embedded UI and CLI parity for all supported local workflows
- `W2 Contract And Persistence Parity`
  - request/response coverage, client/SDK parity, profile/deposit/queue durability, gateway local-sidecar parity
- `W3 Automation And Recovery`
  - queue semantics, restart safety, maintenance summaries, destructive-flow recovery
- `W4 Product Expansion`
  - `eth-xpub` project wallets first, remote/platform work only after the local system is coherent

These workstreams are intentionally ordered. New wallet families or remote
specialization should not move ahead of unfinished local parity and recovery
work.

## Structural Gates

Every phase and workstream must pass these gates before the next one is opened:

1. `Build Gate`
   - `cargo test --workspace` is green
   - `cargo fmt --all --check` is green
   - `cargo clippy --workspace --all-targets` is green
   - `cargo audit` is green
2. `Contract Gate`
   - daemon route, API type, client surface, and docs all exist together
3. `Operator Gate`
   - there is either a UI/CLI surface or an explicit API-only decision recorded
4. `Persistence Gate`
   - restart behavior, corruption handling, and migration/defaulting rules are explicit
5. `Release Gate`
   - readiness docs and roadmap state reflect the code that actually exists

## Structural Implementation Status

Structural scaffolding landed in this catch-up pass:

- non-vault JSON persistence now self-recovers from missing/corrupted live files
  by restoring `.bak` sidecars and quarantining broken state files
- this catch-up roadmap is now the repo-level execution plan
- readiness documentation has been reset to match the current codebase
- CI now enforces workspace tests, formatting, clippy, and audit for the
  release gate

## Phase 0 — Restore A Green Baseline

Objective: make the workspace trustworthy again before adding more scope.
Status: complete on 2026-03-24.

Deliverables:

- Fix current `sigillum-core` compile failures:
  - replace invalid `SecretKey.zeroize()` calls with a safe zeroization strategy
    that matches the actual `k256` API in use
  - fix `SigningKey.public_key()` usage in stealth signing
  - fix `Zeroizing<[u8; 32]>` test expectations in `utils.rs`
- Run and stabilize:
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets`
  - `cargo fmt --all --check`
  - `cargo audit`
- Add a basic CI baseline for build, test, fmt, clippy, and audit so regressions
  stop landing silently.
- Refresh `PRODUCTION_READINESS.md` so it reflects the actual state of the tree.

Exit criteria:

- workspace build is green
- workspace tests are green
- lint and format checks are green
- readiness docs no longer contradict the current code

## Phase 1 — Finish The Local-First Product Surface

Objective: bring existing local features to consistent operator quality.
Status: embedded UI support for provider profiles, wallet profiles, deposits,
queue, and maintenance has landed; daemon-backed CLI coverage for profiles,
deposits, queue work, maintenance, status, unlock, and compartment switching
has landed; non-vault persistence hardening has landed.

Deliverables:

- Complete UI parity for existing daemon capabilities:
  - provider profile management
  - stealth wallet profile management
  - deposit registry browsing and refresh
  - queue and sweep maintenance screens
  - maintenance-run visibility and failure inspection
- Complete CLI parity for existing daemon features where local operator use is
  expected to be scriptable.
- Tighten API/client parity:
  - every daemon route has a client method
  - every client method has route-level integration coverage
  - every request/response type has roundtrip coverage
  - gateway payment flows have local-sidecar integration coverage
- Promote queue, deposit, and profile persistence from "works locally" to
  "operationally safe":
  - atomic writes plus sidecar-backup recovery for non-vault JSON state
  - quarantine of corrupted live state before restore
  - explicit note that the supported local-first model is a single daemon per
    base directory, with in-process serialized mutation through the daemon and
    an optional local-sidecar gateway beside it

Exit criteria:

- no major daemon feature exists only in raw routes without an operator surface
- UI and CLI cover the core local workflows
- API, client, and persistence behavior are test-backed and documented

## Phase 2 — Complete Automation, Recovery, And Policy Flows

Objective: make long-running local operations reliable rather than merely
available.

Deliverables:

- Expand queue and maintenance into a complete policy engine for current flows:
  - explicit job states for blocked, deferred, retryable, and operator-action
    required cases
  - better replay and recovery after restart
  - maintenance summaries that distinguish discovery, sweep enqueue, execution,
    and failure causes
- Finish recovery semantics for destructive flows:
  - vault init/remove
  - snapshot restore
  - compartment replacement/recovery
  - deposit and queue crash recovery
- Add fuzz/property-style coverage for high-risk serialization and cryptographic
  boundary logic.
- Add a tighter release checklist for docs, tests, migrations, and local upgrade
  compatibility.

Exit criteria:

- maintenance and queue behavior is restart-safe
- destructive flows are fail-closed and recoverable
- recovery guarantees are documented and tested

## Phase 3 — Implement `eth-xpub` Project Wallets And Treasury Roles

Objective: add the new HD project-wallet family without lowering the maturity bar
 of the existing system.

Deliverables:

- Add a new `eth-xpub` wallet family alongside `eth-stealth`.
- Export only the receive-branch `xpub` at
  `m/44'/60'/{project_account}'/0/*`.
- Reserve a hidden sibling control branch under
  `m/44'/60'/{project_account}'/1/*`:
  - `/1/0` sponsor wallet
  - `/1/1` hot wallet
  - `/1/2` treasury wallet
- Add gap-limit discovery for receive addresses on the public branch.
- Support native ETH plus allowlisted ERC-20 discovery per project profile.
- Sweep discovered child addresses into the hidden hot wallet.
- Use the sponsor wallet for gas top-ups where required.
- Add treasury policies:
  - hot overflow moves excess to treasury
  - treasury refill restores hot to target when it drops below floor
  - treasury execution is gated by Sigillum quorum authority using compartment
    threshold state
- Add full API/client/storage/test/doc coverage for the new family before
  considering it done.

Exit criteria:

- project-scoped xpub wallets are discoverable, sweepable, and operable entirely
  from Sigillum
- website only needs exported public material
- treasury policies are test-backed and operationally visible

## Phase 4 — Remote And Platform Catch-Up

Objective: only after the local system is coherent, close the remaining
architecture gaps called out in the docs.

Deliverables:

- choose and implement the real remote boundary:
  - keep local-only and document it as final, or
  - build a supported remote/server mode
- if remote mode is pursued:
  - remote audit aggregation
  - connected-client monitoring
  - SSE or equivalent event streaming
  - explicit multi-host coordination rules
- decide whether deep on-chain indexing remains out of scope or becomes a real
  subsystem.

Exit criteria:

- the README and architecture docs describe the actual deployment model, not an
  aspirational one
- remote functionality, if shipped, has the same maturity bar as the local
  daemon

## Prioritization Rules

- No new product family lands on a red or flaky baseline.
- No API feature is done until the client, persistence, tests, and docs are done.
- No automation flow is done until restart/recovery behavior is explicit.
- UI debt for an existing local feature should be paid down before opening a new
  unrelated surface area.
- Remote/server work stays behind local product completion unless it directly
  unblocks the current roadmap.

## Execution Batches

This is the concrete execution order from here:

### Batch A — Structural Enforcement

- add CI for test and clippy enforcement
- add formatting enforcement and keep it green
- keep `PRODUCTION_READINESS.md` and this roadmap synchronized

### Batch B — Local Operator Surface

- add UI screens for provider profiles
- add UI screens for wallet profiles
- add UI screens for deposits, queue, and maintenance
- extend the new daemon-backed CLI surface into the remaining wallet/send and
  operator-polish gaps

### Batch C — Persistence And Recovery Hardening

- extend the newly hardened deposits, queue, and profiles stores into broader
  restart/replay guarantees
- make restart/replay guarantees explicit and test-backed
- improve maintenance and queue observability

### Batch D — `eth-xpub`

- add project-scoped receive `xpub` export
- add hidden sponsor/hot/treasury control branch support
- add discovery, sweeping, and treasury policy flows

### Batch E — Remote/Platform Decision

- either formalize local-first as the final boundary
- or implement a genuinely supported remote mode with the same maturity bar

## Immediate Next Actions

1. Land CI and the updated readiness documents.
2. Finish the remaining Batch B CLI/operator-surface gaps for wallet/send flows
   and operator polish.
3. Expand Batch C into queue-policy replay and maintenance observability.
4. Only then open the `eth-xpub` expansion batch.
