# Sigillum 1.0 Release Plan

**Status:** Active plan of record for the 1.0 release
**Baseline verified:** 2026-07-01, branch `feat/private-receiving-desktop` (2 commits ahead of `main`: `70a087b`, `1cda1f2`)
**Supersedes:** the open items in [catchup-plan.md](./catchup-plan.md) Phases 1–2 are absorbed into Phases D–E below; catchup Phase 3+ (wallet-management roadmap) stays deferred.

This document is written to be executed by an autonomous coding agent with no
additional context. Every task has an ID, explicit file paths, steps,
acceptance criteria, and verification commands. Do not improvise beyond what a
task says. When something is ambiguous, the Decision Register (§3) is the
answer; if it is not covered there, stop and ask a human.

---

## 0. Rules for the executing agent

Read this section before every work session.

### 0.1 Ground rules

1. **Never start work on a red baseline.** Before starting any task, run
   `./scripts/check-release.sh` from the repo root. If it fails for reasons
   unrelated to your task, stop and report; do not "fix" unrelated failures as
   a side effect.
2. **One task per branch.** Branch naming: `task/<task-id>-<short-slug>`
   (e.g. `task/b2-expect-burndown`). One task = one PR into `main`.
3. **Full gate before every PR.** `./scripts/check-release.sh` must pass
   locally before you open a PR. During iteration use the targeted commands
   listed in §0.3.
4. **Docs move with code.** If a task changes behavior, update the affected
   docs (`README.md`, `docs/architecture.md`, `PRODUCTION_READINESS.md`,
   `docs/production-readiness-audit.md`) in the same PR. A PR that makes the
   docs lie is a failed PR.
5. **Track progress in this file.** When a task is done, check its box in the
   Master Checklist (§5), and append one line to the Work Log (§6):
   `YYYY-MM-DD <task-id> <commit-sha> <one-line result>`.
6. **Stop conditions.** Stop and report to a human instead of proceeding if:
   a task fails twice; an acceptance criterion cannot be met as written; you
   would need to change a Decision Register entry; you would need to touch
   vault file formats, key handling, or unlock flows in a way a task does not
   explicitly call for; or a dependency upgrade is required to proceed.
7. **Never weaken fail-closed behavior.** Corruption handling, policy
   blockers, linkage blocking, and typed-confirmation destructive flows must
   stay fail-closed. If a test is hard to pass, fix the test setup, not the
   safety behavior.
8. **Never commit secrets** — no API keys, no seed material, no provider URLs
   containing credentials, not even in test fixtures. Use obviously fake
   values (`0xdead...`, `test-token`).
9. **Scope discipline.** §2.3 lists things that are explicitly NOT part of
   1.0. Do not implement them, even partially, even if they seem adjacent.

### 0.2 Environment prerequisites

- macOS or Linux host. macOS is required for desktop bundle tasks (C2, C4, G4).
- Rust toolchain is pinned by `rust-toolchain.toml` (1.88.0) — do not upgrade it.
- Node.js + npm for the daemon UI (`crates/sigillum-daemon/ui`).
- `cargo-audit` 0.22.1 and `cargo-deny` 0.19.4 (the versions CI pins in
  `.github/workflows/ci.yml`).
- A Chromium-family browser for `scripts/check-browser-smoke.sh`; if the host
  has none, export `SIGILLUM_SKIP_BROWSER_SMOKE=1` and note it in the PR.

### 0.3 Verification commands

Targeted (fast iteration):

```bash
cargo test -p <crate>                                  # single crate
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix crates/sigillum-daemon/ui run typecheck
npm --prefix crates/sigillum-daemon/ui test
npm --prefix crates/sigillum-daemon/ui run build       # regenerates checked-in app.js/styles.css
```

Full gates (before PR):

```bash
./scripts/check-release.sh          # the release contract; runs everything below and more
./scripts/check-adversarial.sh      # adversarial/fuzz pass (included in check-release)
./scripts/check-runtime-smoke.sh    # real daemon runtime smoke (included in check-release)
```

Long-running assurance (Phase F only):

```bash
SIGILLUM_SOAK_SECONDS=3600 SIGILLUM_SOAK_INTERVAL_SECONDS=30 \
  SIGILLUM_SOAK_RECEIPT=target/readiness/<receipt-name>.json \
  ./scripts/check-local-soak.sh
```

### 0.4 Task template

Every task below follows this shape. "Depends on" gates ordering; tasks with
no unmet dependencies may run in parallel on separate branches.

> **Goal** — what must be true afterward.
> **Files** — where the work lands (not exhaustive, but the center of mass).
> **Steps** — ordered instructions.
> **Accept** — checkable acceptance criteria.
> **Verify** — commands that must pass.
> **Size** — S (<half day), M (about a day), L (multi-day).

---

## 1. What 1.0 means

Sigillum 1.0 is the **Local-First Operator Console**, shipped as a versioned,
tagged, reproducible release with binary artifacts. It is the product boundary
the docs already claim — hardened, packaged, and versioned so the claim is
real.

**1.0 deliverables:**

1. `v1.0.0` annotated git tag on `main`, with all workspace crates at version
   `1.0.0`.
2. A GitHub Release with: macOS desktop app bundle (`.dmg`/`.app`), macOS and
   Linux CLI binaries, and a changelog excerpt.
3. `./scripts/check-release.sh` green on a clean clone of the tagged commit,
   on both Ubuntu and macOS CI runners.
4. The desktop app (`sigillum-desktop`) is release-quality: real icons,
   bundling enabled, covered by the release gate, documented.
5. Operator-surface parity is closed or explicitly decided: every daemon route
   family has a UI surface, a CLI surface, or a recorded API-only decision in
   `docs/operator-surface-parity.md`.
6. Automation/recovery semantics (queue states, restart replay, maintenance
   summaries, destructive-flow recovery) are explicit and test-backed.
7. Assurance evidence: adversarial gate covers the receiving/treasury
   surfaces, a chaos-mode soak passes, and per-host soak receipts exist for
   every host named "supported."
8. A `CHANGELOG.md`, a stability policy (`docs/stability.md`), and readiness
   docs that describe exactly what shipped.

### 2.1 In scope (already implemented; 1.0 hardens and ships it)

- Local daemon + embedded web UI, vault, compartments, passphrase/FIDO2
  unlock, snapshots, audit, transit ops.
- Ethereum stealth custody flows, EVM provider profiles, seed/xpub wallet
  profiles (including imported external xpubs and custom paths), watch
  addresses, bounded inventory/risk/plan slices, treasury console + policy
  guardrails, receiving console, deposits/queue/maintenance.
- `sigillum-gateway` as a local-sidecar payment preview surface (unchanged
  positioning).
- `sigillum-desktop` Tauri v2 shell.
- CLI incl. the `sigillum api` daemon bridge.

### 2.2 Current state — verified baseline (2026-07-01)

Facts an executor can rely on without re-deriving:

- **CI:** one workflow, `.github/workflows/ci.yml`. Matrix
  `ubuntu-latest` + `macos-latest`, Rust pinned 1.88.0, runs
  `./scripts/check-release.sh` (line 43). Triggers: push to `main` and
  `codex/**`, all PRs, nightly cron `0 6 * * *`. No release/tag/publish
  workflow exists. Linux step installs only `pkg-config libudev-dev`.
- **Desktop:** `crates/sigillum-desktop` is a workspace member, so
  `cargo check/test/clippy --workspace` cover it, but nothing bundles it:
  `tauri.conf.json` has `bundle.active: false`, a 105-byte placeholder
  `icons/icon.png`, a 133-byte placeholder `dist/index.html`, no signing
  config, and no tests. `src/main.rs` is 381 lines; it boots the daemon
  in-process on an ephemeral loopback port and opens a webview at that origin.
- **Versioning:** `[workspace.package] version = "0.1.0"` in the root
  `Cargo.toml` (line 19); internal `workspace.dependencies` entries also pin
  `version = "0.1.0"` (lines 28–33). No crate sets `publish = false`. No git
  tags exist. No `CHANGELOG.md`.
- **Code debt:** zero `TODO`/`FIXME`/`unimplemented!`/`todo!()` markers. One
  real "not yet" marker:
  `crates/sigillum-daemon/src/service/inventory/treasury.rs:336`
  ("allocations do not yet persist chain_id"). Five `#[allow(dead_code)]`
  sites. Sixteen production (non-test) `expect("...")` calls across 11 files
  (full list in task B2).
- **CLI:** hand-parsed args in `crates/sigillum-cli/src/main.rs:48-78` (no
  clap). The `sigillum api` bridge (`crates/sigillum-cli/src/daemon_api.rs`)
  covers profiles, deposits, inventory, discovery, risk, plans, receiving,
  treasury, queue list/process, maintenance, and session ops. Route families
  with NO CLI coverage: `transit/*`, `evm/*`, `wallets/*` (all 12),
  daemon-side `api-keys/*`+`secrets/*` CRUD, `fido2/*` admin, `compartment`
  list/add/remove/init, manual `queue/enqueue/*`, `setup/reset`,
  `auth/capability`. Daemon routes are all registered in
  `crates/sigillum-daemon/src/routes/mod.rs` (~100 routes, lines 214–623).
- **Tests:** integration suites exist for daemon (5 files under
  `crates/sigillum-daemon/tests/`), gateway (2 + support), CLI smoke, core
  proptest fuzz. The UI has `crates/sigillum-daemon/ui/test/ui-smoke.test.ts`
  run via `node --test`. No `tests/` dir in: `sigillum`, `sigillum-api`,
  `sigillum-client`, `sigillum-desktop`, `sigillum-fido2`,
  `sigillum-generator`, `sigillum-sdk`, `sigillum-server` (some have inline
  unit tests).
- **Release gate env toggles:** only `SIGILLUM_SKIP_BROWSER_SMOKE=1` (skips
  browser smoke) and `TMPDIR`. Everything else is unconditional.
- **GLM convergence:** `docs/glm/sigillum-architecture-converged.md` declared
  the receiving/desktop architecture ready with exactly two blocking items,
  both positioning/documentation; commit `1cda1f2` was made to address them
  (verify in task A1).

### 2.3 Explicitly OUT of scope for 1.0 — do not build these

Everything in `docs/wallet-management-roadmap.md` beyond what already exists,
specifically including:

- non-EVM chains (Bitcoin/UTXO, Solana, Tron, Cosmos)
- token registries/indexers, NFT metadata/spam classification, valuation
  providers
- DeFi protocol exit adapters, airdrop/reward **claim execution**
- queued/automatic **execution** of consolidation plans (plan export remains
  the execution handoff boundary)
- Permit2 expiration-aware scoring, external spender registries, dynamic fee
  policy
- any remote/hosted/multi-host/internet-facing mode, SSE streams, remote
  audit aggregation
- crates.io publishing (see D-1 in the Decision Register)
- Windows support

If a task seems to require one of these, you have misread the task. Stop.

---

## 3. Decision Register

These decisions are already made. Do not re-litigate them; implement them.
Changing any of these requires explicit human sign-off recorded here.

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | **No crates.io publish at 1.0.** All 12 crates get `publish = false`. 1.0 ships as source + GitHub Release binaries. crates.io is a post-1.0 evaluation. | Publishing 12 interdependent crates is a large, irreversible surface (name claims, semver duty on internals). The product is a local-first app, not a library ecosystem, and the README's library framing is corrected in B1. |
| D-2 | **macOS is the supported desktop platform at 1.0.** Linux desktop is compile-only best effort (workspace check keeps it building); Windows is unsupported. | Dev + soak evidence is on macOS (`mac-server`); Tauri Linux packaging/signing would add a platform matrix without a user. |
| D-3 | **Desktop bundles ship unsigned by default.** Signing/notarization is env-gated and optional (task C3); the Gatekeeper caveat is documented. | No Apple Developer credentials are assumed. Fail-open on signing would block release on external accounts. |
| D-4 | **No external penetration test for 1.0.** The readiness claim stays scoped to "source-verified local-first release gate" exactly as `docs/production-readiness-audit.md` words it today. | The audit doc already draws this boundary honestly; 1.0 does not widen the claim, so it does not need the wider evidence. |
| D-5 | **CLI parity scope:** implement CLI commands only where scripting is plausible — `transit`, read-only `evm` helpers, `wallets` export/derive/check/generate, `compartment list`. Everything else UI-covered gets a recorded decision, not code (task D2). `wallets` sign/send stay API+UI-only at 1.0. | Signing/broadcast from shell history is an operator hazard; the daemon UI and API cover it. The Operator Gate requires a surface or a recorded decision — not CLI everywhere. |
| D-6 | **`block_cross_party_linkage` and all policy guardrails stay fail-closed opt-in exactly as shipped.** 1.0 makes no privacy-model changes. | The GLM convergence settled the linkage claim scope; reopening it is post-1.0 work. |
| D-7 | **Rust stays pinned at 1.88.0** unless a RustSec advisory forces a bump; if it does, that is a stop condition (§0.1.6). | Toolchain drift invalidates the soak/readiness evidence chain. |
| D-8 | **Treasury allocations get `chain_id` persistence in 1.0** (task E4) with a schema-versioned migration defaulting existing records to chain id `1`. | It is the only "not yet" marker in the codebase; shipping 1.0 with a known silent mainnet-only assumption in persisted state is a correctness trap for 1.x. |

---

## 4. Phases and tasks

Phase order: **A → (B ∥ C ∥ D) → E → F → G → H.**
B, C, D are independent of each other after A merges. E depends on B. F
depends on C and E (except F4, which runs with Phase H — see its note).
G depends on everything before it except F4. H is the final gate.

---

### Phase A — Land the desktop branch on `main`

#### A1 — Verify GLM convergence blockers are resolved

- **Goal:** Confirm the two blocking items from the GLM architecture
  convergence are actually addressed on this branch before merging.
- **Files:** `docs/glm/sigillum-architecture-converged.md` (read),
  `README.md`, `docs/architecture.md` (possibly edit).
- **Steps:**
  1. Read `docs/glm/sigillum-architecture-converged.md` and extract the two
     blocking items (both are positioning/documentation items).
  2. For each, verify the corresponding doc text exists on this branch
     (commit `1cda1f2` was intended to close them — check the README
     "Privacy Model — Scope and Limitations" section and
     `docs/architecture.md` "Privacy & Linkage Model" section).
  3. If either item is not fully addressed, make the minimal doc edit that
     closes it. No code changes in this task.
- **Accept:** each blocking item can be mapped to specific doc text on the
  branch; any gap is closed by doc-only edits.
- **Verify:** `./scripts/check-release.sh` (docs changes can still break the
  whitespace check). **Size:** S.

#### A2 — Open the PR and make CI green on both OSes

- **Goal:** `feat/private-receiving-desktop` passes CI as a PR against `main`.
- **Files:** `.github/workflows/ci.yml` (contingency only).
- **Steps:**
  1. Push the branch and open a PR to `main`.
  2. Watch both matrix legs. **Known risk:** the desktop crate has never been
     compiled by the Ubuntu leg (push triggers only cover `main`/`codex/**`),
     and Tauri's `wry` needs system WebKit at compile time.
  3. **Contingency (apply only if the Ubuntu leg fails on missing system
     libraries):** extend the Linux-only apt step in
     `.github/workflows/ci.yml` (currently `pkg-config libudev-dev`, lines
     23–25) with the Tauri v2 Linux build deps:
     `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
     librsvg2-dev libxdo-dev libssl-dev`.
  4. If it still fails after that, stop and report (per §0.1.6) — do not
     start excluding crates from the workspace to force green.
- **Accept:** both CI legs green on the PR.
- **Verify:** CI status checks. **Size:** S (M if contingency fires).

#### A3 — Merge to `main`

- **Goal:** desktop + receiving console are on `main`; post-merge CI is green.
- **Steps:** merge the PR (no squash-vs-merge preference recorded; follow
  repo habit of single-commit messages), confirm the push-triggered CI run on
  `main` passes, delete the feature branch.
- **Accept:** `main` contains `70a087b` + `1cda1f2` content; CI green on `main`.
- **Size:** S.

---

### Phase B — Workspace hygiene (debt burn-down)

#### B1 — Set `publish = false` everywhere; fix README library framing

- **Goal:** implement D-1 so an accidental `cargo publish` is impossible and
  the README stops implying a crates.io release.
- **Files:** all 12 `crates/*/Cargo.toml`; `README.md` (Quick Start "As a
  library" section, line ~102, and Feature Flags example, lines ~296–308).
- **Steps:**
  1. Add `publish = false` to the `[package]` section of every crate manifest.
  2. Replace the README dependency examples (`sigillum = "0.1"` and the
     `version = "0.1"` feature-flag examples) with git-dependency form:
     `sigillum = { git = "https://github.com/<org>/<repo>", tag = "v1.0.0" }`
     (read the actual origin URL from `git remote get-url origin`), plus one
     sentence stating crates are not published to crates.io.
- **Accept:** `grep -L 'publish = false' crates/*/Cargo.toml` returns empty;
  README contains no bare crates.io-style version dependency examples.
- **Verify:** `cargo publish --dry-run -p sigillum-core` fails with
  "publish is disabled"; `./scripts/check-release.sh`. **Size:** S.

#### B2 — Production `expect()` burn-down

- **Goal:** no production code path can panic on a recoverable error. The 16
  known production `expect("...")` sites (test code is exempt):

  | File | Count |
  |------|-------|
  | `crates/sigillum-gateway/src/main.rs` | 4 |
  | `crates/sigillum-gateway/src/config.rs` (line ~101, bind-addr parse) | 1 |
  | `crates/sigillum-gateway/src/db.rs` | 1 |
  | `crates/sigillum-gateway/src/webhooks.rs` | 1 |
  | `crates/sigillum-core/src/utils.rs` (incl. line ~102 Argon2id derivation) | 2 |
  | `crates/sigillum-daemon/src/lib.rs` | 2 |
  | `crates/sigillum-daemon/src/state.rs` | 1 |
  | `crates/sigillum-daemon/src/service/helpers.rs` (line ~115) | 1 |
  | `crates/sigillum-fido2/src/crypto.rs` (line ~133) | 1 |
  | `crates/sigillum-desktop/src/main.rs` (line ~115, daemon URL parse) | 1 |
  | `crates/sigillum-client/src/lib.rs` | 1 |

- **Rules:**
  - **Library code** (core, fido2, client, daemon lib/state/service): convert
    to the crate's typed error enum and propagate. Never swap `expect` for
    `unwrap` or a silent default.
  - **Binary entrypoints** (`gateway/src/main.rs`, `desktop/src/main.rs`):
    startup-time failures may print a clear one-line error and exit non-zero
    instead of panicking.
  - **Provably-infallible sites** (e.g. parsing a URL the same function just
    formatted): if conversion is disproportionate, the call may stay, but the
    message must state the invariant (e.g.
    `expect("loopback URL built from a bound port is always valid")`) and a
    one-line comment must justify it. Hard cap: at most 4 such justified
    sites may remain in the whole workspace.
- **Steps:** fix file by file; add a regression test for each newly
  propagated error path where the error is reachable (bad config value, bad
  address, etc.).
- **Accept:** an audit of
  `grep -rn --include='*.rs' 'expect("' crates | grep -v tests` shows only
  test code plus ≤4 justified sites with invariant messages.
- **Verify:** `cargo test --workspace`; `./scripts/check-release.sh`.
  **Size:** M.

#### B3 — Resolve the `#[allow(dead_code)]` sites

- **Goal:** each allow is either deleted (with its dead code) or replaced by
  a live caller. Sites: `crates/sigillum-daemon/src/json_store.rs:138`,
  `crates/sigillum-daemon/src/audit_log.rs:1439` and `:1464`,
  `crates/sigillum-daemon/src/audit_db.rs:43`,
  `crates/sigillum-daemon/src/service/transaction_policy.rs:9`.
- **Steps:** for each, check `git log -S` for why it was added; if the code
  has no planned caller in this plan's task list, delete it; if a Phase D/E
  task will use it, leave it and note the consuming task ID in a comment.
- **Accept:** every remaining `#[allow(dead_code)]` names the task that will
  consume it; all others removed.
- **Verify:** `cargo clippy --workspace --all-targets -- -D warnings`.
  **Size:** S.

#### B4 — Minimal test floors for untested crates

- **Goal:** every crate that ships behavior has at least a smoke-level test
  target so `cargo test --workspace` exercises it.
- **Files:** `crates/sigillum-api`, `crates/sigillum-client`,
  `crates/sigillum-sdk`, `crates/sigillum-server`, `crates/sigillum`,
  `crates/sigillum-generator`.
- **Steps:**
  1. `sigillum-api`: add a `tests/roundtrip.rs` that serde-roundtrips one
     representative request and response type per module (session, profiles,
     deposits, queue, treasury, receiving) — assert
     `deserialize(serialize(x)) == x` and that unknown fields are tolerated
     where the contract says they are.
  2. Others: if inline unit tests already exist (client has
     `src/tests.rs`), confirm they run and skip; otherwise add one
     construct-and-exercise smoke test (e.g. `sigillum` meta-crate re-export
     compiles and `FileVault::new` works against a temp dir).
  3. Desktop is handled separately in C5 — skip it here.
- **Accept:** `cargo test --workspace` reports at least one executed test per
  crate above (or an inline module already covers it — record which in the PR).
- **Verify:** `cargo test --workspace`. **Size:** M.

---

### Phase C — Desktop app productization

#### C1 — Real icon set

- **Goal:** replace the 105-byte placeholder icon with a real, generated icon
  set.
- **Files:** `crates/sigillum-desktop/icons/`,
  `crates/sigillum-desktop/tauri.conf.json`.
- **Steps:**
  1. Produce a 1024×1024 master PNG. If no design asset exists anywhere in
     the repo, generate a minimal wordmark: dark background `#0d1117`, the
     glyph "S" or a seal/sigil monogram, high contrast. Keep it sober — this
     is a security tool, not a toy.
  2. Install tauri-cli (`cargo install tauri-cli --version '^2' --locked`)
     and run `cargo tauri icon <master.png>` from
     `crates/sigillum-desktop/` to generate the platform icon set.
  3. Reference the generated set in `tauri.conf.json` `bundle.icon`.
- **Accept:** `icons/` contains generated `.icns`, `.ico`, and PNG size
  variants; no 105-byte placeholder remains.
- **Verify:** C4's bundle build shows the icon on the `.app`. **Size:** S.

#### C2 — Enable bundling

- **Goal:** `tauri.conf.json` produces a distributable macOS bundle.
- **Files:** `crates/sigillum-desktop/tauri.conf.json`,
  `crates/sigillum-desktop/dist/index.html`.
- **Steps:**
  1. Set `bundle.active: true`, `bundle.targets: ["app", "dmg"]`.
  2. Confirm `identifier` stays `com.sigillum.desktop` and `productName`
     stays `Sigillum`.
  3. Add a `version` field sourced to match the workspace version (0.1.0 now;
     G3 bumps it to 1.0.0 — leave a comment-adjacent note in the PR, JSON has
     no comments).
  4. Replace the 133-byte placeholder `dist/index.html` with a minimal static
     "Sigillum is starting…" page (the real UI is served by the embedded
     daemon; this page is only a fallback shell target).
- **Accept:** `cargo tauri build` on macOS emits `.app` and `.dmg` under
  `target/release/bundle/`.
- **Verify:** open the built `.app`; it must reach the daemon UI
  (setup or unlock screen) in a native window. **Size:** S.

#### C3 — Optional, env-gated signing/notarization (D-3)

- **Goal:** unsigned by default; signing activates only when credentials are
  present in the environment.
- **Files:** `crates/sigillum-desktop/tauri.conf.json` (or env-driven Tauri
  signing config), `docs/deployment.md`.
- **Steps:**
  1. Wire Tauri v2's standard signing env vars
     (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
     `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` for notarization) so that
     builds sign when set and stay unsigned when not. Do not fail the build
     when they are absent.
  2. Document in `docs/deployment.md`: how unsigned builds behave under
     Gatekeeper (right-click → Open, or `xattr -d com.apple.quarantine`),
     and how to provide signing credentials.
- **Accept:** `cargo tauri build` succeeds with no signing env set; docs
  describe both paths.
- **Verify:** build without env vars on a clean shell. **Size:** S.

#### C4 — Desktop check script wired into the release gate

- **Goal:** the release contract actually exercises the desktop crate beyond
  workspace compilation.
- **Files:** new `scripts/check-desktop.sh`, `scripts/check-release.sh`.
- **Steps:**
  1. Create `scripts/check-desktop.sh` following the style of the existing
     check scripts (strict mode, clear step banners):
     - always: `cargo build -p sigillum-desktop --locked`
     - macOS only, and skippable via `SIGILLUM_SKIP_DESKTOP_BUNDLE=1`:
       `cargo tauri build --debug` and assert the bundle path exists
     - non-macOS: print an explicit "bundle build skipped (unsupported OS,
       see D-2)" line so the skip is visible in logs, not silent.
  2. Invoke it from `scripts/check-release.sh` after the browser smoke step.
  3. Update `.github/workflows/ci.yml` if the macOS leg needs tauri-cli
     installed (`cargo install tauri-cli --version '^2' --locked`, cached by
     rust-cache).
- **Accept:** `./scripts/check-release.sh` runs the desktop step on both
  OSes (build everywhere, bundle on macOS); skip toggles documented in
  `docs/production-readiness-audit.md` alongside `SIGILLUM_SKIP_BROWSER_SMOKE`.
- **Verify:** full gate locally on macOS; CI green both legs. **Size:** M.

#### C5 — Desktop testability: extract and test the boot helpers

- **Goal:** the untestable 381-line `main.rs` gets its logic seams under test.
- **Files:** `crates/sigillum-desktop/src/main.rs`, new
  `crates/sigillum-desktop/src/lib.rs`.
- **Steps:**
  1. Move the non-Tauri logic out of `main.rs` into `lib.rs` functions:
     ephemeral port selection, daemon-readiness wait (TCP connect loop with
     timeout), daemon URL construction, and the lock-on-close decision
     helpers.
  2. Unit-test each: readiness-wait returns `Ok` once a listener accepts and
     times out cleanly when nothing listens; URL construction round-trips a
     bound port; port selection returns a bindable port.
  3. This also resolves the desktop `expect()` from B2 if not already done.
- **Accept:** `cargo test -p sigillum-desktop` runs ≥4 meaningful unit tests;
  `main.rs` shrinks to Tauri wiring.
- **Verify:** `cargo test -p sigillum-desktop`; manual launch still works
  (`cargo run -p sigillum-desktop`). **Size:** M.

#### C6 — Desktop documentation

- **Goal:** an operator can install, launch, lock, and reset the desktop app
  from docs alone.
- **Files:** `README.md` (Desktop app section), `docs/deployment.md`.
- **Steps:** document: install from `.dmg`; shared data dir with the CLI
  (`~/.sigillum`, `SIGILLUM_BASE_DIR`); single-instance behavior; tray lock
  state and "Lock now"; close-to-tray auto-lock; quit zeroization; unsigned
  build caveat (from C3); troubleshooting (port in use, daemon fails to
  start).
- **Accept:** a fresh reader can go from `.dmg` to unlocked console without
  reading source.
- **Verify:** whitespace/docs checks in the gate. **Size:** S.

---

### Phase D — Operator-surface parity closure

#### D1 — CLI commands for scriptable route families (D-5)

- **Goal:** implement the CLI half of Decision D-5.
- **Files:** new modules under `crates/sigillum-cli/src/daemon_api/`
  (`transit.rs`, `evm.rs`, `wallets.rs`), dispatch wiring in
  `crates/sigillum-cli/src/daemon_api.rs` (match arms, lines ~59–115), help
  text, `crates/sigillum-cli/tests/cli_smoke.rs`.
- **Commands to add** (mirror the existing `daemon_api/queue.rs` bridge
  pattern; all take `--url`/`--session` like their siblings):
  - `sigillum api transit encrypt|decrypt|hmac`
  - `sigillum api evm nonce|balance|erc20-balance|fees|estimate`
    (read-only; **no** `broadcast` — recorded as API/UI-only in D2)
  - `sigillum api wallets xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check`
    (**no** sign/send variants — D-5)
  - `sigillum api compartment list`
- **Steps:** one route family per commit; each command needs: a client
  method in `crates/sigillum-client` if missing, JSON output consistent with
  sibling commands, an adversarial-parse case and a happy-path case in
  `cli_smoke.rs`, and (where the daemon test harness supports it) a
  route-level test in `crates/sigillum-daemon/tests/daemon_service.rs`.
- **Accept:** all listed commands exist, documented in the README CLI
  paragraph; no sign/send/broadcast CLI paths exist.
- **Verify:** `cargo test -p sigillum-cli`, `cargo test -p sigillum-client`,
  full gate. **Size:** L.

#### D2 — Operator-surface parity matrix

- **Goal:** a single decision-of-record document mapping every daemon route
  family to its operator surface.
- **Files:** new `docs/operator-surface-parity.md`; link it from
  `PRODUCTION_READINESS.md` (Structural Readiness Gates section) and
  `docs/architecture.md`.
- **Steps:**
  1. Enumerate route families from `crates/sigillum-daemon/src/routes/mod.rs`
     (lines 214–623): root/health, session/auth, biometric, diagnostics/
     selfcheck/maintenance/audit/setup-reset/backup, compartment, api-keys,
     secrets, generate/store, transit, evm, profiles (evm/eth-stealth/
     eth-xpub/eth-seed), wallets, inventory, discovery, risk, plans,
     treasury, receiving, deposits, queue, fido2.
  2. For each family produce a table row: routes → UI? → CLI? → decision.
  3. Record the explicit API-or-UI-only decisions with one-line rationale:
     `evm/broadcast` (hazard: shell history + no plan review), `wallets`
     sign/send (same), `fido2/*` admin (requires physical touch; UI flow),
     `compartment add/remove/init` (destructive; UI typed-confirmation flow),
     `queue/enqueue/*` manual routes (maintenance auto-enqueues; UI covers
     manual), `setup/reset` (destructive; UI typed-confirmation only),
     `auth/capability` (internal session plumbing), daemon-side
     `api-keys`/`secrets` CRUD (UI covers; local CLI covers the standalone
     FileVault).
- **Accept:** every route registered in `routes/mod.rs` appears in exactly
  one row; no row has all of UI=no, CLI=no, decision=none.
- **Verify:** cross-check row count against the router file. **Size:** M.

---

### Phase E — Automation, recovery, and policy completion

(Absorbs catchup-plan Phase 2. Depends on Phase B.)

#### E1 — Explicit queue job state model

- **Goal:** queue jobs expose explicit states — `blocked`, `deferred`,
  `retryable`, `operator_action_required` — instead of implicit retry
  behavior.
- **Files:** `crates/sigillum-daemon/src/service/queue/state.rs`,
  `processing.rs`, `payloads.rs`; DTOs in
  `crates/sigillum-api/src/request/queue.rs` and
  `crates/sigillum-api/src/response/queue.rs`; UI queue view; CLI
  `daemon_api/queue.rs` list output; persisted schema version for
  `queue.json`.
- **Steps:**
  1. Add the state enum to the persisted job document **with a schema-version
     bump and a migration** that maps legacy jobs (follow the existing
     schema-versioned JSON pattern noted in `docs/architecture.md`).
  2. Classify existing retry transitions into the new states in
     `processing.rs`/`state.rs`; `operator_action_required` must never be
     auto-retried.
  3. Surface the state in API responses, the UI queue table, and
     `sigillum api queue list`.
  4. Tests: one per state transition, plus a legacy-document migration test.
- **Accept:** every queued job always reports one of the explicit states;
  restart preserves states; migration test passes against a fixture of the
  pre-change JSON shape.
- **Verify:** `cargo test -p sigillum-daemon queue`, UI tests, full gate.
  **Size:** L.

#### E2 — Restart/replay guarantees, test-backed

- **Goal:** kill-mid-write is provably safe for `profiles.json`,
  `deposits.json`, `queue.json`.
- **Files:** `crates/sigillum-daemon/tests/crash_recovery.rs`,
  `crates/sigillum-daemon/src/json_store.rs`.
- **Steps:** add tests that simulate each interruption window the atomic
  writer has (temp file written but not renamed; renamed but `.bak` stale;
  live file truncated/corrupted) and assert: restore from `.bak`, quarantine
  of the broken file next to restored state, and recovery telemetry counters
  present in diagnostics.
- **Accept:** each interruption window has a named test; behavior matches the
  documented restore/quarantine contract in the README Storage Model section.
- **Verify:** `cargo test -p sigillum-daemon --test crash_recovery`.
  **Size:** M.

#### E3 — Maintenance summaries with cause separation

- **Goal:** a maintenance run reports, separately: deposits refreshed,
  sweeps enqueued, jobs executed, and failures **by cause**
  (provider error / policy block / insufficient gas / validation), instead of
  an aggregate count.
- **Files:** maintenance service module in
  `crates/sigillum-daemon/src/service/`, response DTO in `sigillum-api`,
  UI maintenance view, `sigillum api maintenance run` output.
- **Accept:** UI and CLI show the categorized summary; one test per failure
  cause category.
- **Verify:** `cargo test -p sigillum-daemon maintenance`, UI tests.
  **Size:** M.

#### E4 — Persist `chain_id` on treasury receive allocations (D-8)

- **Goal:** remove the known limitation at
  `crates/sigillum-daemon/src/service/inventory/treasury.rs:336`.
- **Steps:** add `chain_id` to the allocation document with schema-version
  bump; migration defaults legacy records to `1` (EVM mainnet) and marks them
  operator-visible as "assumed mainnet"; expose the field through the
  treasury DTOs, UI receive-allocations view, and
  `sigillum api treasury receive-list`; update the comment at that line.
- **Accept:** new allocations persist the chain id of the profile that
  derived them; legacy fixture migrates with the default + marker; the "not
  yet" comment is gone.
- **Verify:** `cargo test -p sigillum-daemon treasury`. **Size:** M.

#### E5 — Destructive-flow recovery completion

- **Goal:** vault init/remove, snapshot restore, and compartment
  replacement/recovery are journaled, fail-closed, and each has an
  interruption test — turning the pending-operation journal into full
  recovery per `docs/architecture.md` "Architectural Priorities" item 2.
- **Files:** daemon lifecycle/storage service modules, pending-operation
  journal, `crates/sigillum-daemon/tests/crash_recovery.rs`,
  `docs/backup.md`.
- **Steps:** for each flow: assert a journal entry exists before mutation;
  simulate interruption after journal write and verify startup reconciliation
  either completes or cleanly rolls back (never half-state); document the
  guarantee per flow in `docs/backup.md`.
- **Accept:** each of the three flows has interruption tests for
  pre-mutation, mid-mutation, and post-mutation crash points; docs state the
  guarantee.
- **Verify:** `cargo test -p sigillum-daemon --test crash_recovery`.
  **Size:** L.

---

### Phase F — Assurance expansion

(Depends on C and E.)

#### F1 — Adversarial coverage for receiving/treasury surfaces

- **Goal:** the surfaces added by the desktop branch get the same boundary
  treatment as older routes.
- **Files:** `crates/sigillum-daemon/tests/adversarial_api.rs`.
- **Steps:** add rejection cases for `/api/receiving/*` and `/api/treasury/*`
  (and any D1-added route usage): malformed JSON, wrong content type, missing/
  bad bearer token, invalid addresses, oversized party labels, negative/
  overflow amounts, policy-violating destination inputs.
- **Accept:** every receiving/treasury route has ≥3 adversarial cases;
  `./scripts/check-adversarial.sh` stays green.
- **Verify:** `cargo test -p sigillum-daemon --test adversarial_api`.
  **Size:** M.

#### F2 — Nightly deep-fuzz in CI

- **Goal:** the nightly cron run works harder than PR runs without slowing
  PRs.
- **Files:** `.github/workflows/ci.yml`.
- **Steps:** on `github.event_name == 'schedule'`, export
  `SIGILLUM_ADVERSARIAL_PROPTEST_CASES=1024` (PR/push runs keep the default
  256).
- **Accept:** nightly log shows 1024 cases; PR runtime unchanged.
- **Verify:** trigger via `workflow_dispatch` if added, or wait one night.
  **Size:** S.

#### F3 — Chaos mode for the soak harness

- **Goal:** the soak proves crash recovery, not just uptime.
- **Files:** `scripts/check-local-soak.sh`.
- **Steps:**
  1. Add `SIGILLUM_SOAK_CHAOS=1` mode: every Nth iteration
     (`SIGILLUM_SOAK_CHAOS_EVERY`, default 10), `kill -9` the daemon,
     restart it, and require the next iteration's doctor + canary checks to
     pass; count kill/restart cycles in the receipt JSON.
  2. Run a 600-second chaos soak locally; keep the receipt.
- **Accept:** chaos run passes with ≥2 kill/restart cycles recorded in the
  receipt; recovery telemetry visible in diagnostics after restart.
- **Verify:** `SIGILLUM_SOAK_CHAOS=1 SIGILLUM_SOAK_SECONDS=600 ./scripts/check-local-soak.sh`.
  **Size:** M.

#### F4 — Release-commit soak receipts per supported host

- **Goal:** the audit doc's host-coverage requirement is satisfied for the
  actual 1.0 commit.
- **Note on ordering:** this is the one Phase F task that runs *late* — it
  depends on G3+G5 (the receipts must reference the release-candidate SHA)
  and is executed as part of preparing H1. Do F1–F3 in phase order; schedule
  F4 with Phase H.
- **Steps:** after G3 lands (version bump), run on each host that
  `docs/production-readiness-audit.md` names as supported (currently
  `mac-server`): a 3600s standard soak and a 600s chaos soak against the
  release-candidate commit; record receipt filenames, commit SHA, host, OS
  version in `docs/production-readiness-audit.md`.
- **Accept:** audit doc lists a fresh receipt per supported host at the RC
  SHA.
- **Verify:** receipts exist and reference the RC SHA. **Size:** M (mostly
  wall-clock).

---

### Phase G — Release engineering

(Depends on A–F. Do these in order.)

#### G1 — CHANGELOG.md

- **Goal:** a `CHANGELOG.md` at repo root in Keep-a-Changelog format.
- **Steps:** create with an `[1.0.0] - <release date>` section summarizing,
  at feature granularity (not commit granularity): vault/compartments/unlock,
  daemon + embedded console, stealth custody + deposits/queue/maintenance,
  inventory/risk/plans slices, treasury + receiving + linkage policy,
  gateway sidecar, desktop app, CLI surface, release gate. Add an
  `[Unreleased]` section on top. Source material: `git log --oneline`,
  README Status section, this plan.
- **Accept:** file exists, links to the v1.0.0 tag, no placeholder text.
- **Size:** S.

#### G2 — Stability policy (`docs/stability.md`)

- **Goal:** 1.0 states what "stable" commits to.
- **Steps:** write `docs/stability.md` declaring: **stable at 1.0** —
  `sigillum-api` DTO wire shapes, daemon HTTP route paths and semantics, CLI
  command syntax, on-disk formats (vault files, compartments, schema-versioned
  JSON stores — evolvable only via versioned migration), `sigillum-core`
  public traits; **explicitly unstable** — daemon internal module layout, UI
  markup/DOM, gateway (remains a preview surface), `sigillum-sdk`/`-server`
  facades, anything in §2.3. SemVer applies to the workspace version from
  1.0.0 on. Link from README Development section.
- **Accept:** doc exists and is linked; claims match D-1..D-8.
- **Size:** S.

#### G3 — Version bump 0.1.0 → 1.0.0

- **Goal:** the workspace is version 1.0.0 everywhere, consistently.
- **Files:** root `Cargo.toml` (`[workspace.package] version`, line 19, AND
  every internal `version = "0.1.0"` pin in `[workspace.dependencies]`,
  lines 28–33 region), `Cargo.lock` (regenerate via `cargo check`),
  `crates/sigillum-desktop/tauri.conf.json` `version`,
  `crates/sigillum-daemon/ui/package.json` `version` (if present), any
  remaining `0.1` strings in README.
- **Steps:** bump, then `grep -rn '"0\.1' --include='*.toml' --include='*.json' .`
  (excluding `target/`, `node_modules/`, `Cargo.lock` noise) to catch stragglers.
- **Accept:** grep shows no stale internal 0.1 versions;
  `cargo metadata --no-deps --format-version 1` reports 1.0.0 for all 12
  crates.
- **Verify:** full gate. **Size:** S.

#### G4 — Release workflow

- **Goal:** pushing tag `v1.0.0` produces the release artifacts
  automatically.
- **Files:** new `.github/workflows/release.yml`.
- **Steps:** create a workflow triggered on `push: tags: ['v*']`:
  1. Job 1 `verify` (matrix ubuntu/macos, mirroring `ci.yml` setup):
     `./scripts/check-release.sh`.
  2. Job 2 `artifacts-macos` (needs `verify`): install tauri-cli,
     `cargo tauri build` in `crates/sigillum-desktop` (signing env optional
     per C3), `cargo build --release -p sigillum-cli`; upload `.dmg`, `.app`
     (zipped), and the `sigillum` CLI binary.
  3. Job 3 `artifacts-linux` (needs `verify`):
     `cargo build --release -p sigillum-cli`; upload the binary.
  4. Job 4 `release` (needs 2+3): create a GitHub Release from the tag, body
     = the `[1.0.0]` section of `CHANGELOG.md`, attach artifacts, generate
     `SHA256SUMS` over all artifacts and attach it.
- **Accept:** a test tag (`v1.0.0-rc.1`) run produces all artifacts and a
  draft release; the rc tag/release are deleted afterward.
- **Verify:** the rc dry run. **Size:** M.

#### G5 — Readiness docs final sync

- **Goal:** every readiness/status doc describes 1.0 exactly.
- **Files:** `PRODUCTION_READINESS.md`, `docs/production-readiness-audit.md`,
  `README.md` (Status), `docs/catchup-plan.md` (mark absorbed phases),
  `docs/architecture.md`.
- **Steps:** update verdicts to "1.0 released scope = Local-First Operator
  Console"; keep the D-4 claim wording (source-verified, local-first, no
  external pen test, no internet-facing claim); record the F4 receipts;
  point "Current Plan Of Record" at this file; state that the wallet
  management roadmap resumes post-1.0.
- **Accept:** no doc contradicts another; the §1 deliverables list is
  satisfiable by reading the docs alone.
- **Size:** S.

---

### Phase H — Final gate and ship

#### H1 — Release candidate verification (all must pass, in order)

- [ ] Fresh clone of `main` at the RC commit into a clean directory;
      `./scripts/check-release.sh` passes there (not in your dirty checkout).
- [ ] CI green on the RC commit, both legs.
- [ ] F4 soak receipts reference the RC commit SHA.
- [ ] Desktop `.dmg` from G4's rc dry run installs and reaches the unlock
      screen on a machine (or fresh account) without a dev toolchain.
- [ ] `sigillum doctor` passes on each supported host.
- [ ] CHANGELOG date filled in; G5 docs merged.

#### H2 — Tag and release

```bash
git checkout main && git pull --ff-only
./scripts/check-release.sh
git tag -a v1.0.0 -m "Sigillum 1.0.0 — Local-First Operator Console"
git push origin v1.0.0
# then: watch .github/workflows/release.yml, verify artifacts + SHA256SUMS,
# publish the draft GitHub Release.
```

#### H3 — Post-release

- [ ] Bump workspace version to `1.1.0-dev` on `main` (same file set as G3)
      in a follow-up PR, adding an `[Unreleased]` CHANGELOG section.
- [ ] Open a post-1.0 planning issue: crates.io decision (D-1 revisit),
      wallet-management roadmap re-entry point (catchup Phase 3), Linux
      desktop support demand check.

---

## 5. Master checklist

Phase A — Land the desktop branch
- [ ] A1 GLM convergence blockers verified/closed
- [ ] A2 PR green on both CI legs
- [ ] A3 Merged to main

Phase B — Workspace hygiene
- [ ] B1 publish=false + README dependency framing
- [ ] B2 expect() burn-down (≤4 justified sites)
- [ ] B3 dead_code allows resolved
- [ ] B4 test floors for untested crates

Phase C — Desktop productization
- [ ] C1 real icon set
- [ ] C2 bundling enabled (.app/.dmg)
- [ ] C3 env-gated signing, unsigned default documented
- [ ] C4 check-desktop.sh in the release gate
- [ ] C5 boot helpers extracted + tested
- [ ] C6 desktop docs

Phase D — Operator-surface parity
- [ ] D1 CLI: transit, evm read-only, wallets read/derive, compartment list
- [ ] D2 docs/operator-surface-parity.md complete

Phase E — Automation & recovery
- [ ] E1 explicit queue job states + migration
- [ ] E2 kill-mid-write replay tests
- [ ] E3 categorized maintenance summaries
- [ ] E4 chain_id persisted on allocations
- [ ] E5 destructive-flow recovery complete

Phase F — Assurance
- [ ] F1 adversarial coverage: receiving/treasury
- [ ] F2 nightly deep-fuzz
- [ ] F3 chaos soak mode
- [ ] F4 RC soak receipts per supported host

Phase G — Release engineering
- [ ] G1 CHANGELOG.md
- [ ] G2 docs/stability.md
- [ ] G3 version bump to 1.0.0
- [ ] G4 release workflow + rc dry run
- [ ] G5 readiness docs final sync

Phase H — Ship
- [ ] H1 RC verification checklist
- [ ] H2 v1.0.0 tagged, artifacts published
- [ ] H3 post-release bump + planning issue

## 6. Work log

(append one line per completed task: `YYYY-MM-DD <task-id> <commit-sha> <result>`)
