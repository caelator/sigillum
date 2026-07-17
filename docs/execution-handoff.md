# Execution Handoff — Operator Surface & Privacy Plan

**Checkpoint date:** 2026-07-17
**For:** the next engineer (human or AI) continuing this work. Zero prior
context assumed — this document is self-contained.
**Master plan:** `docs/operator-surface-and-privacy-plan.md` (task numbers
0.1–5 used below refer to it; its "Progress ledger" section tracks landed
work and is updated at each phase boundary).

---

## 1. Mission and hard constraints

Execute the operator-surface & privacy plan end-to-end: Phase 0 rc-hardening,
Phase 1 backend enablers, Phase 2 stealth ERC-5564 interop, Phase 3 HD
privacy, Phase 4 console redesign, Phase 5 evidence/docs.

Completion bar (verbatim from the goal): all code tasks in Phases 0–5
implemented on the branch with `cargo test --workspace --locked` and the UI
smoke suite green, docs (parity matrix, stability, architecture, changelog)
updated to match, and operator-owned gates (publishing, merge to main,
real-host evidence) explicitly documented as hand-back items.

Hard rules — never violated so far, keep it that way:

- **Work only in the worktree:** `/Users/xx/Documents/Workspaces/Sigillum/.claude/worktrees/agent-ux-privacy-exec`
- **Branch:** `codex/operator-surface-privacy` (based on `main` @ `815d262`).
  Never push. Never touch `main` or other worktrees/branches. The user's main
  checkout at `/Users/xx/Documents/Workspaces/Sigillum` is off-limits except
  read-only reference.
- **Commits:** `git commit --no-verify`, explicit `git add <paths>` per task
  (never `git add -A`/`git add .`). Git identity is the machine's configured
  `Codex <codex@openai.local>` — fine, use as-is.
- **UI bundle discipline:** `crates/sigillum-daemon/ui/src/app.js` and
  `src/styles.css` are CHECKED-IN build artifacts. After ANY UI source edit:
  `cd crates/sigillum-daemon/ui && npm run build` and commit the regenerated
  bundles with the change. The screenshot harness refuses to run on stale
  bundles (mtime guard) — that's intentional.
- **Cargo etiquette:** builds/tests serialize on the target-dir lock. With
  parallel agents: wait, never kill another cargo process. Full gates run in
  the background (`cargo test --workspace --locked > target/<name>.log 2>&1; echo EXIT:$?`).
- **Subagent rules that emerged:** one agent per disjoint file set; shared
  files (app.ts, contracts.ts, CHANGELOG, stability.md, routes/mod.rs) get
  edited by exactly one agent or the orchestrator; agents report, orchestrator
  reviews + commits. Subagent timeout is 30 min — resume the same agent id on
  timeout. A spawn failing with `404 ... model k3` is a transient provider
  error — just retry.

---

## 2. State of play (what's DONE — do not redo)

~90 commits. Phases 0–3 are complete and gate-green; Phase 4 is ~80% done.

- **Phase 0 (rc hardening):** C7 redesign ported onto main (4 cherry-picks);
  compact topbar; journey card collapses; dead code removed; shared confirm
  dialog with consequence tiers and every dangerous action guarded; humanized
  displays (shared `render/format.ts`); labeled policy form + plain-English
  summary; stealth guardrail warnings end-to-end; CLI mnemonic redaction +
  `eth-seed upsert`; screenshot harness `scripts/ui-screenshots/`; planner
  unwraps fixed; status-contract alignment.
- **Phase 1 (backend enablers):** structured error codes + field validation
  (`sigillum-api/src/error_codes.rs` catalog); async operations framework
  (`GET /api/operations*`, real cancel/resume for scans/drains/maintenance,
  adversarial-tested); SSE `GET /api/events` (snapshot/operation/queue/status,
  `v:1`); pagination/filter/sort on six list endpoints; background scheduler
  (retries/receipts/refresh advance with no client; honors lock/gates/kill
  switch); passive reads for status/operations/events (idle-lock can no longer
  be defeated by an open console); `/api/chains*` canonical.
- **Phase 2 (stealth interop):** hash-convention switch to
  `keccak256(compressed33)` (ScopeLift-compatible, verified against the actual
  SDK source) with dual-decode of legacy `x32` records and store migration v3;
  external fixed test vectors (no more self-roundtrip-only); watch-only
  detection (`EthereumStealthWatchView` — spending secret never enters
  detection); EIP-5564 metadata SHOULD layouts (produce+parse); per-wallet gas
  sponsor + `EthStealthGasTopup` queue payload with sweep prerequisites and
  linkage accounting; stealth sweeps gated under the Sweep execution family
  (carve-out closed); persisted announcement-scan cursors; single-key 66-byte
  meta-addresses.
- **Phase 3 (HD privacy):** provider partitioning for same-chain scans
  (`partition_providers`, consistent per-address assignment + jitter); full
  forget/prune (`/api/inventory/addresses/delete`,
  `/api/treasury/receive-addresses/purge`, profile-delete cascade); xpub
  exposure warnings + first-copy gate; `block_cross_party_linkage` default ON;
  `common_gas_funder` advisory risk findings; one-time-address lifecycle
  (allocate → auto-watch → auto-sweep → retire → purge, scheduler-driven,
  e2e-tested).
- **Phase 4 (console redesign) — what's landed:**
  - 4.1 core (`ui/src/core/`): `store.ts` (observable slices), `dom.ts`
    (`el()` + keyed `renderList`), `router.ts` (hash router + legacy adapter),
    `events.ts` (SSE client with poll fallback), `api.ts` (typed client,
    `ApiFailure` union by error code), `state.ts`, `live.ts`
    (`startCoreRuntime` composition root). All strict-typed, tested.
  - 4.2 design system v2: consequence-tier tokens, `[data-tier]`, `.table`,
    `.skeleton`, `.page-header`, `.section-empty`, `.status-dot`,
    `.attention-item`, `.nums` (tabular numerals), `ui/DESIGN.md` rules.
  - 4.3 all five destinations rebuilt as controllers:
    `ui/src/destinations/{Overview,Move,Receiving,portfolio,Vault}.ts`
    (note casing: portfolio is lowercase), styles in
    `ui/src/styles/dest-*.css`, tests in `ui/test/{Overview,Move,Receiving,portfolio,Vault}.test.ts`.
    Wired into `app.ts` (factories registered via `adapter.register(factory(runtime))`),
    CSS imported in `styles/app.css`. **183/183 UI tests green; tsc clean;
    committed as `a866c78`.** A kept-key zombie-row bug in `renderList` was
    fixed during wiring (has a regression test in `test/core-dom.test.ts`).

Latest commits: `e545627` (hero null-safety, below), `a866c78` (five
destinations), `550b69f` (maintenance summary emission rule).

## 3. EXACTLY where work stopped (resume here)

I was visually verifying the rebuilt console with the screenshot harness and
hit two failures. One is fixed; one is diagnosed-but-unfixed.

1. ✅ FIXED (`e545627`): legacy `updateHeroState` wrote to hero elements the
   Overview controller had detached → crashed the refresh loop (sidebar never
   rendered → harness failed with "workspace overview nav item did not become
   true"). Now null-safe.
2. ⛔ OPEN: periodic `TypeError: Cannot read properties of undefined (reading
   'filter')` in the bundled app when running against the MOCK server.
   **Prime suspect: the mock returns `{}` for `GET /api/operations`**
   (confirmed via curl) while the new typed client/destination controllers
   expect `{operations: []}` — every `{}` answer from the mock for endpoints
   the new destinations call will crash a controller doing
   `data.<collection>.filter/.map`. This is a mock-data gap, not a product bug
   (the real daemon always returns proper envelopes).

**Next actions, in order:**

1. Extend `scripts/ui-screenshots/mock-data.mjs` (+ the route table in
   `server.mjs` if needed) so EVERY endpoint the new destinations call returns
   a realistic envelope. Destination endpoint lists are in each controller's
   header/imports; the union includes (verify by grepping `runtime.api.` and
   `requestWithSession(` in `ui/src/destinations/`): `/api/operations`
   (list+cancel), `/api/profiles/evm`, `/api/chains`, `/api/treasury/parties`,
   `/api/treasury/receive-addresses`, `/api/profiles/eth-stealth`,
   `/api/wallets/eth-stealth/export`, `/api/plans/consolidation` (+
   generate/approve/simulate/export, enqueue-step/plan), `/api/queue/jobs`
   (+ process/pause/resume), `/api/deposits/eth-stealth` (+ all verbs),
   `/api/receiving/overview`, `/api/receiving/refresh-balances`,
   `/api/receiving/deposits/tag`, `/api/inventory/scan/evm`,
   `/api/discovery/jobs` (+cancel/resume), `/api/risk/findings`,
   `/api/risk/catalog` (+upsert/delete), `/api/inventory/token-registry*`,
   `/api/inventory/nft-metadata/*`, `/api/secrets`, `/api/api-keys`,
   `/api/compartment/list` (+switch/add), `/api/fido2/{status,detect,list}`,
   `/api/backup/*`, `/api/setup/reset`, `/api/lock`, `/api/session/revoke`,
   `/api/maintenance/run`, `/api/selfcheck/run`, `/api/diagnostics`,
   `/api/audit`. Also add `GET /api/events` SSE stub (or accept the client's
   designed 3-error poll fallback — it works, just slower).
2. Re-run `node scripts/ui-screenshots/drive.mjs` from the worktree root.
   Debug tooling: `target/ui-screenshots-debug.mjs` (a CDP inspector that
   prints body mode, nav counts, console errors/exceptions — edit ports as
   needed). Repeat until all 12 shots render populated (the harness's own
   README documents the shot list).
3. **Visually review the shots** (ReadMediaFile) for the 4.5 acceptance bar:
   no raw hex/epoch/camelCase in default view, tables used, skeletons, empty
   states, human units everywhere, no `key=value` lines.
4. Fix what the review finds (expect small controller bugs — each controller
   was written by an isolated agent against the mock, never run against real
   data end-to-end).
5. Commit mock fixes and any controller fixes separately
   (`git add` explicit paths; rebuild bundles if controllers changed).

## 4. Remaining work after the checkpoint

### 4.4 Interaction layer (plan §4.4)

Partly built into the destinations already (keyboard forms, field
highlighting from `validation_failed` `fields`, busy states). What's left,
per the plan: **⌘K command palette** (actions across destinations — register
in `app.ts` or a new `core/palette.ts`; keep zero-dep), **full keyboard audit**
(Escape dismisses dialogs/menus everywhere, focus traps in modals — the
shared `render/confirm.ts` already traps), **optimistic UI with rollback
toasts** for safe mutations (tag deposit, rename labels — judge where safe;
never for anything signing/broadcasting). Keep it proportionate — this is
polish, not a seventh destination.

### 4.5 Acceptance sweep (per destination)

Run the bar from the plan §4.5 for each of the five destinations: no raw
hex/epoch/camelCase in default view; keyboard-complete; SSE-live (no new
pollers); smoke tests updated; screenshot walkthrough produced (harness).
Record results in the plan ledger. Known gaps already logged by the
destination agents (fix or document as accepted):

- `contracts.ts` `RiskFinding` doesn't match the daemon wire (Portfolio
  defined a local type) — align `ui/src/contracts.ts` with
  `sigillum-api` and delete the local duplicate.
- Receiving: counterparty edit/delete (party sweep-destination management)
  unavailable while the new controller is mounted — port from the hidden
  legacy `receiveBookCard` or accept + document in the parity matrix.
- Move: `POLICY_PRESETS` values were invented by the agent (documented
  inline) — sanity-check them against `docs/architecture.md` policy guidance.
- Per-address balance freshness not exposed by the receiving overview API
  (section-level freshness only) — document as an accepted API gap or add the
  field additively (`sigillum-api` + daemon + controller).
- No per-job queue cancel endpoint exists (only operation-level cancel) —
  documented; adding it is optional.
- Vault: idle countdown is a "this tab's estimate" (daemon exposes no
  last-activity timestamp) — honest label already; optional: expose
  additively in `StatusResponse`.
- Snapshot restore file-read flow is untested (fake-DOM has no `File`) —
  cover in the browser smoke instead.

### Phase 4 gate + docs

- Full gate: `cargo test --workspace --locked` (background; the daemon suite
  alone is ~10+ min under load) AND `cd crates/sigillum-daemon/ui && npm test
  && npm run typecheck && npm run build` AND the screenshot walkthrough.
- Docs lockstep (project rule): `docs/operator-surface-parity.md` (migrated
  destinations replace legacy card rows; note gaps accepted above),
  `docs/architecture.md` (console architecture section: core modules, adapter
  contract, SSE consumption), `docs/stability.md` (any wire changes since the
  last entry), `CHANGELOG.md` (Unreleased), plan ledger.
- `ui/DESIGN.md` — add the destination-controller authoring contract if not
  already there (mount/unmount rules, takeover-safety rule below).

### Takeover-safety rule (new, from the checkpoint bug)

When a migrated controller takes over a legacy card, every legacy writer to
that card's DOM must no-op. The legacy refresh loop is mostly null-safe, but
verify per card (the Overview/#statusCard crash is the pattern: direct
`element.textContent` writes without null checks). The `renderList` kept-key
contract: renderers may either patch `existing` or return a fresh node (old
node is now removed) — documented in `core/dom.ts`.

### Phase 5 (evidence, docs, hand-back)

- Promote the screenshot walkthrough into release evidence. The user's
  evidence store is OUTSIDE the repo: `/Users/xx/Documents/ReleaseEvidence/Sigillum/`
  (see its `v1.0.0-rc.5/` layout for the expected shape). Copy the final shot
  set + a walkthrough note there; do NOT commit binary shots to the repo.
- Run `scripts/browser-smoke.mjs` against a REAL daemon (it needs a live
  daemon + Chrome; see `scripts/check-browser-smoke.sh`) — the Phase 0 port
  updated selectors but it was never executed. Also `scripts/check-runtime-smoke.sh`
  if applicable.
- Consider hardening the three known timing-sensitive tests (do NOT do this
  before the final gate — note for the operator):
  `tests/events_idle.rs` (load-sensitive), `daemon_service`
  `successful_restore_clears_session...` (15s client timeout too tight under
  load), `tests/scheduler.rs` (`scheduler_skips_everything...` takes 60-160s
  under load).
- Docs refresh + hand-back report: list the operator-owned gates explicitly —
  (1) review + merge `codex/operator-surface-privacy` → `main` (PR; note the
  user's docs-refresh branch `codex/public-docs-refresh` adds `docs/README.md`
  — my plan-doc link edit there must be re-applied after it merges, see §6);
  (2) C7 sign-off (superseded — this branch went far beyond C7); (3) H2
  publish decision per `docs/execution-runbook-1.0.md`; (4) real-host evidence
  (F4 soak, F6 testnet receipts, doctor on every host) — not automatable from
  here; (5) rc.6 build + SHA256SUMS + evidence bundle.
- Optional deferred items (recorded in the plan): ERC-6538 registry (D-E),
  per-address freshness API, per-job queue cancel, `/api/audit` cursor
  pagination, test-hardening above, pre-existing fmt/clippy drift (leave it —
  don't mix into feature commits).

## 5. Verification playbook (how this project proves things)

- Rust: `cargo test --workspace --locked` (full gate, run in background,
  check `grep -cE "test result: ok"` + no `FAILED`). Targeted:
  `cargo test --locked -p sigillum-{api,daemon,client,cli} [--test <suite>]`.
- UI: `cd crates/sigillum-daemon/ui && npm test` (fake-DOM smoke, node:test;
  currently 183), `npm run typecheck`, `npm run build` (rebuild bundles!).
- Visual: `node scripts/ui-screenshots/drive.mjs` (needs Chrome at
  `/Applications/Google Chrome.app`; mock server requires fresh bundles).
- Rust integration suites of note: `crash_recovery`, `adversarial_execution`,
  `execution_gates`, `gas_topup`, `plan_enqueue`, `stealth_*`,
  `discovery_operations`, `queue_operations`, `scheduler*`, `events*`,
  `list_queries`, `forget_prune`, `one_time_receive`, `provider_partitioning`.
- Repo AGENTS.md asks for GitNexus re-index after meaningful changes:
  `gitnexus-analyze-safe <worktree path>` (safe to run; output gitignored).

## 6. Repository topography you must not trip over

- Main checkout (`/Users/xx/Documents/Workspaces/Sigillum`) is on branch
  `codex/public-docs-refresh` (one docs commit ahead of main: adds
  `docs/README.md` index + the plan doc at `docs/operator-surface-and-privacy-plan.md`
  and an index link to it — both UNCOMMITTED there; the committed copy of the
  plan doc lives on OUR branch). When our branch merges to main, re-apply the
  docs-index link on that side.
- Many `.claude/worktrees/agent-*` exist (other agents' work). Ignore them.
- `docs/README.md` does NOT exist on our branch (only on docs-refresh) — our
  plan doc is linked from the ledger, not the index.
- The UI's legacy app (`ui/src/app.ts`) is `// @ts-nocheck` by history; all
  NEW code (`core/`, `destinations/`) is fully typed and must keep
  `npx tsc --noEmit` clean. Don't add `@ts-nocheck` anywhere new.
- `cargo fmt --check` and `cargo clippy -D warnings` are NOT clean at base
  (toolchain drift + pre-existing lints). Format only files you touch; don't
  "fix" unrelated drift inside feature commits.
- Filename casing is inconsistent: `destinations/portfolio.ts` +
  `test/portfolio.test.ts` + `styles/dest-portfolio.css` are lowercase, the
  other four destinations are Capitalized. Imports must match exact case
  (fine on macOS; be careful on case-sensitive systems — normalize if you
  touch them anyway).

## 7. Goal/session bookkeeping (for an AI successor)

- There is an active goal tracking this work (objective = execute the plan;
  completion criterion in §1). If you are continuing it in this session,
  resume goal work normally; the runtime tracks it. `TodoList` should mirror
  §4 above.
- The plan ledger (`docs/operator-surface-and-privacy-plan.md` → "Progress
  ledger") is the durable record — update it at every phase boundary and
  commit. It currently runs through Phase 2 + 3.3; add the missing Phase 3
  (3.1, 3.2, 3.4, 3.5) and Phase 4 (4.1–4.3) entries — they landed as commits
  `adab7a0`..`af450c3` (Phase 3) and `2bfa7f3`..`e545627` (Phase 4).
- If context compacts: this handoff + the ledger + `git log --oneline` are
  sufficient to reconstruct everything. Do not re-audit finished phases.
