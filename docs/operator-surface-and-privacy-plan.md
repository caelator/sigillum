# Operator Surface & Privacy Implementation Plan

Status: **in execution** — started 2026-07-16 on branch
`codex/operator-surface-privacy` (worktree `.claude/worktrees/agent-ux-privacy-exec`).
Scope: console UI/UX overhaul, throwaway-wallet privacy, ERC-5564 stealth
correctness and interoperability.

## Progress ledger

Decisions ratified by the operator at execution start: D-A (stealth hash
switch with dual decode), D-B option A (zero-dependency frontend
re-architecture), D-C (C7 as redesign base, ported — see below), D-D (SSE
ratified), D-E (ERC-6538 deferred).

- **Phase 0 — complete** (gate: full workspace tests + 74-console-test suite green):
  - 0.1 C7 ported onto main 815d262 as 4 commits (`6988284`, `6e83590`,
    `04185fd`, `5ddba45`); main-era features (NFT metadata, token registry,
    manual enqueue, extended policy) preserved into the five-destination IA.
  - 0.2 topbar compacted + journey card collapses (`685a13c`); 0.3 dead
    code/copy removed (`cd4cf72`).
  - 0.5 shared confirm dialog + danger model aligned (`2014278`).
  - 0.6 humanized displays (`ba72449`); 0.7 policy labels + summary (`4e8d1eb`);
    0.8 status-contract alignment + cancel gating (`fd853da`), planner
    fail-closed (`8507f51`), UI CSP test fix (`a0538d6`).
  - 0.4 stealth guardrails end-to-end (`6dc51bd`, console `2d40fb7`);
    0.9 CLI mnemonic redaction + eth-seed upsert (`4c9be7d`).
  - 0.10 screenshot harness `scripts/ui-screenshots/` (`a1efff6`); changelog +
    stability pre-tag notes (`f4e5968`, `7de4cd6`).
- **Phase 1**:
  - 1.4 structured error codes + field validation (`0e6a572`..`e03501d`, 7
    commits; code catalog in `sigillum-api/src/error_codes.rs`).
  - 1.1+1.2 async operations framework, real discovery cancel/resume
    (`ee1540d`..`32ccd22`, 8 commits; adversarial suite
    `tests/discovery_operations.rs`); 1.1b async drain/maintenance
    (`12b5ccc`..`8dfbbb8`, 5 commits; `tests/queue_operations.rs`).
  - 1.3 SSE channel + passive-read idle fix (`8635c7f`, wiring `68fde62`);
    1.5 pagination/filter/sort on six list endpoints (`a1187c4`, wiring
    `68fde62`); 19 pagination integration tests, SSE idle-eviction proof.
  - 1.6 background scheduler (`e6a7134`, `b258acc`, `4e569a2`): queue
    retries/receipts/deposit refresh advance with no client; guard with
    skip-on-contention; honors lock/gates/kill switch; env config; diagnostics
    block.
  - 1.7+1.8 (`0d509dc`): status/operations polling joins events as passive
    reads (idle-lock can no longer be defeated by an open console);
    `/api/chains*` canonical, `/api/inventory/chains*` deprecated alias.
- **Phase 2**:
  - 2.1+2.2 stealth hash-convention switch with dual decode (`53152f2`,
    coverage `c0a231f`): new payments derive keccak256 over the 33-byte
    compressed SEC1 point (ScopeLift-compatible); deposits store migrates to
    schema v3 stamping pre-switch records `x32`; detection probes standard
    then legacy and re-stamps on match; sweeps use the record's stamp with
    address-verified probe fallback; fixed external vectors (SDK-published
    keypair, independent @noble reference) pin both conventions.
  - 2.5 stealth execution-gate carve-out closed (`9c3bb24`): stealth
    transfers/sweeps gate under the Sweep family (`allow_plan_execution` +
    `allow_sweep_execution`) at enqueue and drain, mirroring EthSeed*; the
    sponsor gas top-up keeps its `allow_gas_topups` carve-out; the policy UI
    states the coverage; pre-tag adjustment recorded.
  - 2.6 stealth ergonomics (`5d705cc`, single-key meta-addresses in the
    follow-up commit): persisted per-(wallet, provider) announcement-scan
    cursors — `from_block`-less scans resume incrementally, explicit
    `from_block` still wins, `reset_cursor` re-anchors — and single-key
    (66-hex) meta-addresses per the EIP single-key rule, pinned by fixed
    vectors under both hash conventions.

## Original plan

This plan consolidates three evaluations performed against `main` (247cf51) and
the C7 redesign branch (`worktree-agent-ae37914b9b627cdd5`):

1. **Console UI/UX audit** — code audit of `crates/sigillum-daemon/ui` plus
   rendered screenshot review of both UI versions with realistic mock data
   (artifacts: `target/ux-eval/shots/`, gitignored scratch).
2. **Feature/privacy gap analysis** — HD multi-address ("throwaway wallet")
   flows end-to-end.
3. **Stealth-address audit** — `ethereum_stealth.rs` verified point-by-point
   against EIP-5564 / EIP-6538 and the two major ecosystem SDK conventions.

The through-line of all three: **the engine is world-class; the operator
surface and the privacy boundaries are not.** The vault, queue, crash-recovery,
and audit machinery meet a bar most production wallets don't. The UI renders
API wire dumps, scanning deanonymizes the operator at the RPC provider, and the
stealth implementation cannot interoperate with any standard sender.

---

## 1. Guiding principles

- **Consequence-graded interface.** Visual weight and friction proportional to
  money at stake. Browsing is quiet and fast; approval is deliberate and loud.
  One confirmation component, three tiers (inform → confirm → typed phrase),
  applied by a published consequence matrix — never ad hoc per screen.
- **Every screen answers a question.** "What needs my attention?", "Is this
  plan safe?", "How do I get paid privately?" No `key=value` dumps, no raw hex
  by default, no camelCase state lines. Human units (ETH/gwei, chain names,
  relative time) everywhere.
- **Privacy is a boundary discipline, not a feature flag.** On-chain
  unlinkability is only as strong as the weakest boundary: the RPC provider,
  the gas-funding leg, the announcement channel, and the at-rest linkage
  ledger. Every privacy task below names its boundary.
- **Fail-closed stays fail-closed.** No UX work may weaken execution gates,
  typed confirmations where value moves, simulation re-verification, or the
  never-re-sign queue invariants. Redesign must not add runtime dependencies
  with meaningful supply-chain surface (see D-B).
- **Docs and parity matrix stay in lockstep.** Every surface change updates
  `docs/operator-surface-parity.md`; every contract change updates
  `docs/stability.md` and `CHANGELOG.md`.

---

## 2. Decisions to ratify before Phase 0

### D-A — Stealth shared-secret convention: switch before 1.0

`hashed_shared_secret` computes `keccak256(x-coordinate ‖ 32)`
(`crates/sigillum-core/src/ethereum_stealth.rs:775-776`). The de-facto
scheme-1 majority (ScopeLift `stealth-address-sdk`) hashes the 33-byte
compressed point; Fluidkey hashes 64-byte X‖Y. EIP-5564 never pinned the
encoding, so all three diverge. Consequences today:

- A standard sender paying a Sigillum meta-address produces a stealth address
  Sigillum cannot detect (view-tag mismatch) and cannot spend (offset
  mismatch). Funds are stranded, recoverable only with external tooling.
- `/api/wallets/eth-stealth/generate` accepts any meta-address and mints
  Sigillum-flavored addresses third parties cannot detect — a fund-loss
  footgun (`crates/sigillum-daemon/src/service/wallets.rs:181-213`).

Sigillum's stealth tests are all self-roundtrips, which is how this survived
review. **Recommendation: switch to `keccak256(compressed point ‖ 33)` before
1.0, with dual-decode for legacy records (Phase 2, task 2.1).** Every release
that ships the current convention grows the legacy record set and the chance
a real user strands funds. If the operator judges the switch too risky for
rc, the minimum viable action is Phase 0 task 0.4 (guardrails + loud docs).

### D-B — Frontend stack: zero-dependency re-architecture (recommended)

Current state: hand-rolled TS, `innerHTML` strings, a 1,433-line orchestrator
under `// @ts-nocheck` (`ui/src/app.ts:1`), a full-workspace refetch + full
re-render every 5 s (`ui/src/state/refresh.ts:1`), one dead parallel typed API
client, `strict: false`. This architecture caps achievable UX quality and is
already producing focus loss and wiped in-progress input.

- **Option A (recommended):** keep zero runtime dependencies; re-architect as
  strict TypeScript with a small auditable render/component core (~200 lines),
  a store with per-resource subscriptions (render what changed), URL routing
  for deep links, one API client, and shared primitives (`formatEth`,
  `Address`, `ConfirmDialog`, `RelativeTime`).
- **Option B:** adopt Preact (~4 kB) with precompiled templates. Less custom
  code; adds a (small, pinned, auditable) supply-chain surface. Acceptable
  fallback if Option A's custom-core maintenance is judged too costly.

Both are compatible with the existing fake-DOM `node --test` smoke harness,
which is worth keeping — it caught real regressions.

### D-C — C7 branch merges as the redesign base (recommended)

C7 (`worktree-agent-ae37914b9b627cdd5`, 4 commits) delivers the
five-destination IA (Overview/Receive/Portfolio/Move/Vault), hero
deduplication, and untruncated stat labels. It is a genuine improvement but a
rearrangement, not a redesign: the policy wall, `key=value` meta lines, raw
wei hex, and unlabeled inputs survive (verified in `c7-move.png`,
`c7-portfolio.png`). **Recommendation: merge C7 after fixing its two known
regressions (Phase 0 task 0.2), then treat it as the IA foundation Phase 4
builds on — not as the finished UX.** The release plan's C7 sign-off gate
should record that understanding explicitly.

### D-D — SSE event channel: amend the documented non-goal

`docs/architecture.md:289` lists SSE as a non-goal. A live UI without a push
channel is what produced the 5-second full re-render — and polling with the
session token resets idle activity, silently defeating the 15-minute vault
auto-lock (`state.rs:672-692`). **Recommendation: ratify SSE (loopback-only,
same auth model) as a 1.x addition**, with heartbeats explicitly not counted
as session activity. This is the single backend change with the most UX
leverage.

### D-E — ERC-6538 registry: defer, optional

No registry support exists (no `registerKeys`, no resolution). Meta-address
distribution is copy/paste. Defer to Phase 4 as an optional item; it is
convenience, not a correctness gate, and adds an on-chain footprint some
operators will not want.

---

## 3. Phase plan

Effort figures assume one senior engineer; tracks marked ∥ can run in
parallel. All phases keep the existing adversarial/failpoint test culture —
see §4.

### Phase 0 — 1.0 hardening and honest UI (target: rc.6; ~1–2 weeks)

Goal: ship the release with no known-broken claims, no inverted danger model,
and C7's improvements. No re-architecture.

| # | Task | Where | Acceptance |
|---|------|-------|------------|
| 0.1 | Merge C7 to `main` via PR; run full workspace tests + UI smoke + browser smoke | branch `worktree-agent-ae37914b9b627cdd5` | green CI; C7 walkthrough gate signed |
| 0.2 | Fix C7 regressions: 3-row topbar eats ~150 px on every destination (compact chip row); completed "Treasury setup 4 of 4" card collapses to a one-line done state | `ui/src/views/shell.ts`, `app.ts`, journey rendering | screenshot evidence both states |
| 0.3 | Kill dead copy and dead code: "live balance refresh arrives in increment B2" (`index.after-style-before-script.html:377`, `receiving.ts:44`); unreachable hero setup/locked copy (`app.ts:492-521` vs `09-overview-auth.css:132-134`); dead typed API client (`ui/src/api.ts`, `ui/src/actions/session.ts`) and unused exports | ui | grep shows no references; smoke tests updated |
| 0.4 | Stealth guardrails (independent of D-A timing): warn loudly when `eth-stealth/generate` targets a meta-address that is not the operator's own; warn on operator-supplied `ephemeral_private_key_hex` (reuse → identical stealth address); docs state the hash-convention deviation explicitly until Phase 2 lands | `service/wallets.rs:181-213`, `docs/architecture.md`, `CHANGELOG.md` | warnings in API response + UI + docs |
| 0.5 | Danger-model quick fixes: `Process Queue` / per-job `Process` require a confirm dialog (they broadcast real sends — `operations.ts:525-533,558`); `enqueueDepositSweep`, `rotateTreasuryReceiveAddress`, `deleteTreasuryParty` get confirms; single shared confirm component replaces `window.confirm`/`prompt`/two-click-arm sprawl | `ui/src/views/operations.ts`, `treasury.ts`, new `ui/src/render/confirm.ts` | consequence matrix table in code review; all value-moving actions behind ≥ confirm tier |
| 0.6 | Humanize the worst data displays: inventory native balances + holding amounts from raw hex to ETH (`inventory.ts:544,588-589`); deposit `expected/observed` hex → ETH (`operations.ts:148-159`); watch-book / token-registry / NFT unix seconds → locale time (`inventory.ts:356-357,631-632,687-688`); chain names from the chain registry instead of "chain N" | ui views | no raw hex or epoch seconds in default rendering (details disclosure may keep them) |
| 0.7 | Label the treasury policy numeric inputs (0.5/2/900/0.15/0.6/1.5/0.05 currently unlabeled); convert the camelCase state line to a plain-English policy summary; move the three legalese paragraphs behind "How this policy protects you" disclosure | `treasury.ts` policy form, C7 Move equivalent | every input has a visible label; summary reads as sentences |
| 0.8 | Real bugs: status-shape mismatch (`contracts.ts:6-12` `id/label` vs `app.ts:524,562` `compartment_id/compartment_label` — pick one, fix types); planner unwraps (`service/inventory/planner.rs:422,423,436` → error paths); discovery-job cancel/resume honesty — until Phase 1 makes them real, the cancel button must not pretend (disable with tooltip or implement cooperative cancel per task 1.2 early) | daemon + ui | regression tests; no `unwrap` in planner |
| 0.9 | CLI hygiene: `eth-seed create` must not print the mnemonic to stdout unguarded (print-once-then-prompt, or `--out file` with 0600); implement the advertised `profiles eth-seed upsert` arm (import existing mnemonic) | `sigillum-cli/src/daemon_api.rs:282-299,560-566` | CLI/UI parity matrix updated |
| 0.10 | Evidence + release mechanics: rc.6 build, soak receipts, screenshot walkthrough (reuse the `target/ux-eval` harness pattern as a permanent script under `scripts/`), runbook §5 gates | `scripts/`, `ReleaseEvidence/` | rc.6 evidence bundle; H2 publish decision unblocked |

### Phase 1 — Backend enablers for a live UI (~3–4 weeks; ∥ with Phase 2)

Goal: make the daemon capable of supporting a live, forgiving, job-oriented
UI. No UI redesign yet — the current UI adopts these incrementally.

| # | Task | Where | Acceptance |
|---|------|-------|------------|
| 1.1 | Generic async-operation resource: long work (discovery scans, queue drains, maintenance, deposit refresh) runs as daemon-side jobs with progress, ETA, and cooperative cancellation — not inside the HTTP request | `service/inventory.rs:271-456`, `service/maintenance.rs:16-99`, `service/queue/processing.rs` | scan/drain return a job id immediately; `GET /api/operations/{id}` reports progress |
| 1.2 | Real cancel/resume for discovery: the scan loop checkpoints and honors cancel; resume continues from checkpoint. Replaces the current fake verbs (`service/inventory/discovery_jobs.rs:39-70` — cancel is overwritten by the running scan, resume is consumed by nothing) | inventory + discovery_jobs | adversarial test: cancel mid-scan, resume, verify no duplicate observations |
| 1.3 | SSE channel `GET /api/events`: status, queue state changes, operation progress, lock events. Heartbeat does **not** refresh session idle activity (fixes polling-defeats-auto-lock, `state.rs:690`) | new `routes/events.rs`, `state.rs` | UI e2e: lock from second client reflects in first within 1 s without a poll |
| 1.4 | Structured error codes + field-level validation: `ErrorResponse { code, error, action?, fields? }`; disambiguate the overloaded 403 (locked vs gate-denied vs scope) and 404 (not-initialized); map every validation failure to field paths | `sigillum-api/src/response.rs:23-28`, `service/error.rs:61-133`, `sigillum-api/src/validation.rs` | client/CLI updated; stability doc lists the code catalog |
| 1.5 | Pagination/filter/sort on list endpoints (queue jobs, inventory wallets, deposits, plans, risk findings, audit already has `tail`); total counts; aggregate "last refreshed" watermark in overview responses | routes + stores | UI lists render first page < 100 ms at 10k records |
| 1.6 | Background scheduler: queue retries, receipt confirmation, deposit refresh advance without a client calling process/maintenance. Honors the same gates/kill switch; disabled in `doctor` read-only contexts | new `service/scheduler.rs`, `lib.rs` spawn site | failpoint test: retrying job drains with no client connected |
| 1.7 | Session semantics for UIs: memory-only sessions stay (security choice), but lock/idle/restart transitions become distinguishable via 1.4 error codes; document the "polling does not extend idle lock" rule after 1.3 | `state.rs`, docs | UI can render "locked — unlock to continue" deterministically |
| 1.8 | Contract cleanup: remove duplicate `/api/chains` vs `/api/inventory/chains` registrations (keep one, alias the other with deprecation); unify REST/RPC verb mix on `treasury/parties`; regenerate client DTOs | `routes/mod.rs:452-463,590-600` | stability policy updated; no duplicate route docs |

### Phase 2 — Stealth correctness, interoperability, and gas (~3–4 weeks; ∥ with Phase 1)

Goal: turn "ERC-5564-style" into ERC-5564. Prerequisite decisions: D-A (and
optionally D-E, deferred).

| # | Task | Where | Acceptance |
|---|------|-------|------------|
| 2.1 | **Hash-convention switch with dual decode.** New deposits/derivations use `keccak256(compressed ‖ 33)`; detection and key recovery compute *both* the standard and legacy (x-only) hash so rc-era records remain detectable and spendable. Record the convention version on each deposit record; export includes it | `ethereum_stealth.rs:771-777`, `deposits.rs` scan/check paths, storage migration | legacy fixture deposit from rc.5 still sweeps in test; new deposit matches ScopeLift test vector |
| 2.2 | **External test vectors.** Add published cross-implementation vectors (generate with ScopeLift SDK, commit as fixtures) covering: meta-address parse, view tag, stealth address, spending-key recovery. No more self-roundtrip-only coverage | `ethereum_stealth.rs` tests | CI fails if either convention regresses |
| 2.3 | **Watch-only scanning.** Implement `checkStealthAddress` per the EIP signature (viewing key + spending *public* key); deposit detection requires only the viewing compartment unlocked; sweeping still requires the spending compartment | `ethereum_stealth.rs:300-329`, `deposits.rs:558-568` | scan e2e passes with spending compartment locked |
| 2.4 | **Gas story.** (a) Produce and parse the EIP metadata SHOULD layouts (`0xeeeeeeee`+address+amount; token selector+amount) so payers can attach gas ETH and scans learn asset/amount without the operator guessing `token_address`; (b) extend `fund_gas` sponsor top-ups to stealth deposits with the same linkage accounting as seed plans (`gas_topup.rs:191-196` currently seed-only); (c) UI requests payer-attached gas at deposit creation | `ethereum_stealth.rs:830-838`, `deposits.rs:1637-1640`, `gas_topup.rs`, `sweeps.rs:143-151` | ERC-20 stealth deposit sweeps end-to-end without a manual external gas transfer; linkage events audited |
| 2.5 | Reconcile the execution-gate carve-out: `EthStealth*` jobs bypass treasury execution gates (`gates.rs:101-107`). Either route them under `allow_sweep_execution` (with migration note) or surface the carve-out explicitly in the policy UI with a justification | `service/queue/gates.rs`, policy UI | policy screen accurately reflects what gates stealth sweeps |
| 2.6 | Stealth ergonomics: persist announcement-scan cursors per wallet (replace manual `from_block`, 10k cap); support single-key (66-byte) meta-addresses per the EIP `n`-byte rule; honor the 0.4 warnings | `deposits.rs:47-48`, `ethereum_stealth.rs:708` | rescan is incremental; single-key meta-address parses |
| 2.7 | (Optional, D-E) ERC-6538: `registerKeys` publish flow + `stealthMetaAddressOf` resolution, off by default | new module | operator opt-in documented |

### Phase 3 — HD privacy hygiene and the one-time lifecycle (~3 weeks; after 1.6, 2.4)

Goal: make "spawn throwaway addresses from one seed" actually private at every
boundary, not just on paper.

| # | Task | Where | Acceptance |
|---|------|-------|------------|
| 3.1 | **Provider partitioning for scans** — the highest-privacy-ROI item. Assign provider profiles per wallet (or per account), round-robin with jitter, and batch balance calls so no single endpoint sees an ordered full address tree. Document residual provider visibility honestly | `service/inventory.rs:362-456`, `observation.rs:73-84`, profiles | test: two providers each observe a disjoint address subset |
| 3.2 | **Forget/prune.** Delete endpoints for scanned-address records and retired allocations (with audit events); deleting a seed/xpub profile cascades to its inventory rows and counterparty bindings behind a typed confirm; snapshot docs updated | inventory store, `treasury.rs`, routes | pruning test: record gone from store, audit trail, and re-scan does not resurrect stale counterparty bindings |
| 3.3 | **One-time-address mode.** allocate → auto-watch → auto-sweep-on-funds (via 1.6 scheduler + 2.4 gas) → retire → optional purge. Per-allocation policy: destination, sweep threshold, purge-after-sweep | `treasury.rs:1188-1369`, scheduler, queue | e2e: fund a fresh allocation, observe sweep to destination, record retired and purged |
| 3.4 | xpub hygiene: export/copy shows a warning that an xpub exposes the whole address tree (UI + CLI); consider policy-gating `xpub-export`; restrict or scope the unauthenticated `xpub-derive` oracle (`wallets.rs:128-138`) | `service/wallets.rs`, UI wallets view | warning copy reviewed; parity doc updated |
| 3.5 | Linkage defaults and detection: `block_cross_party_linkage` default-on at the API layer (currently `unwrap_or(false)`); detect and warn when one funder pays gas for multiple receive addresses (common-funder linkage), beyond generated top-ups | policy defaults, planner linkage analysis | plan generation warns on common funder; defaults migrated with changelog note |

### Phase 4 — Console redesign to the world-class bar (~8–10 weeks; after 1.1–1.5)

Goal: a control-room instrument, not an admin panel. Base: merged C7 IA +
D-B stack decision + Phase 1 enablers.

**4.1 Frontend re-architecture (weeks 1–2).** Strict TS workspace-wide; store
with per-resource subscriptions fed by SSE (fallback poll); keyed list
rendering (no focus loss, no wiped inputs); hash-based routing with deep links
per destination; single API client generated/aligned with `sigillum-api`
types; shared primitives (`Address` with middle-truncate+copy+chain chip,
`Amount` with tabular numerals, `RelativeTime`, `ConfirmDialog` tiers,
`Progress`, `EmptyState`, `Skeleton`); delete the last `key=value` meta-line
component. Keep and extend the fake-DOM smoke harness.

**4.2 Design system v2 (weeks 2–3).** Consequence tiers as first-class tokens
(quiet → review → danger); AA+ contrast; tabular numerals; density that
privileges the next action; motion budget with `prefers-reduced-motion`;
dark-first with light theme as follow-up. Component catalogue replaces the 15
CSS partials' drift; inline styles banned by lint.

**4.3 Destination rebuilds (weeks 3–9), in priority order:**

1. **Move + plan review — the signature moment.** Plan review as a
   hardware-wallet-grade approval screen: per-step cards in plain language
   ("Sweep 0.42 ETH from 0x71C7…976F → Treasury vault (cold)"), simulation
   badge with evidence age, gas per step, destination trust chip, linkage
   warnings, running total, one deliberate typed-confirm approve. Queue as an
   ops timeline (state transitions, errors with retry affordances), not log
   lines. Policy as presets ("Consolidation", "Recovery operator", "Custom")
   + guided editor with labeled fields, inline validation, and a
   plain-English "what this policy will do" summary.
2. **Overview — "what needs my attention?"** Ranked attention queue
   (review-required plans, `operator_action_required` jobs, failed self-check
   domains, stale scans) each with one action; freshness watermark; recent
   audit digest. Collapses to calm when nothing needs attention.
3. **Receiving.** Address cards (copy, purpose, counterparty, balance,
   freshness); one-time-address mode from 3.3 as a first-class flow; stealth
   deposit status as a guided lifecycle (announced → funded → gas-ready →
   swept); payer instructions panel (meta-address + what to attach).
4. **Portfolio.** Real tables: chain names, human amounts, per-address
   freshness, watch-only/signer affordances; scan as a stepper (pick wallets
   → pick providers with partitioning visible → progress via SSE → results
   summary), replacing the ~30-field form.
5. **Vault.** Compartments, secrets, hardware keys, snapshots as one coherent
   security story: lock state always visible, session countdown, capability
   tokens explained, backup nudges.

**4.4 Interaction layer (weeks 8–10).** ⌘K command palette; full keyboard
flows (Enter submits, Escape dismisses, focus traps in modals); optimistic UI
with rollback toasts; persistent banners for stale data (replacing silent
`catch{}` staleness); error rendering driven by 1.4 codes with field
highlighting; empty states with next-action buttons everywhere.

**4.5 Acceptance bar per destination** (gate): no raw hex/epoch/camelCase in
default view; axe-core clean; keyboard-complete; SSE-live; smoke tests updated;
screenshot walkthrough committed to release evidence.

### Phase 5 — Evidence, docs, and release 1.1 (~1–2 weeks)

- Screenshot/UX walkthrough evidence promoted from scratch to `scripts/` +
  `ReleaseEvidence/`; C7-era `target/ux-eval` harness generalized.
- Docs refresh: architecture (SSE, scheduler, operations resource), privacy
  model (provider partitioning residuals), stealth convention + migration
  note, parity matrix, stability policy (error-code catalog, SSE surface),
  roadmap update.
- Release mechanics per `execution-runbook-1.0.md` pattern: soak receipts,
  doctor on all hosts, publish decision.

---

## 4. Cross-cutting test strategy

- **Keep the adversarial culture.** Every Phase 1/2 state-machine change gets
  failpoint crash-injection tests in the style of `tests/crash_recovery.rs`
  and `tests/adversarial_execution.rs`.
- **Stealth:** external cross-implementation vectors (2.2) are the headline
  addition; plus upgrade tests proving rc-era deposits survive the convention
  switch (2.1), and watch-only scan e2e (2.3).
- **Privacy:** provider-partitioning tests assert disjoint observation sets
  (3.1); prune tests assert no resurrection (3.2).
- **Backend:** SSE tests (connect, resume, lock event); scheduler tests with
  no client connected; pagination bounds tests; error-code snapshot tests so
  the catalog is stable.
- **UI:** extend the fake-DOM smoke suite per destination (it already covers
  action contracts well); add screenshot walkthrough as release evidence, not
  as a flaky gate.
- **No gate regressions:** any PR touching enqueue/drain paths must show the
  gates/typed-confirm/kill-switch tests still green.

## 5. Risk register

- **Dual-hash burden (2.1).** Supporting two conventions forever is real
  complexity; mitigate by version-stamping records and documenting the legacy
  path as frozen (no new legacy deposits after rc.6).
- **SSE scope creep (1.3).** Keep the event vocabulary minimal (status,
  operation progress, queue change, lock). Do not build a general pub/sub.
- **Scheduler vs fail-closed (1.6).** Background execution must honor every
  gate and the kill switch exactly as request-driven drains do; test with
  failpoints.
- **Redesign scope (Phase 4).** The destination rebuilds are sequenced so
  Move + Overview can ship alone if the rest slips; do not hold them hostage
  to Portfolio/Vault.
- **Privacy overclaim.** Docs must stay candid: provider partitioning reduces,
  does not eliminate, provider-side linkage; one-time mode reduces, does not
  erase, at-rest history unless purge is enabled.
- **C7 merge timing.** Merging C7 without tasks 0.2/0.3/0.7 re-ships known
  problems under a new IA; sequence 0.1→0.3 as one PR series.

## 6. Explicit non-goals (unchanged)

Swap execution / DEX routing (D-13); fiat or NFT valuation (D-16); external
runtime risk feeds (D-15); non-EVM chains (D-9, post-1.0); hosted, remote, or
multi-user modes; Tor transport (documented as future enhancement only);
Windows (D-2); crates.io publishing (D-1).

## 7. Sequencing summary

```
Phase 0 (rc.6 hardening) ──┬──> Phase 1 (backend enablers) ──> Phase 3 (HD privacy) ──┐
                           └──> Phase 2 (stealth interop)  ──────────────────────────┤
                                                                                     ▼
                                                          Phase 4 (console redesign) ──> Phase 5 (1.1)
```

Phases 1 and 2 are independent and parallelizable. Phase 3 needs 1.6
(scheduler) and 2.4 (gas). Phase 4 needs 1.1–1.5 (it *is* the consumer of
those enablers) and starts with D-B/D-C ratified. Total critical path to a
redesigned 1.1: roughly 14–18 engineer-weeks.
