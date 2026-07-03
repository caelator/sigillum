# Sigillum 1.0 Execution Runbook

**Status:** Active operational handbook for completing the 1.0 release
**State recorded:** 2026-07-03, `main` @ `6cfcdce` (Wave 1 merged)
**Pairs with:** [release-1.0-plan.md](./release-1.0-plan.md) — the task
specifications and Decision Register. This runbook does not restate task
specs; it tells the executing agent **what is already done, how to run the
remaining tasks, in what order, and how to recover when things go wrong.**
Everything here was learned by actually executing Phases A–D; follow it
literally.

If this runbook and the plan ever disagree on a task's *content*, the plan
wins. If they disagree on *process or sequencing*, this runbook wins (it is
newer and battle-tested).

---

## 1. Current state ledger

### 1.1 Done and merged to `main`

| Scope | Evidence |
|---|---|
| Phase A (desktop branch landed) | PR #1, squash `fd2b35b`; CI green both legs |
| Wave 1 = B1, B2, B3, B4, C1, C2, C3, C5, C6, D1, D2 | PR #2, merge `6cfcdce`; CI green both legs; per-task commits preserved in history |

The Master Checklist in `release-1.0-plan.md` §5 reflects this — trust the
checkboxes; keep updating them as you complete tasks (plan rule §0.1.6).

### 1.2 Complete but NOT merged — action required

- **C7 (console UX redesign):** complete and fully verified on branch
  `worktree-agent-ae37914b9b627cdd5` (4 commits, tip `24a1597`; worktree at
  `.claude/worktrees/agent-ae37914b9b627cdd5` if it still exists — the
  branch is what matters). **Blocked on operator sign-off of the
  screenshots** (review page was delivered to the operator). Procedure on
  approval: §4 Wave 2. If the operator requests changes instead, dispatch a
  new agent on top of that branch with the operator's exact words.

### 1.3 Standing obligations (check every wave)

1. **quick-xml advisory ignores** (RUSTSEC-2026-0194/0195 in
   `.cargo/audit.toml` + `deny.toml`): on every wave's gate run, try
   `cargo update -p plist && cargo audit`. When a fixed `plist` releases,
   take it and delete both ignores + the note in
   `docs/production-readiness-audit.md`.
2. **GitNexus index:** run `gitnexus-analyze-safe <repo-root>` after every
   merge to `main` (a hook will nag you; obey it).
3. **The operator's own daemon** runs on port **9743** from
   `target/release/sigillum` against the real `~/.sigillum`. NEVER kill it,
   never bind 9743, never point anything at the real `~/.sigillum`.
4. **Plan file hygiene:** checkbox + one work-log line per completed task,
   committed with the wave.

### 1.4 Remaining work (specs in release-1.0-plan.md §4)

C4; C7 merge; E0 (new, §4 Wave 3) + E1–E5; W1.1–W1.3, W2, W3.1–W3.5, W4,
W5, W6.1–W6.4, W7.1–W7.5, W8; F1–F7; G1–G5; H1–H3.

---

## 2. The execution method (proven in Waves A–1)

### 2.1 Roles

- **You (the coordinator)** plan waves, dispatch agents, review diffs,
  integrate, run gates, open/merge PRs, keep the plan file current. You do
  not hand-write code except one-line trivialities (note them when you do).
- **Implementation agents** (codex-exec type, one per task, isolated git
  worktrees) read their task spec from `docs/release-1.0-plan.md`,
  implement, run the task's targeted Verify commands, **commit on their
  worktree branch**, and report: worktree path, branch, `git log --oneline`,
  `diff --stat`, verification outputs, acceptance status.

### 2.2 Dispatch rules

1. **One task = one agent = one worktree.** Give each agent: the task ID,
   the instruction to read its spec from the plan, explicit file-scope
   limits (what it must NOT touch when a parallel agent owns those files),
   the exact verify commands, the commit message format with the
   `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` trailer, and
   the report format above.
2. **Concurrency cap: ~5 agents that compile Rust.** Each worktree has its
   own `target/`; more than ~5 parallel workspace builds thrashes the
   machine and causes spurious test timeouts. Docs-only agents are free.
3. **Any agent that starts a daemon or the app MUST use an isolated
   `SIGILLUM_BASE_DIR=$(mktemp -d)` and a unique port.** Put this in every
   prompt for tasks that run anything.
4. **Never let two release gates / smoke scripts run concurrently** without
   unique ports: `SIGILLUM_RUNTIME_SMOKE_PORT`, `SIGILLUM_BROWSER_SMOKE_PORT`.
   Best practice: run the full gate only when no agents are active.
5. **Assign non-overlapping file domains within a wave.** When two tasks
   must touch the same file, either combine them into one agent (as C1–C3+C6
   were) or sequence them. Known hotspots: `README.md`,
   `sigillum-api/src/response.rs`, `service/inventory/*`,
   `service/queue/*`, `docs/release-1.0-plan.md` (never let agents edit the
   plan file — the coordinator owns it).

### 2.3 Integration procedure (per wave)

1. **Verify your location first**: `git rev-parse --show-toplevel` must be
   the main checkout, branch `main`, clean. (A persistent `cd` into an
   agent worktree once caused a whole integration to run in the wrong
   checkout. Check every time.)
2. `git checkout -b wave/<n>-<slug>` from fresh `main`.
3. Merge task branches `git merge --no-ff --no-edit worktree-agent-<id>`,
   **API/signature-changing tasks LAST** (their fallout is fixed once, on
   top of everything else).
4. `cargo check --workspace --all-targets` → fix compile fallout from
   cross-worktree drift (e.g. a signature changed in one task, used by
   another). Trivial fixups you may do directly; anything larger →
   dispatch a fixup agent on the wave branch.
5. Reconcile cross-worktree docs (e.g. a doc created in one worktree that
   another task's report says it would have updated).
6. Update plan checkboxes + work log; commit `Integrate Wave <n>: ...`.
7. **Full gate, solo** (no agents running):
   `./scripts/check-release.sh`. Triage failures per §3.
8. Push, `gh pr create` with a per-task summary body, monitor CI (a poll
   loop on `gh pr checks`), fix Ubuntu-only failures (they are usually
   real — see §3.3), merge with `--merge` (merge commit preserves per-task
   history), delete the remote wave branch, pull main, re-index GitNexus,
   delete the merged `worktree-agent-*` branches.

### 2.4 Agent failure recovery

- **Agent dies mid-report** ("connection closed"): inspect its worktree
  (`git -C <worktree> status`, `log`). If work exists uncommitted, RESUME
  the same agent by id with "verify, commit, and re-report" — do not
  redispatch fresh (it loses context and redoes work).
- **Agent fails its task twice:** stop, record in the work log, surface to
  the operator (plan §0.1.7).
- **Codex sandbox can't `git commit`** (rare `.git/index.lock` permission
  issue): the wrapping agent commits after re-verifying — acceptable, note
  it in the report.

---

## 3. Gate-failure triage (in observed frequency order)

When `./scripts/check-release.sh` or CI fails, classify FIRST — do not
"fix" blindly:

1. **Load flake** (only when agents/gates ran concurrently): timeouts,
   401s from a colliding smoke daemon, `TimedOut` in daemon integration
   tests, browser-smoke waits expiring. **Action:** rerun the failing step
   solo with unique ports. If it passes solo, it was load; proceed. Known
   load-sensitive: `successful_restore_clears_session_and_restores_data_on_disk`,
   browser-smoke reveal/reauth waits.
2. **Real task defect / integration fallout:** compile errors, test
   failures that reproduce solo. Fix via fixup agent on the wave branch.
3. **Architecture budget** (`check-architecture.sh` line caps): split the
   module (see `daemon_api/args.rs` precedent). Never raise the cap.
4. **New RustSec advisory:** try the targeted `cargo update -p <crate>`
   first. Only if no fixed release exists anywhere upstream: scoped ignore
   in BOTH `.cargo/audit.toml` and `deny.toml` with a removal note, plus a
   paragraph in `docs/production-readiness-audit.md` and a work-log line.
   Vulnerabilities in code that parses untrusted input are NEVER ignored —
   stop and surface instead.
5. **fmt/clippy drift:** mechanical; fix and move on.

**Ubuntu-only CI failures are usually real.** All client loopback tests are
blanket-skipped on macOS (until E0 lands), so Ubuntu is the only leg that
executes them. Never dismiss an Ubuntu-only failure as environmental
without reproducing.

Local browser-smoke failure artifacts are deleted by the script's own
cleanup trap; CI uploads them as workflow artifacts (`gh run download`).
If you need them locally, re-run and copy them out before the script exits,
or capture from CI.

---

## 4. Remaining work, sequenced into waves

Follow the order. Within a wave, listed tasks run in parallel unless a
sequencing note says otherwise. After every wave: the §2.3 integration
procedure.

### Wave 2 — C4 + C7 merge (small, do immediately)

- **C4** (spec: plan §C4): solo agent. Note `scripts/check-desktop.sh`
  must also assert the ad-hoc signature (`codesign -dv`) per the C3 accept
  text. Wire into `check-release.sh`; measure the CI wall-clock impact.
- **C7 merge (only after operator sign-off):** from clean `main`:
  `git merge --no-ff worktree-agent-ae37914b9b627cdd5` on a `wave/2` or
  dedicated branch, full gate solo, PR, CI, merge. If C4 and C7 are both
  ready, one wave branch is fine (disjoint files).
- If the operator has NOT signed off C7 yet, do C4 alone and continue to
  Wave 3 — C7 can merge later; nothing depends on it except G5's docs.

### Wave 3 — Phase E + E0 (recovery semantics; prerequisite for W7)

Parallel: **E1** (queue state extension — read the E1 premise carefully:
EXTEND the existing model, wire-compat on existing state strings), **E3**
(maintenance summaries), **E4** (chain_id on allocations AND stealth
deposits — two stores), **E2** (crash-replay tests), **E0** (new task, add
it to the plan under Phase E before dispatching):

> **E0 — Replace the macOS blanket skip on client loopback tests.**
> `crates/sigillum-client/src/tests.rs` (and any sibling) marks loopback
> tests `#[cfg_attr(target_os = "macos", ignore = "sandbox blocks loopback
> bind")]`. Replace with a runtime probe: attempt a `127.0.0.1:0` bind at
> test start; skip (early-return with eprintln) only when the bind actually
> fails. Accept: `cargo test -p sigillum-client` on a normal macOS host
> RUNS the loopback tests; sandboxed environments still skip gracefully.
> Size S.

Sequenced: **E5** after E2 merges or on top of E2's branch (both live in
`crates/sigillum-daemon/tests/crash_recovery.rs`).

Conflict note: E1 and E3 both touch daemon service + `sigillum-api` —
different modules (queue vs maintenance), different response files; give
each an explicit not-your-files list.

### Wave 4 — W2, then W1 (foundations)

- **W2 first, SOLO** (typed domain model): it touches
  `sigillum-api/src/response.rs` and every inventory module — nothing else
  may run in parallel with it. The complete wire literals are enumerated in
  the plan task; the B4 roundtrip suite is the compat anchor — extend it,
  and prove pre-change JSON fixtures still load.
- Then **W1.1** (chain registry) ∥ **W1.2** (chain_id residue — read its
  verified premise: the field EXISTS; do not re-add). Then **W1.3**
  (multi-chain orchestration) after W1.1 merges.

### Wave 5 — Discovery + planner completion

Parallel across mostly disjoint files: **W3.1** (block cursors),
**W3.2** (batch e2e fixture only — decoding exists), **W3.3** (token
registry import), **W3.4** (NFT metadata pipeline — build on the EXISTING
cache structures), **W3.5** (last-activity — depends on W3.1's cursors, so
sequence it after W3.1 within the wave or give it the W3.1 branch as base),
**W6.2** (dynamic fees + `simulation_freshness_secs` policy field),
**W6.3** (hot floor/refill — migration must keep floor = target = 1 ETH),
**W6.4** (step ordering — must merge before Wave 6's UniV2 sub-task; test
with synthetic fixtures only).

Conflict note: W6.2/W6.3 both bump the treasury policy schema — combine
into one agent or sequence (one schema bump each, in order, with separate
migrations — do NOT parallel-edit `response/treasury.rs`).

### Wave 6 — Adapters + claims

- **W4** (DeFi exit adapters): one agent, one adapter per commit, order:
  ERC-4626 → Lido unwrap → Uniswap v2 LP (last; needs W6.4 merged).
  Aave v3 already exists — extend the pattern, don't rebuild it.
- **W5** (Merkle claim execution enablement) in parallel — different files
  (planner blockers + policy field).

### Wave 7 — W6.1, then W7 (STRICTLY SERIAL, highest risk)

- **W6.1** (fund_gas + the cross-party sponsor linkage rule — the README
  privacy-model text change is part of the task).
- Then **W7.1 → W7.2 → W7.3 → W7.4 → W7.5, one at a time, each its own
  PR** (not batched — this is the money-moving surface; CI per step).
  Every W7 task: gates default OFF, byte-identical behavior with gates off
  is an acceptance criterion, key-hygiene assertions in W7.3, typed
  confirmation in W7.2, gate-flip audit events in W7.1. Re-read plan §W7
  before each dispatch.

### Wave 8 — W8 (treasury automation)

Solo agent; hysteresis test is the acceptance heart.

### Wave 9 — Assurance (F)

- Parallel: **F1** (adversarial for receiving/treasury/chains/plans-enqueue),
  **F2** (nightly fuzz env), **F3** (chaos soak + in-flight-job assertion).
- **F5** (execution-path security review) after all W7 merges — dispatch as
  a review agent producing dispositions + regression tests.
- **F7** (0.1→1.0 upgrade fixture) after ALL schema-changing waves (i.e.,
  after Wave 8) and before G3. The fixture generator builds against the A3
  merge SHA (`fd2b35b`).
- **F4** and **F6** are HUMAN-GATED and run at RC time with Phase H — see
  §5.

### Wave 10 — Release engineering (G)

Order: **G1 ∥ G2** (docs), then **G3** (version bump — constrain the
straggler grep to workspace-owned manifests), then **G4** (release
workflow + THIRD-PARTY-NOTICES via pinned cargo-about; rc dry-run tag then
delete it), then **G5** (final doc sync — includes the D-17 residual-risk
statement, the roadmap "complete except swaps (D-13)" wording, and folding
this runbook + plan status to "executed").

### Phase H — Ship

Work through plan §H1 checklist literally. H2 tags only after every H1 box
including F4/F6/F7 evidence. H3 post-release bump.

---

## 5. Human-gated items — surface these, never fake them

| Item | What the human must provide |
|---|---|
| C7 sign-off | Operator approves the redesign screenshots (or requests changes) |
| F4 | Access to each supported target host (currently `mac-server`) for 3600s + chaos soaks at the RC SHA |
| F6 | Funded Sepolia + one L2-testnet account and RPC endpoints; receipts recorded honestly (mock-only status is acceptable for adapter/claim families) |
| H2 | The decision to tag and publish v1.0.0 |
| Any Decision Register change | Explicit operator sign-off recorded in the plan |

When you hit one, deliver everything up to the gate, state precisely what
is needed, and continue with non-blocked work.

---

## 6. Wave-completion checklist (copy per wave)

```
[ ] All wave agents reported; diffs reviewed against their task specs
[ ] Integration branch from fresh main (location verified)
[ ] Task branches merged, API-changers last; fallout fixed
[ ] Cross-worktree docs reconciled
[ ] Plan checkboxes + work-log lines updated and committed
[ ] cargo update -p plist && cargo audit (advisory-removal check)
[ ] Full gate green, run solo
[ ] PR opened with per-task summary; CI green both legs
[ ] Merged with merge commit; branches cleaned; main pulled
[ ] GitNexus re-indexed
```
