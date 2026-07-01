# Sigillum 1.0 Release Plan

**Status:** Active plan of record for the 1.0 release (rev 2 — wallet management is IN scope)
**Baseline verified:** 2026-07-01, branch `feat/private-receiving-desktop` (commits `70a087b`, `1cda1f2` ahead of `main`)
**Supersedes:** [catchup-plan.md](./catchup-plan.md) Phases 1–3 are absorbed into Phases D–E and W1–W8 below. The
[wallet-management-roadmap.md](./wallet-management-roadmap.md) product target is **part of 1.0** (EVM scope — see D-9);
catchup Phase 4 (remote/platform) stays out.

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
   (e.g. `task/w7-2-plan-step-payloads`). One task = one PR into `main`.
3. **Full gate before every PR.** `./scripts/check-release.sh` must pass
   locally before you open a PR. During iteration use the targeted commands
   listed in §0.3.
4. **Docs move with code.** If a task changes behavior, update the affected
   docs (`README.md`, `docs/architecture.md`, `PRODUCTION_READINESS.md`,
   `docs/production-readiness-audit.md`, `docs/wallet-management-roadmap.md`)
   in the same PR. A PR that makes the docs lie is a failed PR.
5. **Feature finish line.** Any task that adds daemon capability is done only
   when it has ALL of: API DTOs in `sigillum-api`, daemon route + service
   logic, client method in `sigillum-client`, an operator surface (UI, CLI,
   or a recorded API-only decision in `docs/operator-surface-parity.md`),
   schema-versioned persistence with migration, tests (happy path, auth
   failure, validation failure, recovery), and docs. This is the repo's
   established Development Standard — no partial landings.
6. **Track progress in this file.** When a task is done, check its box in the
   Master Checklist (§5), and append one line to the Work Log (§6):
   `YYYY-MM-DD <task-id> <commit-sha> <one-line result>`.
7. **Stop conditions.** Stop and report to a human instead of proceeding if:
   a task fails twice; an acceptance criterion cannot be met as written; you
   would need to change a Decision Register entry; you would need to touch
   vault file formats, key handling, or unlock flows in a way a task does not
   explicitly call for; or a dependency upgrade is required to proceed.
8. **Never weaken fail-closed behavior.** Corruption handling, policy
   blockers, linkage blocking, simulation gates, execution gates, and
   typed-confirmation destructive flows must stay fail-closed. If a test is
   hard to pass, fix the test setup, not the safety behavior. Every new
   execution capability in this plan defaults to OFF and is enabled by an
   explicit operator opt-in.
9. **Never commit secrets** — no API keys, no seed material, no provider URLs
   containing credentials, not even in test fixtures. Use obviously fake
   values (`0xdead...`, `test-token`).
10. **Scope discipline.** §2.3 lists things that are explicitly NOT part of
    1.0. Do not implement them, even partially, even if they seem adjacent.

### 0.2 Environment prerequisites

- macOS or Linux host. macOS is required for desktop bundle tasks (C2, C4, G4).
- Rust toolchain is pinned by `rust-toolchain.toml` (1.88.0) — do not upgrade it.
- Node.js + npm for the daemon UI (`crates/sigillum-daemon/ui`).
- `cargo-audit` 0.22.1 and `cargo-deny` 0.19.4 (the versions CI pins in
  `.github/workflows/ci.yml`).
- A Chromium-family browser for `scripts/check-browser-smoke.sh`; if the host
  has none, export `SIGILLUM_SKIP_BROWSER_SMOKE=1` and note it in the PR.
- Task F6 (testnet receipts) needs a human to supply funded Sepolia/L2-testnet
  accounts and RPC endpoints; it is flagged as human-in-the-loop.

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

> **Goal** — what must be true afterward.
> **Files** — where the work lands (center of mass, not exhaustive).
> **Steps** — ordered instructions.
> **Accept** — checkable acceptance criteria.
> **Verify** — commands that must pass.
> **Size** — S (<half day), M (about a day), L (multi-day), XL (multi-week).

"Depends on" gates ordering; tasks with no unmet dependencies may run in
parallel on separate branches.

---

## 1. What 1.0 means

Sigillum 1.0 is the **local-first wallet-management workstation**: the
Local-First Operator Console (daemon + vault + CLI + gateway sidecar + desktop
shell) **plus the completed wallet-management product** defined in
[wallet-management-roadmap.md](./wallet-management-roadmap.md), for EVM
networks. The roadmap's own completion bar is the 1.0 bar:

> a recovered or imported wallet can move from "unknown" to "inventoried" to
> "reviewed plan" to "executed or intentionally ignored" without leaving the
> local Sigillum control plane.

**1.0 deliverables:**

1. `v1.0.0` annotated git tag on `main`, all workspace crates at `1.0.0`.
2. A GitHub Release with: macOS desktop app bundle (`.dmg`/`.app`), macOS and
   Linux CLI binaries, and a changelog excerpt.
3. `./scripts/check-release.sh` green on a clean clone of the tagged commit,
   on both Ubuntu and macOS CI runners.
4. The desktop app is release-quality: real icons, bundling enabled, covered
   by the release gate, documented.
5. **Wallet management complete for EVM (roadmap phases 1–9):** multi-network
   EVM discovery with a chain registry; discovery completion (block-range
   checkpoints, full ERC-1155, local token registries, NFT metadata/spam
   heuristics, last-activity); the bounded DeFi exit-adapter set; Merkle
   claim execution; planner completion (gas top-ups, dynamic fees,
   policy-driven refill, step ordering); **controlled, policy-gated queue
   execution of consolidation plans** (today: export-only); treasury
   automation (overflow/refill). All fail-closed opt-in.
6. Operator-surface parity closed or explicitly decided for every daemon
   route family (`docs/operator-surface-parity.md`).
7. Automation/recovery semantics (queue states, restart replay, maintenance
   summaries, destructive-flow recovery) explicit and test-backed.
8. Assurance evidence: adversarial coverage for receiving/treasury AND
   execution surfaces, chaos-mode soak, per-host soak receipts, and testnet
   execution receipts for the core execution families (D-14).
9. `CHANGELOG.md`, a stability policy (`docs/stability.md`), and readiness
   docs that describe exactly what shipped.

### 2.1 In scope

Everything under §1, on EVM networks. The gateway keeps its local-sidecar
preview positioning. Non-goals are in §2.3.

### 2.2 Current state — verified baseline (2026-07-01)

Facts an executor can rely on without re-deriving. Platform facts first, then
the wallet subsystem.

**Platform:**

- **CI:** one workflow, `.github/workflows/ci.yml`: matrix
  `ubuntu-latest` + `macos-latest`, Rust pinned 1.88.0, runs
  `./scripts/check-release.sh` (line 43). Push triggers only `main` and
  `codex/**`; all PRs; nightly cron. Linux step installs only
  `pkg-config libudev-dev`. No release/tag/publish workflow.
- **Desktop:** `crates/sigillum-desktop` is a workspace member (compiled by
  workspace checks) but not bundled: `tauri.conf.json` has
  `bundle.active: false`, placeholder 105-byte icon and 133-byte
  `dist/index.html`, no signing config, no tests; `src/main.rs` (381 lines)
  boots the daemon in-process on an ephemeral loopback port.
- **Versioning:** workspace version `0.1.0` (root `Cargo.toml` line 19;
  internal pins lines 28–33). No `publish = false` anywhere, no tags, no
  `CHANGELOG.md`.
- **Code debt:** no TODO/FIXME/`unimplemented!` markers. Sixteen production
  `expect("...")` sites across 11 files (list in B2). Five
  `#[allow(dead_code)]` sites (list in B3).
- **CLI:** hand-parsed args (`crates/sigillum-cli/src/main.rs:48-78`, no
  clap). `sigillum api` bridge covers profiles/deposits/inventory/discovery/
  risk/plans/receiving/treasury/queue-list-process/maintenance/session. NOT
  covered: `transit/*`, `evm/*`, `wallets/*`, daemon `api-keys`/`secrets`
  CRUD, `fido2/*` admin, `compartment` add/remove/init, manual
  `queue/enqueue/*`, `setup/reset`, `auth/capability`.
- **Release gate env toggles:** only `SIGILLUM_SKIP_BROWSER_SMOKE=1` and
  `TMPDIR`.

**Wallet subsystem (survey of 2026-07-01):**

- **Chain model:** chains are a bare `chain_id: u64` on provider profiles
  (`crates/sigillum-api/src/response/profiles.rs:13`). A descriptive
  `ChainProfile` struct exists (`crates/sigillum-api/src/response.rs:689`,
  held in `WalletInventoryState.chain_profiles`,
  `crates/sigillum-daemon/src/inventory.rs:21`) but there is **no routing
  chain registry**. Mainnet hardcodes: treasury allocations
  (`service/inventory/treasury.rs:337` and `:354`), canonical Permit2
  fallback (`service/inventory/permit2_discovery.rs:9`). Selfcheck already
  verifies provider chain id via `eth_chainId`
  (`service/selfcheck.rs:186-207`).
- **Discovery jobs:** resumable and checkpointed **by derivation index only**
  (`service/inventory/checkpoints.rs:5`;
  `WalletDiscoveryCheckpoint` at `sigillum-api/src/response.rs:618-632`;
  statuses `running|completed|canceled|resume_requested`). No block-range
  cursors — log scans are bounded one-shots. One scan-job kind,
  parameterized by wallet-family constants (`service/inventory.rs:63-65`)
  and boolean `discover_*` flags (`sigillum-api/src/request/inventory.rs`).
- **Inventory model:** asset kinds and address classifications are
  **strings, not enums** (`asset_kind: String` at `response.rs:589`;
  `classifications: Vec<String>` at `response.rs:574`; values produced in
  `service/inventory/observation.rs:328-402`). Timestamps are scan-time
  only (`first_seen_at_unix`/`last_checked_at_unix`) — **no on-chain
  last-activity**. **No valuation fields.**
- **Planner:** step `action: String` values (`planner.rs:169-180`):
  `sweep_native`, `sweep_erc20`, `sweep_nft`, `revoke_erc20_approval`,
  `revoke_permit2_allowance`, `revoke_nft_operator_approval`,
  `revoke_approval`, `exit_defi_position`, `claim_reward`, `review_asset`.
  Gates: `status` (`review_required|blocked|approved`), `signer_status`,
  `simulation_status`, `blockers`, `linkage_warnings`, `auto_eligible`,
  `approved` (`response.rs:789-827`). Export formats `call_manifest` and
  `safe_tx_builder` (`service/inventory/export.rs:131`, Safe batches at
  `:247`). **No gas top-up or swap step types exist.**
- **Execution boundary:** `QueueJobPayload`
  (`sigillum-api/src/response/queue.rs:7-97`) has 4 stealth variants
  (constructed in `service/queue/payloads.rs`) plus 3 `EthSeed*` variants
  that are **hard-blocked** in `service/queue/processing.rs:189-193`
  ("seed-wallet queue execution is not enabled yet"). **Consolidation plan
  steps have no enqueue path at all** — export-only; blocked steps are
  excluded from export (`export.rs:147-149`). Claim execution is disabled by
  the `claim_execution_disabled` blocker (`planner.rs:136-157`).
- **Control branch: already implemented.** Sponsor/hot/treasury at
  `m/44'/60'/{account}'/1/{0,1,2}`
  (`sigillum-core/src/ethereum_xpub.rs:16,250,270`; mapping in
  `service/profiles/seed_wallets.rs:195-206`; addresses on
  `EthSeedWalletProfile`). Planner routes hot-vs-treasury with a
  **hardcoded 1 ETH refill target** (`planner.rs:357`).
- **Treasury policy:** `TreasuryPolicy`
  (`sigillum-api/src/response/treasury.rs:137-158`): `enabled`,
  `allowed_destinations`, `max_step_native_wei_hex`,
  `max_plan_native_wei_hex`, `require_simulation`,
  `allow_raw_digest_signing`, `block_cross_party_linkage`. **No
  hot-floor/overflow/refill or execution-enable fields.** Quorum is
  satisfied at unlock time only (`service/lifecycle.rs:67-68,115-123`);
  no execution-time quorum surface.
- **Fees/simulation:** dynamic EIP-1559 estimation exists
  (`service/evm/fees.rs:8-28` via `eth_feeHistory` +
  `eth_maxPriorityFeePerGas`), used in sends only when
  `estimate_fees == Some(true)`; otherwise static profile fees. `eth_call`
  preflight (`service/evm/preflight.rs:12-27`,
  `service/inventory/preflight.rs:32-247`) covers sweeps (native/erc20/nft),
  all three revoke kinds, `claim_reward` (`merkle-distributor-v1`), and
  `exit_defi_position` (**adapter `aave-v3-withdraw` exists**,
  `service/inventory/defi_adapters.rs:8`).
- **Test harness:** in-process mock JSON-RPC server in
  `crates/sigillum-daemon/tests/daemon_service.rs:27-205` (fakes
  `eth_chainId`, balances, fee history, `eth_call`, `eth_getLogs`,
  `eth_sendRawTransaction`). **No anvil/foundry or live-testnet tooling
  anywhere.**
- **UI views** (`crates/sigillum-daemon/ui/src/views/`): `fido2`, `inventory`
  (scans, chain profiles, watch book, risk, plans), `journey`, `operations`
  (deposits/queue/maintenance), `receiving`, `selfcheck`, `session`, `setup`,
  `treasury`, `walletManager`, `wallets`.

### 2.3 Explicitly OUT of scope for 1.0 — do not build these

- **Non-EVM chains** — Bitcoin/UTXO, Solana, Tron, Cosmos (roadmap phase 10;
  deferred by D-9).
- **Swap execution and DEX routing** — no swap step type, no router
  integration (D-13). Long-tail/dust assets keep the existing
  `review_asset` fallback.
- **Price/valuation feeds** — no fiat valuation, no NFT floor pricing (D-16).
- **External runtime registries/feeds** — token lists, spender reputation,
  and spam lists are operator-imported local files only (D-15).
- **Lido withdrawal-queue exits** — wstETH→stETH unwrap only (D-11).
- Any remote/hosted/multi-host/internet-facing mode, SSE streams, remote
  audit aggregation.
- crates.io publishing (D-1). Windows support (D-2). External pen test (D-4).

If a task seems to require one of these, you have misread the task. Stop.

---

## 3. Decision Register

These decisions are made. Do not re-litigate them; implement them. Changing
any requires human sign-off recorded here.

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | **No crates.io publish at 1.0.** All crates get `publish = false`; 1.0 ships as source + GitHub Release binaries. | Publishing 12 interdependent crates is a large irreversible surface; the product is a local-first app. |
| D-2 | **macOS is the supported desktop platform at 1.0.** Linux desktop compile-only; Windows unsupported. | Dev + soak evidence is macOS; no Linux desktop user yet. |
| D-3 | **Desktop bundles ship unsigned by default;** signing/notarization env-gated. | No Apple Developer credentials assumed. |
| D-4 | **No external penetration test for 1.0.** Claim stays "source-verified local-first release gate". | The audit doc already draws this boundary honestly. |
| D-5 | **CLI parity for scriptable families only** — `transit`, read-only `evm`, `wallets` export/derive/check/generate, `compartment list`, plus the wallet-management surfaces already bridged. `wallets` sign/send and `evm broadcast` stay API+UI-only. | Signing/broadcast from shell history is an operator hazard; UI/API cover it. |
| D-6 | **All policy guardrails stay fail-closed opt-in.** Every NEW execution capability (plan execution, claim execution, treasury automation, gas top-ups) defaults OFF behind its own `TreasuryPolicy` opt-in, surfaced in onboarding like `block_cross_party_linkage`. | Execution is the highest-risk surface 1.0 adds; defaults must be safe. |
| D-7 | **Rust stays pinned at 1.88.0** unless a RustSec advisory forces a bump (that is a stop condition). | Toolchain drift invalidates the evidence chain. |
| D-8 | **Treasury allocations get `chain_id` persistence** with a schema-versioned migration defaulting legacy records to `1`. | Prerequisite for multi-network EVM (W1). |
| D-9 | **Wallet-management 1.0 scope = roadmap phases 1–9 on EVM networks.** Phase 10 (non-EVM) is 1.x. | The roadmap's own completion sentence is chain-family-agnostic and achievable on EVM; its Multi-Chain Direction section sequences non-EVM explicitly after EVM completion ("only then"). |
| D-10 | **Built-in chain registry entries at 1.0:** Ethereum (1), Base (8453), Arbitrum One (42161), OP Mainnet (10), Polygon PoS (137). Other EVM chains via operator-defined custom entries. | Matches the roadmap's named networks; custom entries keep it open without shipping untested defaults. |
| D-11 | **DeFi exit-adapter set at 1.0:** Aave v3 withdraw (exists), generic ERC-4626 redeem, Uniswap v2 LP removeLiquidity, Lido wstETH unwrap. Nothing else; other positions surface as `review_asset`. | Standard interfaces with dominant TVL coverage; bounded and testable. Uniswap v3 NFT positions and Lido withdrawal queue are disproportionate for 1.0. |
| D-12 | **Claim execution at 1.0 = `merkle-distributor-v1` adapter only**, gated by simulation pass + explicit step approval + risk-catalog review + a policy opt-in. All other claim types remain review/export-only. | The simulation slice for this adapter already exists; it is the only claim shape safe to automate. |
| D-13 | **No swap step type at 1.0.** Planner does not emit swap steps; dust keeps the `review_asset` fallback. | The roadmap marks swaps "optionally"; DEX routing/slippage is a large adversarial surface orthogonal to the completion bar. |
| D-14 | **Execution testing bar:** every execution family needs mock-RPC integration tests (mandatory) AND a recorded public-testnet receipt (Sepolia + one L2 testnet) for native sweep, ERC-20 sweep, revoke, and gas top-up. Adapter exits and Merkle claims: mock-mandatory, testnet best-effort (contract availability permitting). | Real broadcasts need real-network evidence; contract-dependent families should not block on deploying testnet contracts. |
| D-15 | **Registries stay local.** Token lists, spender labels, spam heuristics: operator-imported files + the existing risk catalog. No runtime fetching of external feeds. | Preserves the local-first/no-phone-home boundary; RPC endpoints remain the only outbound surface. |
| D-16 | **No valuation at 1.0.** Holdings show raw amounts; no fiat/floor pricing. | Price feeds add an external dependency + phone-home surface for cosmetic value; the completion bar doesn't need it. |
| D-17 | **Quorum model at 1.0 = unlock-time compartment threshold** (already implemented in `service/lifecycle.rs`). Execution adds per-plan explicit approval + policy gates, not a second quorum ceremony. Documented as the "quorum authority" the catchup plan calls for. | The threshold ceremony already exists at unlock; duplicating it at execution time adds friction without a distinct trust boundary on a single-operator machine. |
| D-18 | **Wire-format compatibility rule for W2:** enum conversions must serialize to the exact current strings; unknown inbound values map to an explicit `Other(String)`/unknown variant, never a parse failure. | The API contract is stable-in-place; typed safety must not break existing clients or persisted state. |

---

## 4. Phases and tasks

Phase order:

```
A → (B ∥ C ∥ D) → E → (W1 ∥ W2) → (W3 ∥ W4 ∥ W5 ∥ W6) → W7 → W8 → F → G → H
```

- B, C, D independent after A merges. E depends on B.
- W1 (chains) and W2 (typed model) start after E; they are each other's peers.
- W3–W6 need W1+W2 (W5 also needs E1). W7 needs E1, E2, W2, W6. W8 needs W7.
- F depends on C, E, W7 (F4/F6 run with Phase H — see their notes).
- G depends on everything except F4/F6. H is the final gate.

---

### Phase A — Land the desktop branch on `main`

#### A1 — Verify GLM convergence blockers are resolved

- **Goal:** confirm the two blocking items from
  `docs/glm/sigillum-architecture-converged.md` (both positioning/
  documentation) are addressed on this branch before merging.
- **Steps:** read the converged doc; map each blocking item to specific text
  in the README "Privacy Model — Scope and Limitations" and
  `docs/architecture.md` "Privacy & Linkage Model" sections (commit `1cda1f2`
  was intended to close them); close any residual gap with doc-only edits.
- **Accept:** each blocking item maps to concrete doc text.
- **Verify:** `./scripts/check-release.sh`. **Size:** S.

#### A2 — Open the PR and make CI green on both OSes

- **Goal:** `feat/private-receiving-desktop` passes CI as a PR against `main`.
- **Steps:** push, open PR, watch both matrix legs. **Known risk:** the
  Ubuntu leg has never compiled the desktop crate; Tauri's `wry` needs system
  WebKit. **Contingency (only if Ubuntu fails on missing system libs):**
  extend the apt step in `.github/workflows/ci.yml` (lines 23–25) with
  `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
  librsvg2-dev libxdo-dev libssl-dev`. If still red, stop and report.
- **Accept:** both CI legs green. **Size:** S (M if contingency fires).

#### A3 — Merge to `main`

- **Steps:** merge, confirm post-merge CI on `main`, delete the branch.
- **Accept:** `main` contains the desktop/receiving work; CI green.
  **Size:** S.

---

### Phase B — Workspace hygiene (debt burn-down)

#### B1 — `publish = false` everywhere; fix README library framing

- **Files:** all 12 `crates/*/Cargo.toml`; `README.md` (Quick Start library
  section and Feature Flags examples).
- **Steps:** add `publish = false` to every crate; replace crates.io-style
  dependency examples with git-dependency form (read the URL from
  `git remote get-url origin`) plus a sentence stating crates are not on
  crates.io.
- **Accept:** `grep -L 'publish = false' crates/*/Cargo.toml` is empty; no
  bare version examples remain in README.
- **Verify:** `cargo publish --dry-run -p sigillum-core` refuses; full gate.
  **Size:** S.

#### B2 — Production `expect()` burn-down

- **Goal:** no production path panics on a recoverable error. The 16 known
  production sites:

  | File | Count |
  |------|-------|
  | `crates/sigillum-gateway/src/main.rs` | 4 |
  | `crates/sigillum-gateway/src/config.rs` (~L101, bind-addr parse) | 1 |
  | `crates/sigillum-gateway/src/db.rs` | 1 |
  | `crates/sigillum-gateway/src/webhooks.rs` | 1 |
  | `crates/sigillum-core/src/utils.rs` (incl. ~L102 Argon2id) | 2 |
  | `crates/sigillum-daemon/src/lib.rs` | 2 |
  | `crates/sigillum-daemon/src/state.rs` | 1 |
  | `crates/sigillum-daemon/src/service/helpers.rs` (~L115) | 1 |
  | `crates/sigillum-fido2/src/crypto.rs` (~L133) | 1 |
  | `crates/sigillum-desktop/src/main.rs` (~L115, URL parse) | 1 |
  | `crates/sigillum-client/src/lib.rs` | 1 |

- **Rules:** library code → typed error propagation (never `unwrap` or a
  silent default); binary entrypoints may print one clear line and exit
  non-zero; provably-infallible sites may remain only with an
  invariant-stating message + justifying comment, hard cap 4 in the
  workspace.
- **Accept:** audit shows only test code plus ≤4 justified sites.
- **Verify:** `cargo test --workspace`; full gate. **Size:** M.

#### B3 — Resolve the `#[allow(dead_code)]` sites

- **Sites:** `crates/sigillum-daemon/src/json_store.rs:138`,
  `audit_log.rs:1439`/`:1464`, `audit_db.rs:43`,
  `service/transaction_policy.rs:9`.
- **Steps:** for each, `git log -S` for origin; delete dead code with no
  consumer in this plan, or annotate with the consuming task ID (W7 will
  likely consume `transaction_policy.rs`).
- **Accept:** every remaining allow names its consuming task.
- **Verify:** clippy with `-D warnings`. **Size:** S.

#### B4 — Minimal test floors for untested crates

- **Steps:** `sigillum-api` gets `tests/roundtrip.rs` serde-roundtripping one
  request+response type per module (session, profiles, deposits, queue,
  treasury, receiving, inventory) — this file becomes the wire-compat anchor
  W2 extends. Others (`sigillum`, `sigillum-client`, `sigillum-sdk`,
  `sigillum-server`, `sigillum-generator`): confirm inline tests run or add
  one construct-and-exercise smoke test. Desktop is C5's job.
- **Accept:** `cargo test --workspace` executes ≥1 test per crate above.
  **Size:** M.

---

### Phase C — Desktop app productization

#### C1 — Real icon set

- **Steps:** produce a 1024×1024 master PNG (sober wordmark if no asset
  exists: dark `#0d1117`, seal/sigil monogram); install tauri-cli
  (`cargo install tauri-cli --version '^2' --locked`); run
  `cargo tauri icon <master.png>` in `crates/sigillum-desktop/`; reference
  the generated set in `tauri.conf.json` `bundle.icon`.
- **Accept:** generated `.icns`/`.ico`/PNGs present; placeholder gone.
  **Size:** S.

#### C2 — Enable bundling

- **Steps:** `bundle.active: true`, `bundle.targets: ["app", "dmg"]`; keep
  identifier `com.sigillum.desktop`; add a `version` field matching the
  workspace; replace the 133-byte `dist/index.html` with a minimal static
  fallback page.
- **Accept:** `cargo tauri build` on macOS emits `.app` + `.dmg`; the app
  reaches the daemon UI. **Size:** S.

#### C3 — Env-gated signing/notarization (D-3)

- **Steps:** wire Tauri v2 standard signing env vars (sign when present,
  unsigned when absent, never fail on absence); document both paths and the
  Gatekeeper caveat in `docs/deployment.md`.
- **Accept:** clean-shell build succeeds unsigned. **Size:** S.

#### C4 — Desktop check script in the release gate

- **Steps:** new `scripts/check-desktop.sh`: always
  `cargo build -p sigillum-desktop --locked`; macOS-only (skippable via
  `SIGILLUM_SKIP_DESKTOP_BUNDLE=1`) `cargo tauri build --debug` + bundle
  path assertion; explicit skip line on other OSes. Wire into
  `check-release.sh` after browser smoke; add tauri-cli install to the CI
  macOS leg if needed; document the toggle in the audit doc.
- **Accept:** gate runs the desktop step on both OSes. **Size:** M.

#### C5 — Desktop testability: extract and test boot helpers

- **Steps:** move port selection, daemon-readiness wait, URL construction,
  and lock-on-close decision logic from `main.rs` into a new
  `crates/sigillum-desktop/src/lib.rs`; unit-test each (readiness-wait Ok on
  listener, clean timeout otherwise; URL round-trip; bindable port).
- **Accept:** `cargo test -p sigillum-desktop` runs ≥4 meaningful tests;
  `main.rs` shrinks to Tauri wiring. **Size:** M.

#### C6 — Desktop documentation

- **Steps:** README + `docs/deployment.md`: install from `.dmg`; shared
  `~/.sigillum` data dir + `SIGILLUM_BASE_DIR`; single-instance; tray lock
  state; close-to-tray auto-lock; quit zeroization; unsigned caveat;
  troubleshooting.
- **Accept:** a fresh reader goes `.dmg` → unlocked console without reading
  source. **Size:** S.

---

### Phase D — Operator-surface parity closure

#### D1 — CLI commands for scriptable route families (D-5)

- **Files:** new `crates/sigillum-cli/src/daemon_api/{transit,evm,wallets}.rs`,
  dispatch arms in `daemon_api.rs` (~L59–115), help text,
  `tests/cli_smoke.rs`; client methods in `sigillum-client` where missing.
- **Commands:** `sigillum api transit encrypt|decrypt|hmac`;
  `sigillum api evm nonce|balance|erc20-balance|fees|estimate` (no
  `broadcast`); `sigillum api wallets
  xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check`
  (no sign/send); `sigillum api compartment list`.
- **Steps:** one family per commit, mirroring the `daemon_api/queue.rs`
  bridge pattern; each command gets an adversarial-parse case and a
  happy-path case.
- **Accept:** commands exist + documented; no sign/send/broadcast CLI paths.
- **Verify:** `cargo test -p sigillum-cli -p sigillum-client`; full gate.
  **Size:** L.

#### D2 — Operator-surface parity matrix

- **Files:** new `docs/operator-surface-parity.md`, linked from
  `PRODUCTION_READINESS.md` and `docs/architecture.md`.
- **Steps:** enumerate every route family from
  `crates/sigillum-daemon/src/routes/mod.rs` (lines 214–623); one table row
  each: routes → UI? → CLI? → decision. Record explicit decisions with
  one-line rationale for: `evm/broadcast`, `wallets` sign/send, `fido2/*`
  admin (physical touch; UI), `compartment add/remove/init` (destructive;
  UI typed-confirmation), manual `queue/enqueue/*`, `setup/reset`,
  `auth/capability`, daemon-side `api-keys`/`secrets` CRUD. **This file
  must be updated by every W-task that adds routes** (per §0.1.5).
- **Accept:** every registered route appears in exactly one row; no row is
  all-no with no decision. **Size:** M.

---

### Phase E — Automation, recovery, and policy completion

(Depends on B. E1/E2 are prerequisites for W7.)

#### E1 — Explicit queue job state model

- **Goal:** jobs expose explicit states — `blocked`, `deferred`,
  `retryable`, `operator_action_required` — instead of implicit retry
  behavior. W7 builds on these states; get them right here.
- **Files:** `crates/sigillum-daemon/src/service/queue/{state,processing,payloads}.rs`;
  DTOs in `sigillum-api/src/{request,response}/queue.rs`; UI queue view
  (`ui/src/views/operations.ts`); CLI queue list; `queue.json` schema
  version.
- **Steps:** add the state enum to the persisted job document with a
  schema-version bump + migration mapping legacy jobs; classify existing
  retry transitions; `operator_action_required` is never auto-retried;
  surface in API/UI/CLI; tests per transition + a legacy-fixture migration
  test.
- **Accept:** every job always reports one explicit state; restart preserves
  states; migration test green. **Verify:**
  `cargo test -p sigillum-daemon queue`; full gate. **Size:** L.

#### E2 — Restart/replay guarantees, test-backed

- **Steps:** in `crates/sigillum-daemon/tests/crash_recovery.rs`, simulate
  each interruption window of the atomic writer for `profiles.json`,
  `deposits.json`, `queue.json` (temp-not-renamed; renamed-with-stale-bak;
  truncated live file) and assert restore, quarantine, and recovery
  telemetry.
- **Accept:** each window has a named test matching the documented
  restore/quarantine contract. **Size:** M.

#### E3 — Maintenance summaries with cause separation

- **Steps:** maintenance run reports deposits refreshed, sweeps enqueued,
  jobs executed, and failures by cause (provider error / policy block /
  insufficient gas / validation) in API + UI + CLI; one test per cause.
- **Accept:** categorized summary visible in all three surfaces. **Size:** M.

#### E4 — Persist `chain_id` on treasury receive allocations (D-8)

- **Goal:** remove the mainnet hardcodes at
  `service/inventory/treasury.rs:337` and `:354`.
- **Steps:** add `chain_id` to the allocation document (schema-version bump;
  legacy defaults to `1` with an operator-visible "assumed mainnet" marker);
  source the value from the deriving provider profile; expose through
  treasury DTOs, UI, and `sigillum api treasury receive-list`; delete the
  comment at `treasury.rs:336`.
- **Accept:** new allocations persist real chain ids; legacy fixture
  migrates; W1 can rely on the field. **Size:** M.

#### E5 — Destructive-flow recovery completion

- **Steps:** vault init/remove, snapshot restore, compartment
  replacement/recovery: journal entry before mutation; interruption tests at
  pre-/mid-/post-mutation crash points; startup reconciliation completes or
  cleanly rolls back; document per-flow guarantees in `docs/backup.md`.
- **Accept:** three flows × three crash points, all tested; docs state the
  guarantees. **Size:** L.

---

### Phase W — Wallet-management completion (the 1.0 product core)

Every W-task follows the Feature finish line (§0.1.5): API + route + client +
operator surface + schema-versioned persistence + tests + docs, in the same
PR chain. Update `docs/wallet-management-roadmap.md` status text and
`docs/operator-surface-parity.md` as you land each one.

#### W1 — Chain registry and multi-network EVM foundation

##### W1.1 — First-class chain registry

- **Goal:** chains stop being bare integers; a registry drives per-chain
  behavior.
- **Files:** new `crates/sigillum-daemon/src/service/chains.rs`; promote
  `ChainProfile` (`sigillum-api/src/response.rs:689`) from descriptive to
  registry-backed; new routes under `/api/chains` (list/upsert/delete for
  custom entries); persistence in a schema-versioned store; UI in
  `inventory.ts` chain-profiles section; CLI `sigillum api chains
  list|upsert|delete`.
- **Registry entry fields:** `chain_id`, `name`, `native_symbol`,
  `native_decimals`, `finality_blocks` (confirmation depth for W7 receipt
  checks), `permit2_address` (optional override), `builtin: bool`.
- **Steps:**
  1. Ship the five built-ins from D-10 as non-deletable `builtin` entries;
     operator-defined custom entries validated (`chain_id != 0`, unique).
  2. Provider profiles resolve their registry entry by `chain_id`; selfcheck
     (`service/selfcheck.rs:186-207`) warns when a provider's chain has no
     registry entry.
  3. Permit2 discovery (`service/inventory/permit2_discovery.rs:9,39`) reads
     the per-chain `permit2_address`, falling back to the canonical address
     only for chains where the registry does not override it.
- **Accept:** registry CRUD works end to end (API/UI/CLI); built-ins
  present; Permit2 override honored in a test with a non-mainnet chain id.
- **Verify:** `cargo test -p sigillum-daemon chains`; full gate. **Size:** L.

##### W1.2 — `chain_id` on inventory records

- **Goal:** every discovered address and holding knows its chain.
- **Files:** `WalletInventoryAddress` / `WalletAssetHolding`
  (`sigillum-api/src/response.rs:574,589` region),
  `crates/sigillum-daemon/src/inventory.rs` state schema,
  `service/inventory/observation.rs`.
- **Steps:** verify whether the records already carry a chain field (survey
  found none); add `chain_id` populated from the scanning provider profile;
  schema-version bump with legacy default `1` + "assumed mainnet" marker
  (same pattern as E4); surface in inventory UI/CLI listings; dedup keys for
  addresses/holdings must include `chain_id` so the same address on two
  chains is two records.
- **Accept:** a scan against a mock provider reporting chain id 8453
  produces records tagged 8453; legacy fixture migrates; dedup respects
  chain. **Size:** M.

##### W1.3 — Multi-chain scan orchestration

- **Goal:** one operator action can inventory a wallet family across all
  configured chains.
- **Files:** `WalletInventoryScanRequest`
  (`sigillum-api/src/request/inventory.rs`), `service/inventory.rs` scan
  entry points, discovery-job records, UI scan form, CLI
  `sigillum api inventory scan-evm`.
- **Steps:** accept either one provider profile (current behavior) or
  `all_configured_chains: true` (iterate every provider profile bound to the
  compartment, sequentially, one discovery job per chain, honoring existing
  checkpoints per chain); job records carry `chain_id`; planner destination
  resolution and treasury roll-ups group by chain; cross-chain steps are
  never generated (a plan is single-chain; the treasury console shows
  per-chain sections).
- **Accept:** with two mock providers (chain 1 + 8453), one scan request
  yields two jobs and chain-tagged inventory; generated plans never mix
  chains; UI/CLI show per-chain grouping.
- **Verify:** `cargo test -p sigillum-daemon inventory`; full gate.
  **Size:** L.

#### W2 — Typed domain model (execution-safety prerequisite)

- **Goal:** the values W7 will branch on stop being raw strings. Convert to
  enums with exact wire-compat (D-18): `asset_kind`
  (`response.rs:589`; values `native|erc20|erc721|erc1155|nft|approval|defi|airdrop|reward`),
  address `classifications` (`response.rs:574`; the 14 values listed in
  §2.2), plan step `action` (`response.rs:791`; the 10 actions), step
  `status`/`signer_status`/`simulation_status` and plan `status`
  (`response.rs:792,817,818,833`).
- **Files:** `sigillum-api` (new enums + serde), producers/consumers in
  `service/inventory/{observation,planner,preflight,simulation,export}.rs`,
  UI type definitions in `ui/src/` where these strings are matched.
- **Steps:**
  1. Define enums in `sigillum-api` with `#[serde(rename = "...")]` matching
     current strings exactly and an `Other(String)` catch-all
     (`#[serde(untagged)]` or custom impl) for forward compat.
  2. Migrate producers/consumers module by module; delete stringly matches.
  3. Extend B4's `tests/roundtrip.rs` with fixtures asserting the serialized
     form of every variant equals the pre-change literal strings (copy the
     literals from this plan, not from the new code).
  4. Persisted inventory/plan state deserializes unchanged (no schema bump
     needed if wire-identical — prove it with a fixture of pre-change JSON).
- **Accept:** no `asset_kind`/classification/action/status string literals
  remain outside the enum definitions and tests
  (`grep -rn '"sweep_native"' crates/*/src` finds only serde attrs +
  fixtures); pre-change JSON fixture loads.
- **Verify:** `cargo test --workspace`; UI tests; full gate. **Size:** L.

#### W3 — Discovery completion

##### W3.1 — Block-range checkpoints for log scans

- **Goal:** ERC-20/721/1155 transfer-log and announcement scans become
  incremental and resumable instead of bounded one-shots.
- **Files:** `service/inventory/checkpoints.rs`, log-scan call sites in
  `service/inventory/*discovery*.rs`, `WalletDiscoveryCheckpoint`
  (`sigillum-api/src/response.rs:618-632`), inventory state schema.
- **Steps:** add per-(address, chain, topic-family) block cursors
  (`last_scanned_block`, persisted each chunk); scans resume from the cursor
  and honor the existing bounded-range safety caps per run; expose cursor
  freshness in job records and the inventory UI ("scanned to block N").
- **Accept:** two consecutive scans against a mock provider scan disjoint
  block ranges; cancel/resume continues from the cursor; caps still bound a
  single run. **Size:** L.

##### W3.2 — Full ERC-1155 coverage

- **Goal:** ERC-1155 discovery handles both `TransferSingle` and
  `TransferBatch`.
- **Steps:** verify current handling (survey did not confirm batch events);
  implement `TransferBatch` decoding if missing; `balanceOf(address,id)`
  confirmation per touched id stays mandatory; mock `eth_getLogs` batch
  fixtures in `tests/daemon_service.rs`.
- **Accept:** a mock batch event with 3 ids yields exactly the
  positive-balance holdings. **Size:** M.

##### W3.3 — Local token registry import (D-15)

- **Goal:** ERC-20 discovery can probe operator-imported token lists, not
  just transfer logs and manual probes.
- **Files:** new registry store (schema-versioned, per-compartment) +
  `/api/inventory/token-registry` routes; import via pasted JSON or local
  file path (validate: `chain_id`, `address`, `symbol`, `decimals`); scan
  flag `probe_token_registry`; UI + CLI (`sigillum api inventory
  token-registry import|list|delete`).
- **Steps:** on scan, probe registry entries matching the scan's chain with
  `balanceOf`; positive balances become holdings with provenance
  `token_registry:<list-name>`. No network fetching of lists — import only.
- **Accept:** imported list produces holdings only for positive balances;
  provenance recorded; wrong-chain entries skipped. **Size:** M.

##### W3.4 — NFT metadata cache + local spam heuristics (D-15, D-16)

- **Goal:** discovered NFTs get optional metadata and a spam/suspicion
  classification, locally.
- **Files:** new `service/inventory/nft_metadata.rs`; holding metadata
  fields; risk-catalog integration; UI inventory NFT rendering.
- **Steps:**
  1. Metadata fetch is **opt-in per collection** (operator action, like RPC
     calls the privacy cost is surfaced): resolve `tokenURI`/`uri`, fetch
     over the daemon's existing bounded HTTP client, cache locally with
     provenance (URI, fetch time, content hash). IPFS URIs resolve through
     an operator-configured gateway; none configured → skip with reason.
  2. Spam heuristics are local rules only: collection contract not in any
     holding the operator interacted with (no outbound tx), airdropped
     pattern (received without matching approval/interaction), name/symbol
     lookalikes of risk-catalog trusted entries, operator overrides via the
     existing risk catalog. Output: `spam_suspected` flag + reasons on the
     holding; **never auto-hide** — suspicious assets go to an explicit
     bucket in the UI.
- **Accept:** metadata cached with provenance for an opted-in mock
  collection; heuristics flag a mock airdropped collection; nothing fetches
  without opt-in. **Size:** L.

##### W3.5 — On-chain last-activity signals

- **Goal:** dormancy classification uses chain evidence, not just scan
  timestamps.
- **Steps:** at scan time record per-address `eth_getTransactionCount`
  (nonce) and the max block seen for that address across its transfer-log
  cursors (W3.1); derive `last_activity_block`; feed the
  `dormant_candidate` classification (`observation.rs:396`) with a policy
  window (blocks-per-chain from the W1.1 registry entry, default heuristic
  documented); expose in inventory UI/CLI.
- **Accept:** mock address with old activity classifies dormant; recent
  activity does not; classification reason includes the block evidence.
  **Size:** M.

#### W4 — DeFi exit adapters (D-11)

- **Goal:** the adapter set is Aave v3 withdraw (exists:
  `service/inventory/defi_adapters.rs:8`, preflight adapter
  `aave-v3-withdraw`), generic ERC-4626 redeem, Uniswap v2 LP
  removeLiquidity, and Lido wstETH unwrap. Each: position detection from
  the existing receipt-token probes, exit-call construction, `eth_call`
  preflight, gas verification, mock-RPC tests.
- **Files:** `service/inventory/defi_adapters.rs`,
  `service/inventory/{defi_discovery,preflight,planner}.rs`, adapter
  constants in `sigillum-api` if exposed, UI plan rendering, docs.
- **Steps (one adapter per PR):**
  1. **ERC-4626:** detect via `convertToAssets`/`maxRedeem` probes on
     operator-configured share tokens; exit = `redeem(maxRedeem(owner),
     owner, owner)`; preflight simulates and records expected assets out.
  2. **Uniswap v2 LP:** exit is two ordered steps — `approve(router, lp_balance)`
     then `removeLiquidity(...)` with `amountMin`s derived from reserves at
     plan time and a deadline; **requires W6.5 step ordering**; router
     address is per-chain operator-supplied config (no hardcoded router).
  3. **Lido:** `wstETH.unwrap()` only, producing stETH holdings; the stETH →
     ETH withdrawal queue is out of scope (D-11) — stETH surfaces as a
     holding with a `review_asset` note.
  4. Positions that match no adapter keep the existing `review_asset`
     fallback; never guess an exit call.
- **Accept:** each adapter has: detection test, preflight-pass test,
  preflight-revert test (blocked step), gas-shortfall test; plan steps carry
  adapter id + expected-output evidence.
- **Verify:** `cargo test -p sigillum-daemon defi`; full gate.
  **Size:** XL (bounded by the fixed adapter list).

#### W5 — Merkle claim execution enablement (D-12)

- **Goal:** `merkle-distributor-v1` claims become executable — everything
  else stays review-only.
- **Files:** `service/inventory/planner.rs:136-157`
  (`push_claim_reward_blockers`), `TreasuryPolicy` (new field
  `allow_claim_execution`, default `false`),
  `sigillum-api/src/{request,response}/treasury.rs`, onboarding policy
  opt-in surface, UI treasury policy editor + plan rendering, CLI treasury
  policy commands.
- **Steps:**
  1. Add the policy field (schema-version bump, default false, surfaced in
     onboarding beside `block_cross_party_linkage` as a fail-closed opt-in).
  2. `claim_execution_disabled` blocker is pushed unless ALL hold: policy
     enabled AND adapter is `merkle-distributor-v1` AND simulation passed
     AND the claim contract has a risk-catalog entry of trusted or an
     explicit operator review record AND the step is approved.
  3. Executable claim steps flow through the W7 execution path; claims are
     `operator_action_required` on any revert (never auto-retried — the
     proof may be consumed).
- **Accept:** with policy off, behavior is byte-identical to today (blocker
  present); with policy on + all gates, the blocker is absent and W7 can
  enqueue; each gate has a negative test.
- **Verify:** `cargo test -p sigillum-daemon claim`; full gate. **Size:** M.

#### W6 — Consolidation planner completion

##### W6.1 — Gas top-up steps (`fund_gas`)

- **Goal:** a new `fund_gas` step funds a source address from the wallet's
  **sponsor address** (`service/profiles/seed_wallets.rs:195-198`) when gas
  verification fails for lack of native balance.
- **Files:** `service/inventory/{planner,preflight,simulation}.rs`, the W2
  action enum, `TreasuryPolicy` (new `max_gas_topup_wei_hex` cap +
  `allow_gas_topups` opt-in, default false), export manifests, UI, CLI.
- **Steps:**
  1. Amount = estimated gas cost of the dependent step × 1.5, capped by
     policy; emitted only when the wallet has a sponsor address with
     sufficient balance; otherwise the dependent step keeps its existing
     gas blocker.
  2. **Linkage rule (critical):** funding two different counterparties'
     addresses from one sponsor links them by common-funder. `fund_gas`
     steps must run the same party analysis as sweeps: cross-party sponsor
     funding always carries a `linkage_warnings` entry, and is a hard
     blocker when `block_cross_party_linkage` is enabled. Add this to the
     linkage analysis module alongside `analyze_plan_linkage`; document in
     the README privacy model (this narrows the current "gas funding is
     operator discipline" caveat — update that text to say Sigillum-generated
     top-ups are checked, manual funding remains operator discipline).
  3. `fund_gas` is ordered before its dependent step (W6.5).
- **Accept:** shortfall + sponsor → `fund_gas` emitted with cap enforced;
  cross-party case warns/blocks per policy; no sponsor → old blocker
  preserved; README updated.
- **Verify:** `cargo test -p sigillum-daemon planner`; full gate. **Size:** L.

##### W6.2 — Dynamic fees in planning and preflight

- **Goal:** plan gas verification and preflight use live estimation, not
  only static profile fees.
- **Steps:** when the provider profile enables estimation, gas verification
  in `service/inventory/preflight.rs` uses `estimate_eip1559_fees`
  (`service/evm/fees.rs:8-28`) and records the fee basis
  (static-profile vs estimated, values, timestamp) as step evidence;
  approval re-checks staleness (older than the policy window → simulation
  status downgraded to `required`).
- **Accept:** fee basis visible on steps; stale-estimate approval forces
  re-simulation; static path unchanged when estimation disabled.
  **Size:** M.

##### W6.3 — Policy-driven hot floor/refill (replaces the 1 ETH hardcode)

- **Goal:** `resolve_default_destination` (`planner.rs:332-372`, hardcode at
  `:357`) reads `TreasuryPolicy` instead.
- **Steps:** add `hot_floor_wei_hex` and `hot_target_wei_hex` to the policy
  (schema bump; migration defaults preserve today's behavior: target = 1
  ETH); planner routes to `hot_address` below floor, `treasury_address`
  otherwise; validation `floor <= target`; UI/CLI policy editor fields.
- **Accept:** hardcode gone; legacy policy migrates to identical routing;
  floor/target respected in planner tests. **Size:** M.

##### W6.4 — Step dependency ordering

- **Goal:** plans express ordered dependencies (`fund_gas` → sweep;
  approve → removeLiquidity) that export and execution must honor.
- **Files:** `ConsolidationPlanStep` (`response.rs:789` region — add
  `sequence: u32` + `depends_on: Vec<String>` step ids), `planner.rs`,
  `export.rs` (manifest ordering; Safe batch ordering), W7 execution.
- **Steps:** planner assigns sequence + dependencies; export emits steps in
  dependency order and refuses to export a step whose dependency is
  blocked/skipped (skip reason names the dependency); W7 executes in order
  and halts dependents on failure.
- **Accept:** Uniswap v2 exit exports approve-before-remove; blocked
  dependency propagates a skip; cycle detection rejects malformed plans
  (defensive — planner should never emit one).
- **Verify:** `cargo test -p sigillum-daemon export planner`; full gate.
  **Size:** M.

#### W7 — Controlled execution of consolidation plans

This is the largest and highest-risk phase. Everything defaults OFF (D-6).
Order within W7 is strict: W7.1 → W7.2 → W7.3 → W7.4 → W7.5.

##### W7.1 — Execution policy gates and kill switch

- **Goal:** the policy surface that all execution checks, before any
  execution code exists.
- **Files:** `TreasuryPolicy` (+`allow_plan_execution: bool` master gate,
  default false; per-family gates `allow_sweep_execution`,
  `allow_revoke_execution`, `allow_exit_execution` — claims use W5's field,
  top-ups W6.1's; `execution_paused: bool` runtime kill switch), onboarding
  opt-in surface, UI policy editor + a prominent pause control in the
  operations view, CLI `sigillum api treasury policy-update` +
  `sigillum api queue pause|resume`.
- **Steps:** master gate AND family gate AND not-paused must all hold at
  BOTH enqueue and execution time (re-read policy at each; a policy flip
  between enqueue and drain blocks the job into `blocked` state, per E1).
  Pause is immediate: no new job starts; an in-flight job finishes its
  current broadcast attempt or aborts pre-broadcast.
- **Accept:** with all gates off nothing about today's behavior changes;
  every gate has an enqueue-time and an execution-time negative test; pause
  halts a drain loop mid-queue in a test.
- **Verify:** `cargo test -p sigillum-daemon policy queue`; full gate.
  **Size:** M.

##### W7.2 — Plan-step queue payloads and enqueue path

- **Goal:** approved, simulated, unblocked plan steps can be enqueued as
  first-class queue jobs.
- **Files:** `QueueJobPayload` (`sigillum-api/src/response/queue.rs:7-97` —
  new `PlanStepExecution` variant carrying plan id, step id, chain id,
  source address + derivation evidence, prepared call/transfer parameters,
  simulation evidence hash, fee basis), `service/queue/payloads.rs`, new
  route `POST /api/plans/enqueue-step` (+ bulk `enqueue-plan` for all
  auto-eligible steps), `service/inventory/consolidation.rs`, UI plan view
  ("Execute" appears only when every gate passes), CLI
  `sigillum api plans enqueue-step|enqueue-plan`.
- **Steps:**
  1. Enqueue validation re-checks, server-side, at enqueue time: step
     approved + simulation passed + not blocked + policy gates (W7.1) +
     treasury destination/cap checks + linkage policy re-evaluation
     (same re-check-at-approval pattern that already exists for treasury
     policy) + simulation freshness (W6.2 window; stale → refuse with
     "re-simulate").
  2. Idempotency: a step can be enqueued once; re-enqueue of a
     pending/succeeded step is rejected; a failed step needs explicit
     operator re-approval after inspection.
  3. Dependency ordering (W6.4): enqueue-plan enqueues in sequence;
     dependents carry their prerequisite job id.
- **Accept:** every validation has a negative test; idempotency and
  dependency chaining tested; export-only behavior fully preserved when
  gates are off.
- **Verify:** `cargo test -p sigillum-daemon plans queue`; full gate.
  **Size:** L.

##### W7.3 — Seed-wallet signing execution

- **Goal:** lift the hard block at `service/queue/processing.rs:189-193`
  behind the W7.1 gates, and execute `PlanStepExecution` jobs.
- **Files:** `service/queue/{processing,sweeps}.rs` (new execution module
  `service/queue/plan_steps.rs` mirroring the sweeps split),
  `service/profiles/{sends,resolution}.rs`, `service/evm.rs` signing paths,
  audit events (`state/audit_keys.rs`).
- **Steps:**
  1. Signer resolution: derive the signing key for the step's source address
     from the profile's seed (receive or control branch) inside the unlocked
     compartment; watch-only sources are unreachable here by construction
     (enqueue validation), but re-check and fail to `blocked` anyway.
  2. Execute per action family: native/ERC-20/NFT sweep transfers, revoke
     calls, exit-adapter calls, Merkle claims, gas top-ups — reusing the
     prepared calldata from preflight (never rebuild calldata at execution
     time; if inputs changed, fail to `operator_action_required`).
  3. `EthSeed*` legacy variants: route through the same gate checks; with
     gates off the block message is unchanged.
  4. Audit: every execution emits typed audit events (enqueued → signed →
     broadcast → confirmed/failed) with plan/step/job ids and tx hash.
- **Accept:** with gates on, a full mock-RPC plan (sweep + revoke + top-up
  chain) executes in dependency order with audit trail; with gates off,
  processing behavior is byte-identical to today.
- **Verify:** `cargo test -p sigillum-daemon plan_steps`; full gate.
  **Size:** XL.

##### W7.4 — Execution semantics: nonces, receipts, failure classes

- **Goal:** execution is safe under concurrency, reorgs, and fee volatility.
- **Files:** `service/queue/{processing,plan_steps}.rs`, chain registry
  (`finality_blocks` from W1.1), E1 state machine.
- **Steps:**
  1. **Per-source serialization:** at most one in-flight job per (source
     address, chain); others wait in `deferred`.
  2. **Nonce management:** fetch at broadcast time; on `nonce too low`
     re-fetch once; on repeat → `operator_action_required`.
  3. **Receipt confirmation:** poll `eth_getTransactionReceipt` until
     `finality_blocks` confirmations; success/revert recorded with gas
     used; timeout window → `operator_action_required` with the tx hash
     (never assume failure of a broadcast tx).
  4. **Failure classes:** provider/network error → `retryable` (existing
     backoff); revert → `operator_action_required` (never auto-retry a
     revert); underpriced/replacement → one fee bump within the policy fee
     cap, then `operator_action_required`.
- **Accept:** mock-RPC tests for each: serialization, nonce race, revert,
  underpriced-then-bump, receipt timeout; kill -9 during in-flight job →
  restart resumes receipt polling (job not duplicated, ties into E2).
- **Verify:** `cargo test -p sigillum-daemon plan_steps queue`;
  full gate. **Size:** L.

##### W7.5 — Linkage enforcement parity at execution

- **Goal:** the linkage guarantees hold on the new path exactly as on
  stealth sweeps (enforced at plan generation, approval, AND enqueue).
- **Steps:** extend `detect_stealth_sweep_linkage`-equivalent checks to
  plan-step enqueue (W7.2 already calls it — this task proves parity):
  matrix test of tagged/untagged parties × destination collisions ×
  policy on/off, on the plan-step path; document in the README privacy
  model that execution enforces the same single-hop destination-axis claim.
- **Accept:** the matrix test passes; a policy flip between approval and
  enqueue blocks (parity with the existing treasury allowlist behavior).
  **Size:** M.

#### W8 — Treasury automation (overflow/refill)

- **Goal:** the maintenance cycle GENERATES hot-overflow and treasury-refill
  steps under policy; execution rides W7.
- **Files:** maintenance service, `service/inventory/{planner,treasury}.rs`,
  `TreasuryPolicy` (+`hot_overflow_wei_hex` threshold,
  `allow_treasury_automation: bool` default false), UI treasury console,
  CLI.
- **Steps:**
  1. During maintenance, when automation is enabled: hot balance >
     overflow threshold → generate a `sweep_native` step hot → treasury for
     the excess above `hot_target_wei_hex`; hot < `hot_floor_wei_hex` →
     generate treasury → hot refill up to target. Both use the standard
     plan pipeline (simulation, policy, linkage, approval).
  2. Steps are `review_required` by default; they become auto-eligible only
     when `allow_treasury_automation` AND the W7.1 gates hold AND
     simulation passed — then maintenance may enqueue them (D-17: quorum
     was satisfied at unlock; the policy opt-ins are the authorization).
  3. Hysteresis: no oscillation — refill and overflow may not both trigger
     in one cycle; enforce `floor <= target <= overflow` at policy
     validation.
- **Accept:** maintenance summary (E3) reports generated/enqueued treasury
  steps distinctly; oscillation test (balances near thresholds across
  cycles) shows no ping-pong; with automation off, maintenance behavior is
  unchanged.
- **Verify:** `cargo test -p sigillum-daemon treasury maintenance`;
  full gate. **Size:** L.

---

### Phase F — Assurance expansion

(Depends on C, E, W7. F4/F6 run with Phase H — see notes.)

#### F1 — Adversarial coverage: receiving, treasury, chains, plans-execution

- **Steps:** extend `crates/sigillum-daemon/tests/adversarial_api.rs` with
  rejection cases for `/api/receiving/*`, `/api/treasury/*`, `/api/chains/*`,
  `/api/plans/enqueue-*`: malformed JSON, wrong content type, missing/bad
  bearer tokens, invalid addresses, oversized labels, negative/overflow
  amounts, policy-violating destinations, stale/foreign plan+step ids,
  double-enqueue, policy flipped between approve and enqueue.
- **Accept:** every listed route family has ≥3 adversarial cases; the
  execution path has the specific stale/replay/flip cases. **Size:** M.

#### F2 — Nightly deep-fuzz in CI

- **Steps:** in `.github/workflows/ci.yml`, on
  `github.event_name == 'schedule'` export
  `SIGILLUM_ADVERSARIAL_PROPTEST_CASES=1024` (PRs keep 256).
- **Accept:** nightly log shows 1024 cases; PR runtime unchanged. **Size:** S.

#### F3 — Chaos mode for the soak harness

- **Steps:** add `SIGILLUM_SOAK_CHAOS=1` to `scripts/check-local-soak.sh`:
  every `SIGILLUM_SOAK_CHAOS_EVERY` (default 10) iterations, `kill -9` the
  daemon, restart, require next-iteration doctor + canary pass; count
  cycles in the receipt. After W7 lands, the chaos run must also have a
  pending mock plan-step job in flight and verify it resumes to a terminal
  state without duplication.
- **Accept:** 600s chaos run passes with ≥2 kill cycles and the in-flight
  job assertion. **Size:** M.

#### F4 — Release-commit soak receipts per supported host

- **Ordering note:** depends on G3+G5 — receipts must reference the
  release-candidate SHA. Execute while preparing H1.
- **Steps:** on each host named supported in
  `docs/production-readiness-audit.md` (currently `mac-server`): 3600s
  standard soak + 600s chaos soak at the RC commit; record receipt
  filename, SHA, host, OS in the audit doc.
- **Accept:** fresh receipts per host at the RC SHA. **Size:** M (wall-clock).

#### F5 — Execution-path security review

- **Steps:** run a focused review of the W7 execution surface before
  enabling it in any receipt: threat cases — malicious plan JSON injected
  into state files (quarantine path), calldata tampering between preflight
  and execution (evidence-hash check from W7.2), policy TOCTOU (re-read
  tests), session-token theft → enqueue attempt (audit + policy gates),
  linkage bypass attempts. Each becomes a regression test where feasible;
  findings and dispositions recorded in `docs/production-readiness-audit.md`.
- **Accept:** the five threat cases have written dispositions; testable ones
  have regression tests. **Size:** M.

#### F6 — Testnet execution receipts (D-14) — human-in-the-loop

- **Ordering note:** runs with Phase H, against the RC build. Needs a human
  to fund accounts.
- **Steps:** with operator-supplied Sepolia + one L2-testnet RPC endpoints
  and funded seed profiles: execute and record (tx hashes + audit export)
  one each of native sweep, ERC-20 sweep (faucet token), ERC-20 revoke, gas
  top-up chain (`fund_gas` → sweep). Adapter exits and a Merkle claim:
  attempt if suitable contracts are available; otherwise record
  "mock-verified only" explicitly. Store the receipt summary in
  `docs/production-readiness-audit.md`.
- **Accept:** the four core families have real-testnet tx evidence at the RC
  SHA; adapter/claim status recorded honestly either way. **Size:** M
  (wall-clock + human).

---

### Phase G — Release engineering

(Depends on all prior phases except F4/F6. Do in order.)

#### G1 — CHANGELOG.md

- **Steps:** create root `CHANGELOG.md` (Keep-a-Changelog): `[1.0.0]`
  section at feature granularity — vault/compartments/unlock, daemon +
  console, stealth custody, discovery/inventory/risk, consolidation
  planning **and execution**, treasury + receiving + linkage policy, chain
  registry, DeFi/claim adapters, gateway sidecar, desktop app, CLI, release
  gate. `[Unreleased]` on top.
- **Accept:** exists, links the tag, no placeholders. **Size:** S.

#### G2 — Stability policy (`docs/stability.md`)

- **Steps:** declare **stable at 1.0**: `sigillum-api` wire shapes, daemon
  route paths/semantics, CLI syntax, on-disk formats (schema-versioned,
  migration-only evolution), `sigillum-core` public traits, the
  `TreasuryPolicy` fail-closed defaults. **Unstable:** daemon internal
  modules, UI DOM, gateway (preview), `sigillum-sdk`/`-server` facades,
  §2.3 items. SemVer from 1.0.0. Link from README.
- **Accept:** doc exists, linked, consistent with D-1..D-18. **Size:** S.

#### G3 — Version bump 0.1.0 → 1.0.0

- **Steps:** root `Cargo.toml` workspace version + internal dependency pins
  (lines 19, 28–33 region); regenerate `Cargo.lock` (`cargo check`);
  `crates/sigillum-desktop/tauri.conf.json` version; UI `package.json`
  version if present; sweep `grep -rn '"0\.1' --include='*.toml'
  --include='*.json' .` (minus `target/`, `node_modules/`) for stragglers.
- **Accept:** `cargo metadata --no-deps` reports 1.0.0 for all 12 crates.
- **Verify:** full gate. **Size:** S.

#### G4 — Release workflow

- **Steps:** new `.github/workflows/release.yml` on `push: tags: ['v*']`:
  job `verify` (ubuntu+macos matrix mirroring ci.yml) runs
  `./scripts/check-release.sh`; job `artifacts-macos` (needs verify)
  installs tauri-cli, `cargo tauri build`, `cargo build --release -p
  sigillum-cli`, uploads `.dmg`/zipped `.app`/CLI binary; job
  `artifacts-linux` builds + uploads the CLI binary; job `release` creates
  a draft GitHub Release with the `[1.0.0]` CHANGELOG section and a
  `SHA256SUMS` file. Dry-run with `v1.0.0-rc.1`, then delete the rc
  tag/release.
- **Accept:** rc dry run produces all artifacts + draft release. **Size:** M.

#### G5 — Readiness and product docs final sync

- **Steps:** update `PRODUCTION_READINESS.md`,
  `docs/production-readiness-audit.md`, `README.md` (the "current release
  boundary" and Status sections change materially: wallet management is now
  shipped scope, execution is policy-gated opt-in),
  `docs/wallet-management-roadmap.md` (mark phases 1–9 complete for EVM;
  phase 10 = post-1.0), `docs/catchup-plan.md` (absorbed), and
  `docs/architecture.md`. Keep D-4 claim wording. Point "Current Plan Of
  Record" at this file. Record F5 dispositions.
- **Accept:** no doc contradicts another; §1 deliverables verifiable from
  docs alone. **Size:** M.

---

### Phase H — Final gate and ship

#### H1 — Release candidate verification (all must pass, in order)

- [ ] Fresh clone of `main` at the RC commit; `./scripts/check-release.sh`
      passes there.
- [ ] CI green on the RC commit, both legs.
- [ ] F4 soak receipts (standard + chaos) reference the RC SHA.
- [ ] F6 testnet receipts recorded for the four core execution families.
- [ ] Desktop `.dmg` from the G4 rc run installs and reaches the unlock
      screen on a machine without a dev toolchain.
- [ ] `sigillum doctor` passes on each supported host.
- [ ] A full local walkthrough of the completion bar: import a seed →
      multi-chain scan → review inventory/risk → generate plan → approve →
      execute against a local mock provider → audit trail complete.
- [ ] CHANGELOG date filled; G5 docs merged.

#### H2 — Tag and release

```bash
git checkout main && git pull --ff-only
./scripts/check-release.sh
git tag -a v1.0.0 -m "Sigillum 1.0.0 — Local-first wallet-management workstation"
git push origin v1.0.0
# watch .github/workflows/release.yml, verify artifacts + SHA256SUMS,
# publish the draft GitHub Release.
```

#### H3 — Post-release

- [ ] Bump workspace to `1.1.0-dev` (same file set as G3) + `[Unreleased]`
      CHANGELOG section.
- [ ] Open a post-1.0 planning issue: non-EVM entry point (Bitcoin/UTXO
      first, per roadmap phase 10), swaps (D-13 revisit), valuation (D-16
      revisit), crates.io (D-1 revisit), Linux desktop demand.

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

Phase W — Wallet-management completion
- [ ] W1.1 chain registry (built-ins + custom + Permit2 override)
- [ ] W1.2 chain_id on inventory records
- [ ] W1.3 multi-chain scan orchestration
- [ ] W2 typed domain model (wire-compatible enums)
- [ ] W3.1 block-range checkpoints for log scans
- [ ] W3.2 full ERC-1155 (TransferBatch)
- [ ] W3.3 local token registry import
- [ ] W3.4 NFT metadata cache + local spam heuristics
- [ ] W3.5 on-chain last-activity signals
- [ ] W4 DeFi exit adapters: ERC-4626, UniV2 LP, Lido unwrap (Aave v3 exists)
- [ ] W5 Merkle claim execution enablement
- [ ] W6.1 fund_gas steps with linkage rule
- [ ] W6.2 dynamic fees in planning/preflight
- [ ] W6.3 policy-driven hot floor/refill
- [ ] W6.4 step dependency ordering
- [ ] W7.1 execution policy gates + kill switch
- [ ] W7.2 plan-step queue payloads + enqueue validation
- [ ] W7.3 seed-wallet signing execution
- [ ] W7.4 nonces, receipts, failure classes
- [ ] W7.5 linkage enforcement parity at execution
- [ ] W8 treasury automation (overflow/refill, hysteresis)

Phase F — Assurance
- [ ] F1 adversarial coverage: receiving/treasury/chains/execution
- [ ] F2 nightly deep-fuzz
- [ ] F3 chaos soak mode (+ in-flight job assertion)
- [ ] F4 RC soak receipts per supported host
- [ ] F5 execution-path security review
- [ ] F6 testnet execution receipts (human-in-the-loop)

Phase G — Release engineering
- [ ] G1 CHANGELOG.md
- [ ] G2 docs/stability.md
- [ ] G3 version bump to 1.0.0
- [ ] G4 release workflow + rc dry run
- [ ] G5 readiness + product docs final sync

Phase H — Ship
- [ ] H1 RC verification checklist
- [ ] H2 v1.0.0 tagged, artifacts published
- [ ] H3 post-release bump + planning issue

## 6. Work log

(append one line per completed task: `YYYY-MM-DD <task-id> <commit-sha> <result>`)
