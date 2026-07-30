# Sigillum 1.0 Release Plan

**Status:** Active plan of record for the 1.0 release (rev 5 — wallet management
IN scope; RC5 draft evidence and 2026-07-30 stop-ship hardening reconciled)
**Baseline verified:** 2026-07-01, branch `feat/private-receiving-desktop` (commits `70a087b`, `1cda1f2` ahead of `main`)
**Supersedes:** [catchup-plan.md](./catchup-plan.md) Phases 1–3 are absorbed into Phases D–E and W1–W8 below. The
[wallet-management-roadmap.md](./wallet-management-roadmap.md) product target is **part of 1.0** (EVM scope — see D-9);
catchup Phase 4 (remote/platform) stays out.

**Operational companion:** [execution-runbook-1.0.md](./execution-runbook-1.0.md)
records current hardening truth, release sequencing, operator gates, and
failure-recovery procedures. Read it before executing anything here.

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
- Task F6 (testnet receipts) needs a human to supply funded Ethereum Sepolia
  (`11155111`) plus Base Sepolia (`84532`), Arbitrum Sepolia (`421614`), or OP
  Sepolia (`11155420`) accounts and RPC endpoints; it is flagged as
  human-in-the-loop.

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

### 2.2 Historical discovery baseline (2026-07-01; not current truth)

This section preserves the initial survey that motivated the plan. Its version,
workflow, packaging, and product-state claims are intentionally historical and
must not be used as current repository truth. Executors must re-anchor on the
current checkout and use `docs/execution-runbook-1.0.md` for the live release
contract. Platform history comes first, then the wallet subsystem.

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
- **Release gate env toggles:** `SIGILLUM_SKIP_BROWSER_SMOKE=1`, `TMPDIR`,
  and (via `scripts/check-adversarial.sh:13`)
  `SIGILLUM_ADVERSARIAL_PROPTEST_CASES` (default 256).

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
  ERC-1155 discovery **already decodes both `TransferSingle` and
  `TransferBatch`** with length-mismatch rejection and unit tests
  (`service/inventory/nft_discovery.rs:17,185,314,363`).
- **Inventory model:** asset kinds and address classifications are
  **strings, not enums** (`asset_kind: String` at `response.rs:589`;
  `classifications: Vec<String>` at `response.rs:574`; values produced in
  `service/inventory/observation.rs:328-402`). Addresses, holdings, and plan
  steps **already carry `chain_id`** populated from the scanning provider,
  and the dedup keys already include it (`response.rs:562/586/796`;
  `service/inventory/support.rs:228,372,406,438`). Per-address
  `transaction_count` is already recorded at scan time and feeds
  activity/dormancy classification (`observation.rs:61-71`), but no
  `last_activity_block` is derived. NFT holdings already carry
  `metadata_uri`/`metadata_name`/`spam_label` fields, and an
  `nft_metadata_cache` store with a conservative spam labeler exists
  (`response.rs:605-609,660,678`; `support.rs:84-141`). **No valuation
  fields.**
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
  ("seed-wallet queue execution is not enabled yet"). Queue jobs **already
  persist an explicit `state`** in the schema-versioned `sigillum.queue` v1
  store (`crates/sigillum-daemon/src/queue_store.rs:21`):
  `queued|blocked|retrying|sent|failed_terminal` plus legacy
  `deferred`/`failed` normalization (`service/queue/state.rs`), rendered in
  `ui/src/views/queue.ts`. **Consolidation plan
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
  (deposits/maintenance), `queue`, `receiving`, `selfcheck`, `session`,
  `setup`, `shell`, `treasury`, `walletManager`, `wallets`.

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
| D-3 | **Desktop bundles ship with a project-enforced full-bundle ad-hoc signature by default** (no Apple account; set `APPLE_SIGNING_IDENTITY=-` explicitly after fail-closed credential validation); full Developer ID signing/notarization remains env-gated. macOS 15+ removed the right-click Gatekeeper bypass, so C3/C6 must document the exact System Settings → Privacy & Security → "Open Anyway" path plus `SHA256SUMS` verification. | No Apple Developer credentials assumed. Tauri's no-identity path can leave only a linker signature, as RC3 proved, so source and release bundles must pass strict app/dmg verification before H1. |
| D-4 | **No external penetration test for 1.0.** Claim stays "source-verified local-first release gate". | The audit doc already draws this boundary honestly. |
| D-5 | **CLI parity for scriptable families only** — `transit`, read-only `evm`, `wallets` export/derive/check/generate, `compartment list`, plus the wallet-management surfaces already bridged. `wallets` sign/send and `evm broadcast` stay API+UI-only. | Signing/broadcast from shell history is an operator hazard; UI/API cover it. |
| D-6 | **All policy guardrails stay fail-closed opt-in.** Every NEW execution capability (plan execution, claim execution, treasury automation, gas top-ups) defaults OFF behind its own `TreasuryPolicy` opt-in, surfaced in onboarding like `block_cross_party_linkage`. | Execution is the highest-risk surface 1.0 adds; defaults must be safe. |
| D-7 | **Rust stays pinned at 1.88.0** unless a RustSec advisory forces a bump (that is a stop condition). | Toolchain drift invalidates the evidence chain. |
| D-8 | **Treasury allocations get `chain_id` persistence** with a schema-versioned migration defaulting legacy records to `1`. | Prerequisite for multi-network EVM (W1). |
| D-9 | **Wallet-management 1.0 scope = roadmap phases 1–9 on EVM networks.** Phase 10 (non-EVM) is 1.x. | The roadmap's own completion sentence is chain-family-agnostic and achievable on EVM; its Multi-Chain Direction section sequences non-EVM explicitly after EVM completion ("only then"). |
| D-10 | **Built-in chain registry entries at 1.0** (chain id / native symbol — use exactly these, do not guess): Ethereum (1/ETH), Base (8453/ETH), Arbitrum One (42161/ETH), OP Mainnet (10/ETH), Polygon PoS (137/POL — post-MATIC migration). Other EVM chains via operator-defined custom entries. | Matches the roadmap's named networks; custom entries keep it open without shipping untested defaults. |
| D-11 | **DeFi exit-adapter set at 1.0:** Aave v3 withdraw (exists), generic ERC-4626 redeem, Uniswap v2 LP removeLiquidity, Lido wstETH unwrap. Nothing else; other positions surface as `review_asset`. | Standard interfaces with dominant TVL coverage; bounded and testable. Uniswap v3 NFT positions and Lido withdrawal queue are disproportionate for 1.0. |
| D-12 | **Claim execution at 1.0 = `merkle-distributor-v1` adapter only**, gated by simulation pass + explicit step approval + risk-catalog review + a policy opt-in. All other claim types remain review/export-only. | The simulation slice for this adapter already exists; it is the only claim shape safe to automate. |
| D-13 | **No swap step type at 1.0.** Planner does not emit swap steps; dust keeps the `review_asset` fallback. | The roadmap marks swaps "optionally"; DEX routing/slippage is a large adversarial surface orthogonal to the completion bar. |
| D-14 | **Execution testing bar:** every execution family needs mock-RPC integration tests (mandatory) AND recorded public-testnet evidence on Sepolia (`11155111`) plus one supported L2 testnet — Base Sepolia (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`) — for native sweep, ERC-20 sweep, revoke, and gas top-up. Four families require five transactions: gas top-up must include both receipt-confirmed `fund_gas` and dependent-sweep legs, with the sweep blocked until the top-up reaches finality. Adapter exits and Merkle claims: mock-mandatory, testnet best-effort (contract availability permitting). | Real broadcasts need real-network evidence; contract-dependent families should not block on deploying testnet contracts. The offline bundle checker validates schema-v2 structure and runtime identity bindings, while H2 independently verifies all five claimed transactions on the named public chains. |
| D-15 | **Registries stay local.** Token lists, spender labels, spam heuristics: operator-imported files + the existing risk catalog. No runtime fetching of external feeds. | Preserves the local-first/no-phone-home boundary; RPC endpoints remain the only outbound surface. |
| D-16 | **No valuation at 1.0.** Holdings show raw amounts; no fiat/floor pricing. | Price feeds add an external dependency + phone-home surface for cosmetic value; the completion bar doesn't need it. |
| D-17 | **Quorum model at 1.0 = unlock-time compartment threshold** (already implemented in `service/lifecycle.rs`). Execution adds per-plan explicit approval + typed confirmation at enqueue (W7.2) + policy gates + gate-flip audit events (W7.1), not a second quorum ceremony. **Recorded residual risk:** with `allow_plan_execution` on, a stolen session token on the local machine can move funds — the mitigations above detect and bound it, they do not prevent it. This residual risk MUST be stated in `docs/stability.md` and the readiness docs (G2/G5). FIDO2 tap-to-execute is the named post-1.0 hardening candidate. | The threshold ceremony already exists at unlock; on a single-operator machine the marginal boundary of a second ceremony is real but small, and the risk is honestly documented instead of silently accepted. |
| D-18 | **Wire-format compatibility rule for W2:** enum conversions must serialize to the exact current strings; unknown inbound values map to an explicit `Other(String)`/unknown variant, never a parse failure. | The API contract is stable-in-place; typed safety must not break existing clients or persisted state. |

---

## 4. Phases and tasks

Phase order:

```
A → (B ∥ C ∥ D) → E → (W1 ∥ W2) → (W3 ∥ W4 ∥ W5 ∥ W6) → W7 → W8 → F → G → H
```

- B, C, D independent after A merges. E depends on B.
- W1 (chains) and W2 (typed model) start after E; they are each other's peers.
- W3–W6 need W1+W2 (W5 also needs E1). **Exception inside the parallel group:
  W6.4 (step ordering) must land before W4's Uniswap v2 sub-task.**
- W7 needs E1, E2, W2, W6. W8 needs W7.
- F depends on C, E, W1.1 (F1 covers `/api/chains/*`), and W7 (F4/F6/F7 run
  with Phase G/H — see their notes).
- G depends on everything except F4/F6/F7. H is the final gate.

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
  identifier `com.sigillum.desktop`; the `version` field already exists
  (`"0.1.0"`) — keep it matching the workspace (G3 bumps it); replace the
  133-byte `dist/index.html` with a minimal static fallback page.
- **Accept:** `cargo tauri build` on macOS emits `.app` + `.dmg`; the app
  reaches the daemon UI. **Size:** S.

#### C3 — Env-gated signing/notarization (D-3)

- **Steps:** validate Tauri v2 signing and notarization variables as complete,
  mutually exclusive credential families; fail closed on partial/mixed values.
  With no credentials, set `APPLE_SIGNING_IDENTITY=-` so Tauri signs the whole
  app bundle. With Developer ID credentials, explicitly submit and staple the
  signed dmg after Tauri creates it because the pinned bundler notarizes the app
  first. Enforce nonempty `CodeResources`, strict deep verification, the
  exact identifier and executable, bound `Info.plist`, sealed resources, and
  the expected ad-hoc or Developer ID mode in `scripts/check-desktop.sh` (C4).
  Document in `docs/deployment.md`: macOS
  15+ removed the right-click Gatekeeper bypass — give the exact System
  Settings → Privacy & Security → "Open Anyway" flow, `SHA256SUMS`
  verification before opening, and the full-credentials path.
- **Accept:** clean-shell build yields a strictly verified full-bundle ad-hoc
  signature; complete Developer ID inputs plus exactly one notarization family
  select the signed/notarized/stapled path; every incomplete credential matrix
  or Developer ID-without-notarization configuration fails before build; docs
  cover both paths including the macOS 15 flow.
  **Size:** S.

#### C4 — Desktop check script in the release gate

- **Steps:** `scripts/check-desktop.sh`: always
  `cargo build -p sigillum-desktop --locked`; macOS-only (skippable via
  `SIGILLUM_SKIP_DESKTOP_BUNDLE=1` outside CI) wrapper-driven debug bundle
  build; require exactly one app/dmg; mount the dmg read-only and run one
  reusable verifier against source and mounted apps, including CDHash parity.
  Run negative regressions for the RC3 linker-only shape, missing seal,
  missing hardened runtime, tampering, wrong identifier, CDHash mismatch,
  zero/multiple/wrong-name apps, symlink escape, and paths with spaces. In
  Developer ID mode, require the dmg to be non-ad-hoc and signed by the same
  team as the app and validate stapled tickets on source app, mounted app, and
  dmg. Keep mode-independent hostile dmg-layout regressions in the always-on
  ad-hoc suite; use a scoped stapler-failure injection for the Developer ID dmg
  ticket error because Tauri deletes its temporary certificate keychain after
  bundling. Print an explicit skip line on other OSes. Wire into
  `check-release.sh` after browser smoke; add tauri-cli install to the CI
  macOS leg if needed; document the toggle in the audit doc.
- **Accept:** gate runs the desktop step on both OSes; the added macOS CI
  wall-clock is measured and recorded in the PR (tauri-cli cached by
  rust-cache; if the debug bundle adds >10 min, gate it to `main`-push and
  nightly runs only and record that decision). **Size:** M.

#### C5 — Desktop testability: extract and test boot helpers

- **Steps:** move port selection, daemon-readiness wait, URL construction,
  and lock-on-close decision logic from `main.rs` into a new
  `crates/sigillum-desktop/src/lib.rs`; unit-test each (readiness-wait Ok on
  listener, clean timeout otherwise; URL round-trip; bindable port).
- **Accept:** `cargo test -p sigillum-desktop` runs ≥4 meaningful tests;
  `main.rs` shrinks to Tauri wiring. **Size:** M.

#### C7 — Operator console UX redesign (user-directed, 2026-07-03)

- **Goal:** the embedded console gets a ground-up UX redesign. Previous
  incremental passes were judged insufficient by the operator; this is an
  information-architecture restructure, not a polish pass.
- **Design direction (fixed):** a *quiet security instrument* — calm,
  dark (`#0d1117` base, matching the app icon), monochrome with a single
  accent, high density where data lives and generous space where decisions
  happen, AA+ contrast, visible focus states, no decorative chrome.
- **Information architecture (fixed):** replace the feature-bucket
  navigation with five goal-oriented destinations in a left rail:
  1. **Overview** — lock/compartment status, self-check, recommended next
     action, recent audit events.
  2. **Receive** — receiving console: allocations, stealth deposits,
     counterparties, rotation.
  3. **Portfolio** — inventory, risk findings, discovery jobs, watch book.
  4. **Move** — consolidation plans, queue, maintenance, treasury policy.
  5. **Vault** — secrets/API keys, transit, snapshots, compartments,
     FIDO2 keys, diagnostics.
  A persistent status strip shows lock state, active compartment,
  self-check pill, and "Lock now". Setup wizard and locked states keep
  their flows but adopt the same visual system. Danger actions are
  visually distinct and keep typed confirmation.
- **Hard contracts (break nothing, or update every consumer in the same
  change):** the runtime smoke greps (`Sigillum Vault` title,
  `id="statusCard"`, `/api/status` wiring in
  `scripts/check-runtime-smoke.sh`); every selector used by
  `scripts/browser-smoke.mjs`; the DOM expectations in
  `crates/sigillum-daemon/ui/test/ui-smoke.test.ts`; the nonce-based CSP
  and Rust-side asset assembly; the checked-in generated `app.js` +
  `styles.css` (regenerate and commit — the gate checks freshness).
  Prefer keeping element IDs / `data-action` contracts stable and
  restructuring layout, navigation, hierarchy, and styles around them.
- **Accept:** all five destinations navigable with consolidated content;
  no feature loses its surface (cross-check against
  `docs/operator-surface-parity.md`); UI tests, typecheck, build, daemon
  HTML tests, runtime smoke, and browser smoke all green; screenshots of
  setup, locked, and unlocked states reviewed by the operator.
- **Verify:** `npm --prefix crates/sigillum-daemon/ui run typecheck && npm
  --prefix crates/sigillum-daemon/ui test && npm --prefix
  crates/sigillum-daemon/ui run build`; `cargo test -p sigillum-daemon`;
  `./scripts/check-runtime-smoke.sh`; `./scripts/check-browser-smoke.sh`.
  **Size:** XL.

#### C6 — Desktop documentation

- **Steps:** README + `docs/deployment.md`: install from `.dmg`; shared
  `~/.sigillum` data dir + `SIGILLUM_BASE_DIR`; single-instance; tray lock
  state; close-to-tray auto-lock; quit zeroization; ad-hoc
  signing/Gatekeeper caveat;
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
  `crates/sigillum-daemon/src/routes/mod.rs` (`api_routes()` starting at
  line 233; router assembled in `api_router()` at line 212); one table row
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

#### E0 — Replace the macOS blanket skip on client loopback tests

- **Goal:** macOS CI actually executes the client loopback tests. Today
  `crates/sigillum-client/src/tests.rs` marks them
  `#[cfg_attr(target_os = "macos", ignore = "sandbox blocks loopback bind")]`
  — a blanket platform skip that made macOS blind to a real regression
  (caught only by Ubuntu during Wave A2).
- **Steps:** replace the attribute with a runtime probe: attempt a
  `127.0.0.1:0` bind at test start; skip (early-return with an eprintln
  naming the reason) only when the bind actually fails. Apply the same
  pattern to any sibling crate using the blanket skip (grep for the
  attribute string workspace-wide).
- **Accept:** on a normal macOS host `cargo test -p sigillum-client` RUNS
  the loopback tests (not ignored); sandboxed environments still skip
  gracefully with a visible reason. **Size:** S.

#### E1 — Extend the existing queue job state model

- **Goal:** EXTEND — do not replace — the state model that already exists.
  The `sigillum.queue` v1 store already persists
  `queued|blocked|retrying|sent|failed_terminal` plus legacy
  `deferred`/`failed` normalization (`service/queue/state.rs`,
  `queue_store.rs:21`). What is missing for W7:
  an `operator_action_required` state (terminal-until-human), and explicit,
  documented semantics for `deferred` (currently a legacy value that
  normalization folds away).
- **Files:** `crates/sigillum-daemon/src/service/queue/{state,processing,payloads}.rs`;
  `crates/sigillum-daemon/src/queue_store.rs`; DTOs in
  `sigillum-api/src/{request,response}/queue.rs`; UI queue view
  (`ui/src/views/queue.ts` — NOT `operations.ts`); CLI queue list.
- **Steps:**
  1. **Wire-compat rule (same spirit as D-18):** the existing state strings
     `queued`, `blocked`, `retrying`, `sent`, `failed_terminal` and the
     legacy normalization behavior must not change. Do NOT rename
     `retrying` to `retryable`.
  2. Add `operator_action_required` as a new state with a schema-version
     bump (v1 → v2) and a migration test on a v1 fixture; it is never
     auto-retried and requires an explicit operator action (inspect +
     re-approve or cancel) to leave.
  3. Give `deferred` first-class semantics (waiting on balance/gas/
     dependency, re-evaluated each maintenance cycle) or explicitly retire
     it in favor of `blocked` with a reason — decide from how
     `QUEUE_STATE_LEGACY_DEFERRED` is produced today, record the choice in
     the PR, and keep legacy normalization working either way.
  4. Surface any new states + reasons in API/UI/CLI; tests per new
     transition.
- **E1 implementation decision (2026-07-03):** producer search found no active
  producer of legacy `deferred`; keep it legacy-only and preserve the existing
  `deferred` → `blocked` normalization. Recovery records a missing reason when
  it performs that normalization. `operator_action_required` is added as a
  distinct non-runnable persisted state; explicit reapprove/cancel endpoints
  remain out of E1 and belong with the later approval semantics.
- **Accept:** pre-change v1 queue fixture loads unchanged; existing state
  strings byte-identical on the wire; `operator_action_required` exists,
  is never auto-retried, and round-trips restart. **Verify:**
  `cargo test -p sigillum-daemon queue`; full gate. **Size:** M.

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

#### E4 — Persist `chain_id` on receiving records (D-8)

- **Goal:** remove BOTH mainnet hardcodes in
  `service/inventory/treasury.rs` — they live in different stores:
  `:337` is the HD **allocation** document; `:354` is
  `stealth_receiving_item`, built from `EthStealthDeposit` (the deposits
  store).
- **Steps:**
  1. Allocations: add `chain_id` to the allocation document
     (schema-version bump; legacy defaults to `1` with an operator-visible
     "assumed mainnet" marker); source the value from the deriving provider
     profile.
  2. Stealth deposits: persist `chain_id` on deposit records at
     creation/refresh, sourced from the deposit's provider profile
     (deposits store schema bump, same legacy-default pattern);
     `stealth_receiving_item` reads it instead of hardcoding `1`.
  3. Expose through treasury/receiving DTOs, UI, and
     `sigillum api treasury receive-list`; delete the comment at
     `treasury.rs:336`.
- **Accept:** both hardcodes gone; new records persist real chain ids; both
  legacy fixtures migrate; W1 can rely on the fields. **Size:** M.

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
  `ChainProfile` (`sigillum-api/src/response/inventory.rs`) from descriptive
  to registry-backed; new routes under `/api/chains` (list/upsert/delete for
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

##### W1.2 — `chain_id` legacy defaults + surfacing on inventory records

- **Premise (verified):** `chain_id` ALREADY exists on addresses, holdings,
  and plan steps, populated from the scanning provider, with chain-aware
  dedup (`response/inventory.rs`; `support.rs`). Do NOT add the field, a
  migration for it, or dedup changes — they exist.
- **Goal:** close the residue only.
- **Steps:**
  1. Verify how a legacy inventory JSON document written before the field
     existed deserializes today (serde default?); if the default is `0` or
     absent-panics, add an explicit legacy default of `1` with an
     operator-visible "assumed mainnet" marker (fixture test either way).
  2. Surface `chain_id` in inventory UI listings and
     `sigillum api inventory list` output wherever addresses/holdings are
     shown (registry name from W1.1 when available, raw id otherwise).
- **Accept:** legacy-fixture deserialization behavior is tested and
  explicit; UI/CLI show chains on inventory rows. **Size:** S.

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
  enums with exact wire-compat (D-18). The complete literal sets (copy
  these, they are verified against the code as of 2026-07-01):
  - `asset_kind` (`response.rs:589`):
    `native|erc20|erc721|erc1155|nft|approval|defi|airdrop|reward`
  - address `classifications` (`response.rs:574`; produced in
    `observation.rs:328-402`): `signer_available`, `watch_only`,
    `signer_unknown`, `gas_available`, `transaction_history`,
    `token_holding`, `nft_holding`, `protocol_holding`, `value_detected`,
    `asset_value_detected`, `stranded_value`, `approval_exposure`,
    `dormant_candidate`, `empty_candidate`
  - plan step `action` (`response.rs:791`): `sweep_native`, `sweep_erc20`,
    `sweep_nft`, `revoke_erc20_approval`, `revoke_permit2_allowance`,
    `revoke_nft_operator_approval`, `revoke_approval`,
    `exit_defi_position`, `claim_reward`, `review_asset`
  - step `status` (`response.rs:792`): `review_required|blocked|approved`
  - `signer_status` (`response.rs:817`): `watch_only|available|unknown`
  - `simulation_status` (`response.rs:818`):
    `required|not_run|passed|failed|unsupported|blocked`
  - plan `status` (`response.rs:833`):
    `empty|blocked|review_required|approved`
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

##### W3.2 — ERC-1155 batch end-to-end fixture

- **Premise (verified):** `TransferBatch` decoding is ALREADY implemented
  with length-mismatch rejection and unit tests
  (`service/inventory/nft_discovery.rs:17,185,314,363,481,494`). Do NOT
  re-implement it.
- **Goal:** the missing piece is end-to-end coverage: a mock `eth_getLogs`
  `TransferBatch` fixture in `tests/daemon_service.rs` exercising the full
  scan → holding pipeline.
- **Accept:** a mock batch event with 3 ids yields exactly the
  positive-balance holdings through the real scan route. **Size:** S.

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

##### W3.4 — NFT metadata fetch pipeline + heuristics extension (D-15, D-16)

- **Premise (verified):** the storage layer ALREADY exists — holdings carry
  `metadata_uri`/`metadata_name`/`spam_label` (`response.rs:605-609`), an
  `nft_metadata_cache` store exists (`response.rs:660,678`;
  `daemon/src/inventory.rs:31`), and a conservative spam labeler runs
  (`support.rs:84-141`). Build ON these structures; do NOT create a second
  cache or parallel fields.
- **Goal:** add the missing fetch pipeline and extend the heuristics.
- **Files:** `service/inventory/support.rs` (existing labeler),
  a new fetch module (e.g. `service/inventory/nft_metadata.rs`) writing
  into the EXISTING cache entries; UI inventory NFT rendering.
- **Steps:**
  1. Metadata fetch is **opt-in per collection** (operator action; the
     privacy cost is surfaced like RPC calls): resolve `tokenURI`/`uri`,
     fetch over the daemon's existing bounded HTTP client, populate the
     existing cache entries with provenance (URI, fetch time, content
     hash). IPFS URIs resolve through an operator-configured gateway; none
     configured → skip with reason.
  2. Extend `conservative_nft_spam_label` with local rules: airdropped
     pattern (received without matching approval/interaction), name/symbol
     lookalikes of risk-catalog trusted entries, operator overrides via the
     existing risk catalog. Reasons recorded alongside the label;
     **never auto-hide** — suspicious assets go to an explicit bucket in
     the UI.
- **Accept:** metadata cached with provenance for an opted-in mock
  collection into the existing store; heuristics flag a mock airdropped
  collection with reasons; nothing fetches without opt-in. **Size:** M.

##### W3.5 — `last_activity_block` derivation

- **Premise (verified):** per-address `eth_getTransactionCount` is ALREADY
  recorded and feeds activity/dormancy (`observation.rs:61-71`, dormant
  classification at `:396`). Do NOT re-add it.
- **Goal:** derive the missing `last_activity_block`: the max block seen
  for an address across its transfer-log cursors (W3.1) and announcement
  scans; classify dormancy against a per-chain block window (window config
  on the W1.1 registry entry, documented default), instead of transaction
  count alone.
- **Accept:** mock address with only old-block activity classifies dormant;
  recent-block activity does not; the classification reason includes the
  block evidence; inventory UI/CLI show last activity. **Size:** M.

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
     plan time and a deadline; **requires W6.4 step ordering — do not start
     this sub-task until W6.4 has merged**; router address is per-chain
     operator-supplied config (no hardcoded router).
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
  3. `fund_gas` is ordered before its dependent step (W6.4).
- **Accept:** shortfall + sponsor → `fund_gas` emitted with cap enforced;
  cross-party case warns/blocks per policy; no sponsor → old blocker
  preserved; README updated.
- **Verify:** `cargo test -p sigillum-daemon planner`; full gate. **Size:** L.

##### W6.2 — Dynamic fees in planning and preflight

- **Goal:** plan gas verification and preflight use live estimation, not
  only static profile fees.
- **Steps:**
  1. **Create the policy field this depends on** (no other task does):
     `simulation_freshness_secs: u64` on `TreasuryPolicy`, default `900`,
     schema-version bump + migration, editable in the UI policy editor and
     `sigillum api treasury policy-update`. W7.2 reads this same field.
  2. When the provider profile enables estimation, gas verification in
     `service/inventory/preflight.rs` uses `estimate_eip1559_fees`
     (`service/evm/fees.rs:8-28`) and records the fee basis
     (static-profile vs estimated, values, timestamp) as step evidence.
  3. Approval re-checks staleness: evidence older than
     `simulation_freshness_secs` → simulation status downgraded to
     `required`.
- **Accept:** the policy field exists with default + migration; fee basis
  visible on steps; stale-estimate approval forces re-simulation; static
  path unchanged when estimation disabled. **Size:** M.

##### W6.3 — Policy-driven hot floor/refill (replaces the 1 ETH hardcode)

- **Goal:** `resolve_default_destination` (`planner.rs:332-372`, hardcode at
  `:357`) reads `TreasuryPolicy` instead.
- **Steps:** add `hot_floor_wei_hex` and `hot_target_wei_hex` to the policy
  (schema bump). **Migration must preserve today's exact routing:** today
  the planner routes to hot when `hot_balance < 1 ETH`, else treasury
  (`planner.rs:358`), so BOTH defaults are 1 ETH — `floor = target =
  0xde0b6b3a7640000`. Planner routes to `hot_address` when balance < floor,
  `treasury_address` otherwise, refilling up to target; validation
  `floor <= target`; UI/CLI policy editor fields.
- **Accept:** hardcode gone; a legacy-policy fixture produces routing
  byte-identical to today for balances below/at/above 1 ETH; floor/target
  respected in planner tests. **Size:** M.

##### W6.4 — Step dependency ordering

- **Goal:** plans express ordered dependencies (`fund_gas` → sweep;
  approve → removeLiquidity) that export and execution must honor.
- **Files:** `ConsolidationPlanStep` (`response.rs:789` region — add
  `sequence: u32` + `depends_on: Vec<String>` step ids), `planner.rs`,
  `export.rs` (manifest ordering; Safe batch ordering), W7 execution.
- **Ordering note:** this task lands BEFORE W4's Uniswap v2 sub-task and
  before W6.1 (both consume it). Its tests must not depend on either.
- **Steps:** planner assigns sequence + dependencies; export emits steps in
  dependency order and refuses to export a step whose dependency is
  blocked/skipped (skip reason names the dependency); W7 executes in order
  and halts dependents on failure.
- **Accept:** a synthetic two-step plan fixture (constructed directly in
  tests, not via the UniV2 adapter) exports in dependency order; blocked
  dependency propagates a skip; cycle detection rejects malformed plans
  (defensive — planner should never emit one). After W6.1/W4 land, their
  own tests re-exercise this via `fund_gas`→sweep and approve→remove.
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
  top-ups W6.1's; `execution_paused: bool` runtime kill switch;
  `max_fee_per_gas_cap_hex: Option<String>` — the fail-closed signing fee
  ceiling W7.4 uses, no other task creates it), onboarding opt-in
  surface, UI policy editor + a prominent pause control in the operations
  view, CLI `sigillum api treasury policy-update` +
  `sigillum api queue pause|resume`.
- **Steps:**
  1. Master gate AND family gate AND not-paused must all hold at BOTH
     enqueue and execution time (re-read policy at each; a policy flip
     between enqueue and drain blocks the job into `blocked` state, per
     E1). Pause is immediate for subsequent work: an RPC already entered may
     finish, but no later job or new network submission starts after the latch
     is observed. Pause sets that lock-free latch before waiting for the drain's
     operation mutex; the drain checks it between jobs and immediately before
     broadcast. A job already signed stays durably `prepared` and resumes later
     with the same bytes.
  2. **Every change to an execution gate or the pause flag emits a typed
     audit event** (old value, new value, session) — gate flips are
     security-relevant actions (F5 threat model).
- **Accept:** with all gates off nothing about today's behavior changes;
  every gate has an enqueue-time and an execution-time negative test; pause
  halts a drain loop mid-queue in a test; gate-flip audit events asserted
  in tests.
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
  4. **Typed confirmation** (house pattern from `setup/reset`): bulk
     `enqueue-plan` requires the operator to type a confirmation phrase
     that includes the step count and total native value (UI and CLI);
     single-step enqueue requires an explicit confirm flag. Simulation
     freshness uses W6.2's `simulation_freshness_secs`.
- **Accept:** every validation has a negative test; idempotency and
  dependency chaining tested; export-only behavior fully preserved when
  gates are off.
- **Verify:** `cargo test -p sigillum-daemon plans queue`; full gate.
  **Size:** L.

##### W7.3 — Seed-wallet signing execution

- **Goal:** lift the hard block at `service/queue/processing.rs:189-193`
  behind the W7.1 gates, and execute `PlanStepExecution` jobs.
- **Files:** `service/queue/{processing,sweeps,broadcast}.rs` (new execution
  module `service/queue/plan_steps.rs` mirroring the sweeps split),
  `service/profiles/{sends,resolution}.rs`, `service/evm.rs` signing paths,
  audit events (`state/audit_keys.rs`).
- **Steps:**
  1. Signer resolution: derive the signing key for the step's source address
     from the profile's seed (receive or control branch) inside the unlocked
     compartment; watch-only sources are unreachable here by construction
     (enqueue validation), but re-check and fail to `blocked` anyway.
  2. Prepare per action family: native/ERC-20/NFT sweep transfers, revoke
     calls, exit-adapter calls, Merkle claims, gas top-ups — reusing the
     prepared calldata from preflight (never rebuild calldata at execution
     time; if inputs changed, fail to `operator_action_required`). Signing
     performs no subsequent network I/O: the drain first persists exact raw
     bytes and the local transaction hash as `prepared`.
  3. `EthSeed*` legacy variants: route through the same gate checks; with
     gates off the block message is unchanged.
  4. Audit: every execution emits typed audit events (enqueued → signed →
     broadcast → confirmed/failed) with plan/step/job ids and tx hash;
     broadcast occurs only through the shared exact-byte submission module.
- **Accept:** with gates on, a full mock-RPC plan (sweep + revoke + top-up
  chain) executes in dependency order with audit trail; with gates off,
  processing behavior is byte-identical to today. **Key hygiene:** derived
  signing keys are held in `Zeroizing` wrappers (house style, cf.
  `ethereum_xpub.rs`), zeroized after each job, and NEVER logged, embedded
  in audit payloads, or persisted in queue/job state — an explicit test
  asserts audit events and persisted jobs contain no key material.
- **Verify:** `cargo test -p sigillum-daemon plan_steps`; full gate.
  **Size:** XL.

##### W7.4 — Execution semantics: durable submission, nonces, receipts, failure classes

- **Goal:** execution is safe under crashes, concurrency, reorgs, and fee
  volatility without ever re-signing a job after its durable prepare barrier.
- **Files:** `service/queue/{processing,broadcast,serialization,state}.rs`,
  `service/queue/plan_steps/{signing,receipts}.rs`, queue receipt DTO/store,
  chain registry (`finality_blocks` from W1.1), E1 state machine.
- **Steps:**
  1. **Per-source serialization:** at most one `prepared`,
     `submitted_unknown`, or broadcast-but-unconfirmed job per (source address,
     chain); independent siblings remain in their existing state with a visible
     skip reason rather than using the legacy `deferred` string.
  2. **One-signature preparation:** resolve the nonce once while signing, then
     durably persist `state=prepared`, the exact raw transaction bytes, and the
     locally derived hash before any network I/O. Once prepared, the job may
     never be re-signed.
  3. **Crash-safe submission:** durably persist `state=submitted_unknown`
     before the first RPC call. On restart or an ambiguous transport outcome,
     check the receipt by stored hash and, when still unresolved, resubmit only
     the exact prepared bytes.
  4. **Receipt confirmation:** poll `eth_getTransactionReceipt` until
     `finality_blocks` confirmations; success/revert recorded with gas
     used; timeout window → `operator_action_required` with the tx hash
     (never assume failure of a broadcast tx).
  5. **Failure classes:** pre-sign provider errors may use existing retry
     backoff; an ambiguous post-submission result remains `submitted_unknown`;
     revert, `nonce too low`, and underpriced/replacement-underpriced park as
     `operator_action_required`. The latter two require an explicit future
     replacement job and never trigger nonce re-fetch, fee bump, or re-signing.
     Claims are stricter: any returned submission error parks because a proof
     may be single-use; a process crash still recovers from the durable marker.
- **Accept:** tests prove serialization, durable schema compatibility,
  deterministic rejection without re-signing, ambiguous outcome preservation,
  receipt/revert/timeout handling, exact-byte crash recovery, and a real
  concurrent HTTP pause that stops later broadcasts.
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
  filename, SHA, host, and OS in the sanitized external release-evidence
  bundle. H2 binds that bundle's digest into the immutable final tag; H3 writes
  the sanitized summary and bundle link into the audit doc. Receipts cannot be
  committed into their own receipt-bearing RC SHA.
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
- **Steps:** with operator-supplied Sepolia (`11155111`) plus one supported
  L2-testnet RPC endpoint — Base Sepolia (`84532`), Arbitrum Sepolia
  (`421614`), or OP Sepolia (`11155420`) — and funded seed profiles: execute
  and record five transaction hashes plus audit exports: one each for native
  sweep, ERC-20 sweep (faucet token), and ERC-20 revoke, plus both legs of the
  gas top-up chain (`fund_gas` → dependent sweep). Process the top-up to
  receipt-confirmed finality before allowing the dependent sweep to sign.
  Adapter exits and a Merkle claim:
  attempt if suitable contracts are available; otherwise record
  "mock-verified only" explicitly. Store the sanitized receipt summary and
  audit export in the external release-evidence bundle. H2 binds that bundle's
  digest into the immutable final tag; H3 records the public bundle link and
  sanitized summary in `docs/production-readiness-audit.md`.
- **Accept:** the four core families have five real-testnet transactions at the
  RC SHA. F6 schema v2 binds the two gas legs to one plan/network/chain, their
  distinct jobs and steps, the top-up destination/dependent source, the
  prerequisite edge, confirmed successful receipts, and strict block order.
  Before H2, the operator independently confirms every transaction on the
  claimed public chain (chain ID, successful receipt, finality, and the family
  effect represented by its audit export). The offline evidence checker proves
  archive structure and internal bindings, not live-chain truth. Adapter/claim
  status is recorded honestly either way. **Size:** M
  (wall-clock + human).

#### F7 — 0.1 → 1.0 data-directory upgrade verification

- **Ordering note:** runs after ALL W-track schema changes have merged and
  before G3. This is a release blocker for a local-first tool holding
  fund-controlling state: the plan adds many per-store schema bumps (E1,
  E4, W1.1, W3.1, W3.3, W5, W6.3, W7.1, W8) but nothing else proves they
  compose on a real old data directory.
- **Steps:**
  1. Build the daemon at the A3 merge SHA and script a fixture generator:
     initialize a temp base dir with a passphrase compartment and populate
     EVERY store — profiles (provider/stealth/xpub/seed), deposits, queue
     (with a pending job), inventory (addresses/holdings/watch book/risk
     catalog), treasury policy + receive allocations, counterparties,
     audit history, and a passphrase-encrypted snapshot export. Commit the
     generator script and the resulting fixture archive under
     `crates/sigillum-daemon/tests/fixtures/` (fake data only, §0.1.9).
  2. Add an integration test: boot the CURRENT daemon on a copy of the
     fixture dir → all migrations apply (assert each store's schema
     version), no quarantine events, `sigillum doctor` passes, canaries
     read back, queue job still terminal-or-active with a valid state.
  3. Add a restore test: the 0.1-era snapshot restores under the 1.0
     daemon (E5's crash tests cover interruption; this covers version
     skew).
- **Accept:** both tests green in CI; upgrade guarantees documented in
  `docs/backup.md`; H1 references this evidence.
- **Verify:** `cargo test -p sigillum-daemon upgrade`; full gate.
  **Size:** M.

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
  §2.3 items. SemVer from 1.0.0. Must include the D-17 residual-risk
  statement (stolen session token + execution gates on = fund movement;
  mitigations bound, not prevent). Link from README.
- **Accept:** doc exists, linked, consistent with D-1..D-18, states the
  D-17 residual risk. **Size:** S.

#### G3 — Version bump 0.1.0 → 1.0.0

- **Steps:** root `Cargo.toml` workspace version + internal dependency pins
  (lines 19, 28–33 region); regenerate `Cargo.lock` (`cargo check`);
  `crates/sigillum-desktop/tauri.conf.json` version; UI `package.json`
  version if present; then sweep for stragglers with a grep **constrained
  to workspace-owned manifests only** — root `Cargo.toml`,
  `crates/*/Cargo.toml`, `crates/sigillum-desktop/tauri.conf.json`, and the
  UI `package.json` `version` field. Do NOT touch third-party version pins
  in dependencies/devDependencies or `deny.toml` that happen to match
  `0.1`.
- **Accept:** `cargo metadata --no-deps` reports 1.0.0 for all 12 crates.
- **Verify:** full gate. **Size:** S.

#### G4 — Release workflow

- **Steps:** new `.github/workflows/release.yml` on `push: tags: ['v*']`:
  job `verify` (ubuntu+macos matrix mirroring ci.yml) runs
  `./scripts/check-release.sh`; job `artifacts-macos` (needs verify)
  installs tauri-cli, builds through the fail-closed signing wrapper, strictly
  verifies the release app and read-only-mounted dmg before staging,
  `cargo build --release -p
  sigillum-cli`, uploads `.dmg`/zipped `.app`/CLI binary; job
  `artifacts-linux` builds + uploads the CLI binary; job `release` creates
  a draft GitHub Release with the `[1.0.0]` CHANGELOG section and a
  `SHA256SUMS` file. Job 4 also attaches a `THIRD-PARTY-NOTICES` file
  generated with a pinned `cargo-about` (MIT/Apache attribution for shipped
  binaries) and includes it in the `.dmg` resources — `cargo deny` gates
  licenses but does not produce attribution. Dry-run with the next monotonically
  numbered annotated RC tag, retain that tag permanently as the receipt anchor,
  and retain its draft/assets through final-draft verification. Delete only the
  older RC draft after final publication.
- **Accept:** rc dry run produces all artifacts + draft release, and the exact
  release app/dmg pair passes the reusable strict verifier before upload.
  **Size:** M.

#### G5 — Readiness and product docs final sync

- **Steps:** update `PRODUCTION_READINESS.md`,
  `docs/production-readiness-audit.md`, `README.md` (the "current release
  boundary" and Status sections change materially: wallet management is now
  shipped scope, execution is policy-gated opt-in),
  `docs/wallet-management-roadmap.md` (mark phases 1–9 complete for EVM
  **except swap steps, deferred per D-13** — phases 8–9 textually include
  swaps, so an unqualified "complete" would make the docs lie;
  phase 10 = post-1.0), `docs/catchup-plan.md` (absorbed), and
  `docs/architecture.md`. Keep D-4 claim wording. Point "Current Plan Of
  Record" at this file. Record F5 dispositions.
- **Accept:** no doc contradicts another; §1 deliverables verifiable from
  docs alone. **Size:** M.

---

### Phase H — Final gate and ship

#### H1 — Release candidate verification (all must pass, in order)

Historical RC commit: `a22a98a` (the `main` HEAD at that time). The G4 rc dry-run ran on it as
`v1.0.0-rc.1` (release.yml run 29071519514, all four jobs green) and was
reversed (tag + draft release deleted); the five release assets were
checksum-verified locally against SHA256SUMS. Autonomous items are checked
below; the remaining items are operator human-gates.

> **Historical RC snapshot:** every receipt below describes `a22a98a`. The
> 2026-07-10 auth/payment/queue/release hardening supersedes it; none of these
> checks certifies the current line. A fresh RC must rerun H1 at one new commit.

> **RC3 failure:** `v1.0.0-rc.3` at `0a97c18` passed the legacy workflows and
> produced checksum-valid assets, but its macOS app had no bundle resource seal
> and failed `codesign --verify --deep --strict`. Its F4, doctor, asset, and any
> install receipts are void for final promotion. At that point RC4 became the
> next candidate, contingent on protected-main signing-remediation gates.

> **RC4 failure:** `v1.0.0-rc.4` at `f73b861` contains the signing remediation,
> but its F6 evidence validator accepted any numeric chain other than Sepolia
> as the L2 and represented the two-transaction gas-top-up chain with one hash.
> Its queue also treated `sent` (broadcast, unconfirmed) as prerequisite
> success. Preserve RC4 as immutable failed-contract evidence; no RC4 operator
> receipt can promote a final tag.

> **RC5 draft evidence:** `v1.0.0-rc.5` peels to pre-hardening `origin/main`
> commit `7e04743`. Release run `29248938476` passed all six jobs and produced the six
> expected draft assets. The GitHub Release remains unpublished. RC5 does not
> include the later PostCSS, ERC-5564 interoperability, session/broadcast race,
> discovery lifecycle, and FIDO2 causal-recovery hardening, so neither its
> workflow nor any same-SHA receipts certify the current line. After protected
> merge and CI, the next eligible candidate is the retained, annotated
> `v1.0.0-rc.6`.

- [ ] Fresh clone of `main` at RC6; `./scripts/check-release.sh` passes there.
      (The current hardening checkout has not yet completed this gate.)
- [ ] CI green on the RC6 commit, both legs. (No qualifying RC6 run yet.)
- [ ] F4 soak receipts (standard + chaos) reference the RC6 SHA. (No
      qualifying RC6 receipt yet.)
- [ ] F6 testnet receipts record five transactions for the four core execution
      families, including both confirmed gas-chain legs. (No qualifying RC6
      receipt yet; funded testnet access is required.)
- [ ] F7 upgrade-path tests green: 0.1-era fixture dir boots and migrates on
      the RC6 build; 0.1-era snapshot restores. (The tests remain in the source
      gate but must rerun at the RC6 SHA.)
- [ ] Desktop `.dmg` from RC6 strictly verifies, installs, and reaches the unlock
      screen on a machine without a dev toolchain. (No RC6 artifact exists yet.)
- [ ] `sigillum doctor` passes on each supported host at the RC6 SHA. (No
      qualifying RC6 receipt yet.)
- [~] A full local walkthrough of the completion bar: import a seed →
      multi-chain scan → review inventory/risk → generate plan → approve →
      execute against a local mock provider → audit trail complete. (execute→audit
      backend CI-proven — dependency_chain_executes_in_order_with_full_audit_trail,
      enqueue_step_happy_path_persists_job_marker_evidence_hash_and_audit,
      eth_seed_jobs_are_gate_driven_and_execute_once_gates_pass via spawn_mock_evm_provider,
      chaos_kill_in_flight_plan_step_resumes_terminal_without_duplication; UI click-through = operator acceptance)
- [~] CHANGELOG release-candidate notes are dated and explicitly state that no
      final release exists; G5 docs merged. (The hardening candidate still
      needs full gate, merge, CI, and fresh-RC evidence.)

#### H2 — Tag and release

```bash
(
  set -euo pipefail

  REPO=caelator/sigillum
  FINAL_TAG=v1.0.0
  RC_TAG=v1.0.0-rc.N # replace with the receipt-bearing retained RC
  EVIDENCE_BUNDLE="${EVIDENCE_BUNDLE:?set the absolute path to the sanitized release evidence archive}"
  EVIDENCE_NAME="$(basename -- "${EVIDENCE_BUNDLE}")"
  test "${EVIDENCE_NAME}" = "sigillum-v1.0.0-release-evidence.tar.gz"
  test -f "${EVIDENCE_BUNDLE}"

  git fetch --prune --tags origin
  RC_REFS="$(git ls-remote --exit-code --tags origin \
    "refs/tags/${RC_TAG}" "refs/tags/${RC_TAG}^{}")"
  RC_TAG_OBJECT="$(awk -v ref="refs/tags/${RC_TAG}" \
    '$2 == ref { print $1 }' <<< "${RC_REFS}")"
  RC_SHA="$(awk -v ref="refs/tags/${RC_TAG}^{}" \
    '$2 == ref { print $1 }' <<< "${RC_REFS}")"
  [[ "${RC_TAG_OBJECT}" =~ ^[0-9a-f]{40}$ ]]
  [[ "${RC_SHA}" =~ ^[0-9a-f]{40}$ ]]
  test "${RC_TAG_OBJECT}" != "${RC_SHA}"

  git switch --detach "${RC_SHA}"
  test -z "$(git status --porcelain)"
  test "$(git rev-parse origin/main)" = "${RC_SHA}" || {
    echo "main moved beyond ${RC_TAG}; create and qualify a new RC" >&2
    exit 1
  }
  bash ./scripts/check-release-tag-contract.sh \
    "${RC_TAG}" "${RC_SHA}" origin "${RC_TAG_OBJECT}"
  bash ./scripts/check-release-evidence-bundle.sh \
    "${EVIDENCE_BUNDLE}" "${RC_TAG}" "${RC_SHA}" "${RC_TAG_OBJECT}"
  EVIDENCE_SHA256="$(shasum -a 256 "${EVIDENCE_BUNDLE}" | awk '{print $1}')"
  [[ "${EVIDENCE_SHA256}" =~ ^[0-9a-f]{64}$ ]]
  ./scripts/check-release.sh

  # The gate is long: refresh and reassert every code and evidence identity
  # immediately before creating the immutable final tag.
  git fetch --prune --tags origin
  test "$(git rev-parse HEAD)" = "${RC_SHA}"
  test -z "$(git status --porcelain)"
  test "$(git rev-parse origin/main)" = "${RC_SHA}" || {
    echo "main moved during the gate; create and qualify a new RC" >&2
    exit 1
  }
  bash ./scripts/check-release-tag-contract.sh \
    "${RC_TAG}" "${RC_SHA}" origin "${RC_TAG_OBJECT}"
  bash ./scripts/check-release-evidence-bundle.sh \
    "${EVIDENCE_BUNDLE}" "${RC_TAG}" "${RC_SHA}" "${RC_TAG_OBJECT}"
  test "$(shasum -a 256 "${EVIDENCE_BUNDLE}" | awk '{print $1}')" = \
    "${EVIDENCE_SHA256}"

  if git ls-remote --exit-code --tags --refs origin "refs/tags/${FINAL_TAG}"; then
    # Safe resume after an interrupted H2: the immutable existing tag must
    # already match the exact RC and evidence binding.
    bash ./scripts/check-release-tag-contract.sh \
      "${FINAL_TAG}" "${RC_SHA}" origin
    EXISTING_FINAL_OBJECT="$(git ls-remote --exit-code --tags --refs origin \
      "refs/tags/${FINAL_TAG}" | awk '{print $1}')"
    test "$(git cat-file tag "${EXISTING_FINAL_OBJECT}" |
      sed -n 's/^Release-Evidence-File: //p')" = "${EVIDENCE_NAME}"
    test "$(git cat-file tag "${EXISTING_FINAL_OBJECT}" |
      sed -n 's/^Release-Evidence-SHA256: //p')" = "${EVIDENCE_SHA256}"
  else
    test "$?" -eq 2 # exit 2 means the exact remote tag is absent
    git tag -a "${FINAL_TAG}" "${RC_SHA}" \
      -m "Sigillum 1.0.0 — Local-first wallet-management workstation" \
      -m "Release-Evidence-File: ${EVIDENCE_NAME}" \
      -m "Release-Evidence-SHA256: ${EVIDENCE_SHA256}"
    git push origin "refs/tags/${FINAL_TAG}:refs/tags/${FINAL_TAG}"
  fi

  # Wait for the exact final-tag workflow and require all six release jobs.
  FINAL_RUN_JSON=""
  for _ in {1..120}; do
    FINAL_RUN_JSON="$(
      gh run list -R "${REPO}" --workflow release.yml --event push --limit 20 \
        --json databaseId,headBranch,headSha,status,conclusion,url,createdAt |
        jq -c --arg tag "${FINAL_TAG}" --arg sha "${RC_SHA}" '
          [.[] | select(
            .headBranch == $tag and
            .headSha == $sha and
            (.status != "completed" or .conclusion == "success"))]
          | sort_by(.createdAt) | last // empty'
    )"
    [[ -n "${FINAL_RUN_JSON}" ]] && break
    sleep 5
  done
  [[ -n "${FINAL_RUN_JSON}" ]]
  FINAL_RUN_ID="$(jq -r '.databaseId' <<< "${FINAL_RUN_JSON}")"
  gh run watch -R "${REPO}" "${FINAL_RUN_ID}" --exit-status
  FINAL_RUN_RESULT="$(gh run view -R "${REPO}" "${FINAL_RUN_ID}" \
    --json headBranch,headSha,conclusion,url,jobs)"
  jq -e --arg tag "${FINAL_TAG}" --arg sha "${RC_SHA}" '
    .headBranch == $tag and
    .headSha == $sha and
    .conclusion == "success" and
    (["release-contract", "verify (ubuntu-24.04)", "verify (macos-15)",
      "artifacts-macos", "artifacts-linux", "release"] -
      [.jobs[] | select(.conclusion == "success") | .name] | length == 0) and
    all(.jobs[]; .conclusion == "success")
  ' <<< "${FINAL_RUN_RESULT}" >/dev/null

  # Require the exact draft and independently verify every generated asset.
  FINAL_RELEASE="$(gh api "repos/${REPO}/releases?per_page=100" |
    jq -c --arg tag "${FINAL_TAG}" '
      [.[] | select(.tag_name == $tag)]
      | if length == 1 then .[0] else error("expected one final release") end')"
  jq -e '.draft == true and .published_at == null' \
    <<< "${FINAL_RELEASE}" >/dev/null

  VERIFY_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sigillum-final-release.XXXXXX")"
  trap 'rm -rf "${VERIFY_DIR}"' EXIT
  EXPECTED_ASSETS="${VERIFY_DIR}/expected-assets"
  REMOTE_ASSETS="${VERIFY_DIR}/remote-assets"
  printf '%s\n' \
    "SHA256SUMS" \
    "Sigillum-v1.0.0-macos-aarch64.app.zip" \
    "Sigillum-v1.0.0-macos-aarch64.dmg" \
    "THIRD-PARTY-NOTICES.txt" \
    "sigillum-cli-v1.0.0-linux-x86_64.tar.gz" \
    "sigillum-cli-v1.0.0-macos-aarch64.tar.gz" |
    LC_ALL=C sort > "${EXPECTED_ASSETS}"
  EVIDENCE_ASSET_COUNT="$(jq -r --arg name "${EVIDENCE_NAME}" \
    '[.assets[] | select(.name == $name)] | length' <<< "${FINAL_RELEASE}")"
  test "${EVIDENCE_ASSET_COUNT}" -le 1
  jq -r --arg name "${EVIDENCE_NAME}" \
    '.assets[] | select(.name != $name) | .name' \
    <<< "${FINAL_RELEASE}" | LC_ALL=C sort > "${REMOTE_ASSETS}"
  cmp -s "${EXPECTED_ASSETS}" "${REMOTE_ASSETS}"

  while IFS=$'\t' read -r asset_name asset_id; do
    gh api -H 'Accept: application/octet-stream' \
      "repos/${REPO}/releases/assets/${asset_id}" > "${VERIFY_DIR}/${asset_name}"
  done < <(jq -r --arg name "${EVIDENCE_NAME}" \
    '.assets[] | select(.name != $name) | [.name, (.id | tostring)] | @tsv' \
    <<< "${FINAL_RELEASE}")
  (
    cd "${VERIFY_DIR}"
    shasum -a 256 --check SHA256SUMS
  )

  # Upload without replacement, re-download, and compare with the protected
  # tag object immediately before publication.
  if [[ "${EVIDENCE_ASSET_COUNT}" -eq 0 ]]; then
    gh release upload -R "${REPO}" "${FINAL_TAG}" "${EVIDENCE_BUNDLE}"
  fi
  FINAL_RELEASE="$(gh api "repos/${REPO}/releases?per_page=100" |
    jq -c --arg tag "${FINAL_TAG}" '
      [.[] | select(.tag_name == $tag)]
      | if length == 1 then .[0] else error("expected one final release") end')"
  EVIDENCE_ASSET_ID="$(jq -r --arg name "${EVIDENCE_NAME}" '
    [.assets[] | select(.name == $name)]
    | if length == 1 then .[0].id else error("expected one evidence asset") end' \
    <<< "${FINAL_RELEASE}")"
  gh api -H 'Accept: application/octet-stream' \
    "repos/${REPO}/releases/assets/${EVIDENCE_ASSET_ID}" > \
    "${VERIFY_DIR}/${EVIDENCE_NAME}"

  bash ./scripts/check-release-tag-contract.sh \
    "${FINAL_TAG}" "${RC_SHA}" origin
  FINAL_TAG_OBJECT="$(git ls-remote --exit-code --tags --refs origin \
    "refs/tags/${FINAL_TAG}" | awk '{print $1}')"
  TAG_EVIDENCE_SHA256="$(git cat-file tag "${FINAL_TAG_OBJECT}" |
    sed -n 's/^Release-Evidence-SHA256: //p')"
  test "${TAG_EVIDENCE_SHA256}" = "${EVIDENCE_SHA256}"
  test "$(shasum -a 256 "${VERIFY_DIR}/${EVIDENCE_NAME}" | awk '{print $1}')" = \
    "${TAG_EVIDENCE_SHA256}"

  # Re-fetch and reverify all seven live draft assets immediately before the
  # publish mutation, closing the draft-asset replacement window.
  PREPUBLISH_RELEASE="$(gh api "repos/${REPO}/releases?per_page=100" |
    jq -c --arg tag "${FINAL_TAG}" '
      [.[] | select(.tag_name == $tag)]
      | if length == 1 then .[0] else error("expected one final release") end')"
  jq -e '.draft == true and .published_at == null' \
    <<< "${PREPUBLISH_RELEASE}" >/dev/null
  PREPUBLISH_EXPECTED="${VERIFY_DIR}/prepublish-expected-assets"
  PREPUBLISH_ACTUAL="${VERIFY_DIR}/prepublish-actual-assets"
  printf '%s\n' \
    "${EVIDENCE_NAME}" \
    "SHA256SUMS" \
    "Sigillum-v1.0.0-macos-aarch64.app.zip" \
    "Sigillum-v1.0.0-macos-aarch64.dmg" \
    "THIRD-PARTY-NOTICES.txt" \
    "sigillum-cli-v1.0.0-linux-x86_64.tar.gz" \
    "sigillum-cli-v1.0.0-macos-aarch64.tar.gz" |
    LC_ALL=C sort > "${PREPUBLISH_EXPECTED}"
  jq -r '.assets[].name' <<< "${PREPUBLISH_RELEASE}" |
    LC_ALL=C sort > "${PREPUBLISH_ACTUAL}"
  cmp -s "${PREPUBLISH_EXPECTED}" "${PREPUBLISH_ACTUAL}"

  PREPUBLISH_DIR="${VERIFY_DIR}/prepublish"
  mkdir -p "${PREPUBLISH_DIR}"
  while IFS=$'\t' read -r asset_name asset_id; do
    gh api -H 'Accept: application/octet-stream' \
      "repos/${REPO}/releases/assets/${asset_id}" > \
      "${PREPUBLISH_DIR}/${asset_name}"
  done < <(jq -r '.assets[] | [.name, (.id | tostring)] | @tsv' \
    <<< "${PREPUBLISH_RELEASE}")
  (
    cd "${PREPUBLISH_DIR}"
    shasum -a 256 --check SHA256SUMS
  )
  TAG_EVIDENCE_SHA256="$(git cat-file tag "${FINAL_TAG_OBJECT}" |
    sed -n 's/^Release-Evidence-SHA256: //p')"
  test "${TAG_EVIDENCE_SHA256}" = "${EVIDENCE_SHA256}"
  test "$(shasum -a 256 "${PREPUBLISH_DIR}/${EVIDENCE_NAME}" |
    awk '{print $1}')" = "${TAG_EVIDENCE_SHA256}"

  FINAL_RELEASE_ID="$(jq -r '.id' <<< "${PREPUBLISH_RELEASE}")"
  PUBLISHED_RELEASE="$(gh api --method PATCH \
    "repos/${REPO}/releases/${FINAL_RELEASE_ID}" \
    -F draft=false -F prerelease=false -f make_latest=true)"
  jq -e '.draft == false and .prerelease == false and .published_at != null' \
    <<< "${PUBLISHED_RELEASE}" >/dev/null
)
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
- [x] A1 GLM convergence blockers verified/closed
- [x] A2 PR green on both CI legs
- [x] A3 Merged to main

Phase B — Workspace hygiene
- [x] B1 publish=false + README dependency framing
- [x] B2 expect() burn-down (≤4 justified sites)
- [x] B3 dead_code allows resolved
- [x] B4 test floors for untested crates

Phase C — Desktop productization
- [x] C1 real icon set
- [x] C2 bundling enabled (.app/.dmg)
- [~] C3 fail-closed env-gated signing and explicit full-bundle ad-hoc default
      (landed and RC5 release-workflow proven; final promotion still needs the
      same proof at RC6 with all current hardening)
- [~] C4 strict source + mounted-dmg verification and negative regressions in
      the release gate (landed and RC5 release-workflow proven; final promotion
      still needs the same proof at RC6 with all current hardening)
- [x] C5 boot helpers extracted + tested
- [x] C6 desktop docs
- [ ] C7 operator console UX redesign (user-directed)

Phase D — Operator-surface parity
- [x] D1 CLI: transit, evm read-only, wallets read/derive, compartment list
- [x] D2 docs/operator-surface-parity.md complete

Phase E — Automation & recovery
- [x] E0 macOS loopback skip → runtime sandbox probe
- [x] E1 queue state model extended (operator_action_required, deferred semantics)
- [x] E2 kill-mid-write replay tests
- [x] E3 categorized maintenance summaries
- [x] E4 chain_id persisted on allocations
- [x] E5 destructive-flow recovery complete

Phase W — Wallet-management completion
- [x] W1.1 chain registry (built-ins + custom + Permit2 override)
- [x] W1.2 chain_id legacy defaults + UI/CLI surfacing (field already exists)
- [x] W1.3 multi-chain scan orchestration
- [x] W2 typed domain model (wire-compatible enums)
- [x] W3.1 block-range checkpoints for log scans
- [x] W3.2 ERC-1155 batch e2e fixture (decoding already implemented)
- [x] W3.3 local token registry import
- [x] W3.4 NFT metadata cache + local spam heuristics
- [x] W3.5 on-chain last-activity signals
- [x] W4 DeFi exit adapters: ERC-4626, UniV2 LP, Lido unwrap (Aave v3 exists)
- [x] W5 Merkle claim execution enablement
- [x] W6.1 fund_gas steps with linkage rule
- [x] W6.2 dynamic fees in planning/preflight
- [x] W6.3 policy-driven hot floor/refill
- [x] W6.4 step dependency ordering
- [x] W7.1 execution policy gates + preemptive kill-switch latch
- [x] W7.2 plan-step queue payloads + enqueue validation
- [x] W7.3 seed-wallet signing execution
- [x] W7.4 durable exact-byte submission, nonces, receipts, failure classes
- [x] W7.5 linkage enforcement parity at execution
- [x] W8 treasury automation (overflow/refill, hysteresis)

Phase F — Assurance
- [x] F1 adversarial coverage: receiving/treasury/chains/execution
- [x] F2 nightly deep-fuzz
- [x] F3 chaos soak mode (+ in-flight job assertion)
- [ ] F4 RC soak receipts per supported host
- [x] F5 execution-path security review
- [ ] F6 testnet execution receipts (human-in-the-loop)
- [x] F7 0.1→1.0 data-dir upgrade verification

Phase G — Release engineering
- [x] G1 CHANGELOG.md
- [x] G2 docs/stability.md
- [x] G3 version bump to 1.0.0
- [~] G4 release workflow (historical dry run validated on a22a98a; RC3 exposed
      a signature false positive; RC4 exposed an evidence-contract false
      positive and unconfirmed-dependency execution; RC5 proved those
      remediations and produced six draft assets, while RC6 must cover the later
      hardening)
- [x] G5 readiness + product docs final sync

Phase H — Ship
- [~] H1 RC verification checklist (RC3 void after bundle-signature failure;
      RC4 void after the F6 schema and dependency-finality failures; RC5 is an
      unpublished older-code draft; all source, release, F4/F6, clean-machine,
      doctor, C7, and evidence-bundle receipts must bind RC6)
- [ ] H2 v1.0.0 tagged, artifacts published (human gate — operator go)
- [ ] H3 post-release bump + planning issue

## 6. Work log

(append one line per completed task: `YYYY-MM-DD <task-id> <commit-sha> <result>`)

- 2026-07-02 A1 (see below) B1 verified: claim scoped in README "Privacy Model —
  Scope and Limitations" + architecture.md "Privacy & Linkage Model". B2
  verified: onboarding opt-in implemented in `ui/src/views/setup.ts:585-604`
  (explicit enable with fail-closed copy, or defer with pointer to Treasury
  policy). No doc gaps found. Pre-existing `cargo fmt` drift in 4 files fixed
  mechanically in the same pass.
- 2026-07-02 A2-prep: pre-existing red baseline fixed before PR — 3 clippy
  errors (params-struct refactor in `deposits.rs`, struct-literal test init in
  `planner.rs`, via codex-exec); `quinn-proto` bumped 0.11.14→0.11.15
  (RUSTSEC-2026-0185); RUSTSEC-2026-0194/0195 (quick-xml via plist←tauri, no
  upstream fix available) temporarily ignored in `.cargo/audit.toml` +
  `deny.toml` with removal notes — see production-readiness-audit.md.
- 2026-07-02 A2: Ubuntu CI caught a real desktop-branch bug the macOS legs
  cannot see — stale diagnostics fixture in `sigillum-client/src/tests.rs`
  missing the three new `idle_lock_*` DTO fields. Root cause of the blind
  spot: all client loopback tests carry
  `#[cfg_attr(target_os = "macos", ignore = "sandbox blocks loopback bind")]`,
  a blanket platform skip — macOS CI runners CAN bind loopback, so the skip
  should be sandbox-detecting, not platform-blanket. **Follow-up queued for
  Phase B (B4 scope):** replace the blanket macOS ignore with a runtime
  sandbox probe so macOS CI exercises these tests too.
- 2026-07-02 A2 (cont.): three further Ubuntu-only gate hardenings —
  runtime smoke now prebuilds the CLI and allows a 120s readiness window
  (`cargo run --quiet` was compiling silently through the old 20s window);
  browser-smoke temp-profile cleanup is best-effort (Chromium helpers
  outlive the killed main process); browser-smoke reauth interaction
  retries once (re-render race on slow runners) and CI now uploads failure
  screenshots/DOM snapshots as workflow artifacts.
- 2026-07-03 A2+A3 fd2b35b: PR #1 green on both legs, squash-merged to
  main, branch deleted. Phase A complete; B/C/D are unblocked and may run
  in parallel.
- 2026-07-03 Wave 1 (B1-B4, C1-C3, C5, C6, D1, D2) implemented by eight
  parallel codex-exec agents in isolated worktrees and integrated on
  wave/1-bcd. Highlights: KDF/daemon-init/client/gateway-config made
  fallible (B2, 3 justified expect sites remain); desktop bundles as
  ad-hoc-signed .app/.dmg with generated icons (C1-C3); desktop boot
  helpers extracted with 6 tests (C5); 14 new scriptable CLI commands, no
  sign/send/broadcast (D1, 53 smoke tests); 121/121 routes covered in the
  parity matrix (D2); serde roundtrip anchor for W2 (B4). Integration
  fixups: two test callers adapted to fallible SigillumClient::new; parity
  doc rows flipped from planned to landed. Deviation note: wave tasks
  batched into one PR (per-task worktree branches preserved) to bound CI
  wall-clock; C4 follows as its own PR; C7 (UX redesign) pending operator
  screenshot review.
- 2026-07-03 E1 40cdf5f: queue state model extended with
  `operator_action_required`, queue store schema v2 accepting v1 envelopes,
  non-runnable state/recovery/count tests, API/UI/CLI-visible counts/status,
  and a documented legacy `deferred` decision: no active producer found, keep
  legacy-only `deferred` → `blocked` normalization with a recovery reason.
- 2026-07-03 E2 cb2c987: added restart/replay coverage for
  `profiles.json`, `deposits.json`, and `queue.json` across orphaned atomic
  temp files, renamed live files with stale `.bak`, and truncated live files;
  tests assert public-route restore, backup refresh, quarantine artifacts, and
  startup health readiness.
- 2026-07-04 E3 local: added `failures_by_cause` to queue-process and
  maintenance responses, daemon failure-cause classification
  (provider_error / policy_block / insufficient_gas / validation + unknown),
  UI result summaries, and DTO/client/route/UI tests. Targeted gates passed:
  API response tests, daemon cause tests, daemon maintenance route test, client
  maintenance helper test, UI smoke tests, and UI bundle build. Full
  `./scripts/check-release.sh` passed.
- 2026-07-03 Wave 1 merged: PR #2 → main 6cfcdce (merge commit, per-task
  history preserved), CI green both legs. C4 remains (Wave 2); C7 complete
  on its branch pending operator sign-off. Execution runbook created at
  docs/execution-runbook-1.0.md (current-state ledger, proven multi-agent
  method, wave sequencing for E/W/F/G/H, triage + recovery procedures);
  E0 added under Phase E from the A2 lesson.
- 2026-07-03 C4 9357245: added `scripts/check-desktop.sh`, wired it into
  `scripts/check-release.sh` after browser smoke, installed `tauri-cli` on the
  macOS CI leg, and documented `SIGILLUM_SKIP_DESKTOP_BUNDLE`. Local desktop
  check produced debug `.app` + `.dmg`, verified `Signature=adhoc`, and took
  136s; full release gate passed with the new step included.
- 2026-07-03 E0 b30f781: replaced macOS blanket ignores on client loopback
  tests and the sibling daemon EVM loopback tests with runtime bind probes that
  early-return only when `127.0.0.1:0` cannot bind. Local
  `cargo test -p sigillum-client -- --nocapture` ran 21 tests with 0 ignored;
  full `./scripts/check-release.sh` passed.
- 2026-07-04 E5 local: added named destructive-flow crash-point coverage for
  compartment init, compartment removal/replacement, and snapshot restore
  recovery; documented the per-flow crash guarantees in `docs/backup.md`.
  Focused `cargo test -p sigillum-daemon --test crash_recovery` passed
  23/23 tests; full `./scripts/check-release.sh` passed.
- 2026-07-04 W2 local: converted wallet inventory/action-plan wire domains
  to serde-compatible typed enums with `Other(String)` forward compatibility,
  updated inventory producers/consumers and UI contracts, and extended the B4
  roundtrip anchor with exact literal and legacy JSON compatibility coverage.
  Focused API/daemon/UI tests passed; `cargo test --workspace` passed; full
  `./scripts/check-release.sh` passed.
- 2026-07-04 W1.1+W1.2 local: added schema-versioned built-in/custom chain
  registry, `/api/chains` aliases, CLI `sigillum api chains`, UI registry
  controls, provider selfcheck warnings, per-chain Permit2 overrides, and
  explicit legacy mainnet defaults plus registry labels for inventory rows.
  Focused API/daemon/CLI/UI tests passed; manual CLI chain registry smoke
  passed; full `./scripts/check-release.sh` passed.
- 2026-07-04 W1.3 PR #13, merge `c97eda0`: added explicit
  all-configured-chain EVM inventory scans, chain-tagged discovery jobs,
  CLI/UI scan controls, CLI/UI plan chain
  surfacing, and single-chain consolidation-plan generation with optional
  `chain_id` filtering. Focused API/daemon/CLI/UI tests passed; full gate
  and CI passed.
- 2026-07-04 W3.1 local: added per-address/chain/topic transfer-log block
  cursors for ERC-20, ERC-721, and ERC-1155 discovery, persisted inventory
  schema v13 with v12 compatibility, resumable disjoint-range scans after a
  canceled job, CLI/UI cursor surfacing, and `eth_blockNumber` RPC support.
  Focused cursor/backcompat tests, Rust API/daemon/CLI gates, and UI
  typecheck/test/build passed.
- 2026-07-08 W3.2 a18f347: ERC-1155 TransferBatch end-to-end mock fixture
  through the real scan route; zero-balance ids filtered; test-only diff.
- 2026-07-08 W3.3 a8a75e7: local token registry import — store
  `sigillum.token-registry` v1, `/api/inventory/token-registry` routes,
  `probe_token_registry` scan flag, UI/CLI surfaces, D-15 network-path
  rejection at import, holdings provenance `token_registry:<list-name>`.
- 2026-07-08 W3.4 3379af8: opt-in NFT metadata fetch pipeline writing
  provenance (URI, fetch time, content hash) into the existing
  `nft_metadata_cache`; airdrop/lookalike spam heuristics with recorded
  reasons; never-auto-hidden suspicious bucket in UI; API-only (no-CLI)
  parity decision recorded (row 33); zero-traffic-without-opt-in negative
  tests.
- 2026-07-08 W3.5 75f7041: `last_activity_block` derived from observed
  transfer-log/announcement evidence only (monotonic, never cursor
  progress); per-chain `dormancy_block_window` on registry entries;
  dormancy classified by block window with block evidence in risk
  findings; UI/CLI surfacing.
- 2026-07-08 W6.2 4cbf58f: `simulation_freshness_secs` policy field
  (default 900); estimated-vs-static fee basis recorded as step evidence;
  stale approvals downgrade simulation to `required`; estimation RPC
  failure fails closed.
- 2026-07-08 W6.3 47975fa: `hot_floor_wei_hex`/`hot_target_wei_hex`
  replace the 1 ETH planner hardcode; migration defaults both to
  `0xde0b6b3a7640000` with a byte-identical legacy routing test;
  `floor <= target` validated; UI/CLI policy editor fields.
- 2026-07-08 W6.4 f543e1f: `sequence`/`depends_on` on
  ConsolidationPlanStep with serde-default backcompat; export emits
  dependency order, refuses blocked/skipped/missing dependencies
  fail-closed with named reasons, rejects cycles.
- 2026-07-08 Wave 5 integrated on wave/5-discovery-planner (8c8fca5):
  wallet-inventory schema chain reconciled to v15 (v14 = W3.4+W3.5 fields,
  v15 = W6.2/W6.3 fields); parity doc recounted at 132 route
  registrations / 33 family rows; quick-xml RUSTSEC-2026-0194/0195
  ignores removed from `.cargo/audit.toml` + `deny.toml` after
  `cargo update` reached quick-xml 0.41.0 (audit + deny green with empty
  ignore lists).
- 2026-07-09 W4 f014959/b3057cb/b3257be: D-11 exit adapter set complete —
  `erc4626-redeem` (maxRedeem/convertToAssets detection, redeem exit,
  expected-assets-out evidence), `lido-wsteth-unwrap` (unwrap only, stETH
  surfaces as review_asset per D-11), `uniswap-v2-remove-liquidity`
  (dependency-ordered approve→removeLiquidity via W6.4, plan-time
  amountMins from reserves with 0.5% haircut, per-chain operator-supplied
  router, no hardcode). Each adapter: detection, preflight-pass,
  preflight-revert→blocked, gas-shortfall tests; no-adapter positions
  keep the review_asset fallback.
- 2026-07-09 W5 393d7fd: `allow_claim_execution` policy opt-in (default
  false, wallet-inventory schema v16) gating `claim_execution_disabled`
  behind policy enabled + merkle-distributor-v1 + simulation passed +
  trusted-or-reviewed claim contract + step approval; per-gate negative
  tests plus a policy-off byte-identical regression; gate re-evaluated
  fail-closed at approval and after every simulation; W7.3 revert rule
  (operator_action_required — proof may be consumed) documented for the
  execution phase.
- 2026-07-09 Wave 6 integrated on wave/6-adapters-claims: both branches'
  independent v15→v16 store bumps collapsed into one v16 (serde-default
  envelope; both legacy-load tests pass); simulation.rs import union;
  claim_gate fixture gained W4's exit_* fields; full targeted battery
  green (315 daemon unit tests + all integration suites, UI 56/56,
  architecture/fmt/clippy clean).
- 2026-07-09 W6.1 4b9bba2..a4d607b: planner-emitted `fund_gas` top-up
  steps (sponsor → source, gas×1.5 capped by `max_gas_topup_wei_hex`,
  ordered before dependents via W6.4) behind `allow_gas_topups` (default
  false, schema v17 + legacy test); common-funder linkage added to
  `analyze_plan_linkage` — cross-party sponsor funding always warns,
  hard-blocks under `block_cross_party_linkage` (matrix-tested at
  generation and approval); dependents simulate via explicit
  `pending_gas_topup_wei_hex` credit from unblocked top-ups only; no
  sponsor / insufficient balance / policy off preserve today's gas
  blockers byte-identically; README + architecture privacy caveat
  narrowed to manual gas funding. Reviewed behavior change: seed control
  reserves (sponsor/hot/treasury) are no longer generic sweep sources —
  prevents plans from draining the sponsor gas float (test-covered; W8
  builds on this).

> **Historical execution entries:** the W7.1/W7.3/W7.4 lines below remain
> accurate records of their cited 2026-07-09 commits. Their pause and
> submission mechanics were superseded on the 2026-07-10 hardening line by the
> AtomicBool preemption latch and the durable `prepared` ->
> `submitted_unknown` exact-byte protocol described in the normative W7 text.

- 2026-07-09 W7.1 57cd960..e0624b8: execution policy surface — master
  `allow_plan_execution` + per-family sweep/revoke/exit gates +
  `execution_paused` kill switch + `max_fee_per_gas_cap_hex` (schema v18,
  all fail-closed defaults); reusable gate evaluation re-reads persisted
  policy at enqueue and drain chokepoints (policy flip between → blocked
  with named reason); pause halts the drain loop immediately
  (mid-drain test) via `POST /api/queue/pause|resume`, UI pause
  control/banner, CLI; typed gate-flip audit events carry old/new value
  + session fingerprint (never the raw token); per-gate enqueue-time and
  execution-time negatives; defaults-off byte-identical regressions;
  EthSeed* hard block untouched; stealth families exempt by construction.
  Parity doc 134 registrations / 135 endpoints. UI/docs step implemented
  by a Sonnet agent (operator-directed fallback after a Codex CLI
  usage-limit block).
- 2026-07-09 W7.2 080e56c..d2dd301: `PlanStepExecution` queue payload
  (queue store v3) carrying plan/step/chain ids, derivation evidence,
  preflight-prepared call parameters, fee basis, and a canonical
  simulation-evidence SHA-256 that W7.3 verifies before signing;
  `POST /api/plans/enqueue-step` (explicit confirm flag) and
  `enqueue-plan` (typed confirmation `EXECUTE {n} PLAN STEPS TOTAL {wei}
  WEI`); enqueue re-validates everything server-side — approval,
  simulation passed + fresh (stale demands re-simulate), unblocked,
  W7.1 gates, treasury destination/step/plan caps via pre-planted
  BlockPlanCap/BlockUnsimulated policy actions, linkage, W5 claim gate,
  W6.1 top-up opt-in — each with a named refusal and negative test;
  idempotent enqueue markers (inventory v19) with failed→reapprove via
  operator_action_required; dependency chaining carries prerequisite job
  ids; drain-time hard block "plan-step execution is not enabled yet"
  regardless of gates (lifts in W7.3); stealth families byte-identical;
  UI execute affordances gate-aware with typed-confirmation dialog; CLI
  `sigillum api plans enqueue-step|enqueue-plan`. 29 new integration
  tests. Implemented directly by a Sonnet agent under the operator's
  standing fallback directive.
- 2026-07-09 W7.3 03eb022..1d8e63a: signing execution lands — new
  `service/queue/plan_steps.rs` (+signing.rs) executes PlanStepExecution
  jobs in fixed order: dependency check → evidence-hash re-verification
  (tamper → operator_action_required, never signed) → signer resolution
  from the unlocked compartment seed with a BlockWatchOnlySigner
  re-check → fee-cap check → sign + broadcast with typed
  signed/broadcast audit events; `seed_sends.rs` executes legacy
  EthSeed* (now gated as the sweep family; stealth stays exempt);
  same-batch dependents resolve in one drain via a job-state snapshot;
  claims revert/failure parks operator_action_required and never
  retries; keys held in Zeroizing wrappers and an explicit test
  re-derives the key and asserts its hex (and mnemonic) absent from
  audit events and persisted store bytes; full sweep→revoke→fund_gas
  chain executes in dependency order with a 3-sign/3-broadcast audit
  trail; gates-off behavior preserved. Implemented directly by a Sonnet
  agent under the operator's standing fallback directive.
- 2026-07-09 W7.5 353f9c7/e00da22: linkage parity proven at the
  plan-step enqueue path — nine-cell matrix (tagged/untagged parties ×
  destination collisions × policy on/off, incl. fund_gas common-funder
  and bulk enqueue-plan skip reasons) plus the approval→enqueue policy
  flip test mirroring the treasury-allowlist flip. The matrix exposed a
  real gap, now fixed: enqueue previously skipped linkage recomputation
  entirely when `block_cross_party_linkage` was off, leaving stale
  per-step warnings — warnings now always refresh while the hard block
  stays policy-gated (no fail-closed weakening). README + architecture
  document that execution enforces the same single-hop destination-axis
  claim as generation/approval. Implemented via codex-exec (quota
  restored) with independent re-verification by the wrapping agent.
- 2026-07-09 F1 9637893: 57 adversarial cases across receiving/treasury/
  chains/plans-enqueue (auth, transport abuse, invalid/oversized/overflow
  inputs, stale/foreign ids, replay → 409 with queue unchanged, TOCTOU
  gate/destination/linkage flips → 403); every rejection clean 4xx with
  state asserted unchanged; new tests/adversarial_execution.rs wired
  into check-adversarial.sh. Finding for F5 disposition:
  decode_quantity_hex accepts prefix-less strings on policy caps
  ("1000" persists as 0x1000 = 4096 wei).
- 2026-07-09 F2: nightly scheduled CI exports
  SIGILLUM_ADVERSARIAL_PROPTEST_CASES=1024; PRs keep the 256 default.
- 2026-07-09 F3 98182b8: SIGILLUM_SOAK_CHAOS=1 soak mode with guarded
  kill -9 cycles, next-iteration doctor+canary requirement, receipt
  cycle counts, and the W7.4-superset in-flight plan-step assertion
  recorded in the receipt; real 600s chaos run passed with 3 kill
  cycles; plain-mode receipt byte-identical apart from additive fields.
- 2026-07-09 W7.4 f972dcb..3a99d1d: execution semantics — per-source
  (address, chain) serialization with an E1-consistent transient skip
  (no legacy `deferred` wire string; dependency-chained same-batch jobs
  still resolve in one drain); broadcast-time nonce fetch with one
  re-fetch on nonce-too-low then operator_action_required; receipt
  confirmation to the chain registry's `finality_blocks` with new
  terminal `confirmed` state (queue store v4, additive receipt fields;
  `sent` = awaiting confirmation for PlanStepExecution only, legacy
  families byte-identical); bounded receipt-timeout parks with the tx
  hash (never assumes failure); failure classes — provider error →
  retrying, revert → operator_action_required (never auto-retried),
  underpriced → one fee bump within the policy cap then park; kill-9
  restart resumes receipt polling with no duplicate broadcast (mock
  counts eth_sendRawTransaction=1). Fixed a real RPC transport bug:
  JSON-RPC null results (unmined receipt) were parsed as transport
  errors. Implemented directly by a Sonnet agent under the operator's
  standing fallback directive.
- 2026-07-09 W8 a28cbae (PR #25): treasury automation — hot-overflow/refill
  steps generated in maintenance behind allow_treasury_automation (default
  false, schema v20), through the standard plan pipeline and the real W7.2
  enqueue path; hysteresis proven across 5 cycles; distinct generated/enqueued
  summaries; automation-off byte-identical. Merge also carried the F1 receiving
  adversarial flake fix that had begun failing all PRs' merge-CI.
- 2026-07-09 F1/F2/F3 1d4017c (PR #24): 57 adversarial cases, nightly 1024-case
  fuzz, chaos soak with W7 in-flight assertion (recorded in earlier work log).
- 2026-07-09 F5 (PR #30): execution-path security review — five threat
  dispositions + regression tests in production-readiness-audit.md; fixed the
  F1 hex-prefix finding (fail-closed 0x-prefix validation on all six policy cap
  fields; global decode_quantity_hex left tolerant per impact review); findings
  N1 (cross-plan linkage gap), N2 (typed-confirmation is fat-finger not attacker
  barrier), N3 (inert canonicalization collision) documented. No critical vuln.
- 2026-07-09 F7 4bfa48c (PR #29): 0.1→1.0 upgrade verification — committed
  0.1-era fixture + generator; proves all 15 schema bumps compose on one real
  data dir (queue v1→v4, inventory v11→v20, deposits v1→v2, audit log→db, etc.),
  no quarantine, canaries + snapshot restore; guarantees in docs/backup.md.
- 2026-07-09 G1/G2 4788769 (PR #26): CHANGELOG.md [1.0.0] at feature
  granularity (date placeholder for H2); docs/stability.md with stable/unstable
  surface list and the D-17 residual-risk statement; linked from README.
- 2026-07-09 G4 (PR #27): tag-triggered release.yml (verify mirror + macOS/Linux
  artifacts + SHA256SUMS + THIRD-PARTY-NOTICES via pinned cargo-about + draft
  release); rc dry-run procedure documented, execution deferred to post-G3.
- 2026-07-09 Process: switched implementation to Sonnet agents mid-session after
  a Codex CLI usage-limit block (operator-directed); then to maximum
  parallelism (concurrent PRs, CI-as-authoritative-gate, batched bookkeeping).
- 2026-07-09 G3 ec01102 (PR #32): workspace version + six internal sigillum-*
  crate pins bumped 0.1.0→1.0.0; CI green both legs; main now reports 1.0.0.
- 2026-07-10 G5 0c8a168 (PR #33): final readiness/product docs sync (README,
  PRODUCTION_READINESS, production-readiness-audit, wallet-management-roadmap,
  architecture, catchup-plan) so docs describe exactly what shipped — EVM
  wallet-management phases 1–9 (swaps excepted, D-13), policy-gated execution
  default-off, D-17 residual risk, no valuation (D-16). Docs-only; both legs
  green. (Recovery note: an earlier chained push failed and stranded the commit
  on the local main checkout; re-created the branch at the commit, reset local
  main to origin, rebased conflict-free onto current main, re-pushed.)
- 2026-07-10 G4 rc dry-run VALIDATED on RC commit a22a98a: tagged v1.0.0-rc.1;
  release.yml run 29071519514 all four jobs green (verify ×2, artifacts-macos,
  artifacts-linux, release); draft release carried the .dmg, .app.zip, both
  sigillum-cli tarballs, THIRD-PARTY-NOTICES.txt, and SHA256SUMS; all five
  assets checksum-OK locally; body carried the CHANGELOG [1.0.0] section.
  Reversed cleanly (release + remote/local tag deleted); main untouched. G4
  complete.
- 2026-07-10 H1 partial (RC = a22a98a): autonomous items verified — fresh-clone
  gate (release verify jobs, both OS), CI-green-both-legs (push-CI 29071505668),
  F7 upgrade path (in-gate), execute→audit backend (mock-provider + chaos
  in-flight tests), G5 docs merged. Operator human-gates remain: F4 mac-server
  soak at the RC SHA, F6 testnet execution receipts, clean-machine .dmg install
  + unlock, `sigillum doctor` per host. The CHANGELOG date must be committed
  before the next RC tag; H2 (real v1.0.0 tag/publish) is the final human gate;
  H3 follows publish.
- 2026-07-12 RC3 FAILURE (`0a97c18`, release run 29216941593): the protected
  source gate, six-job draft workflow, asset checksums, local standard/chaos
  soak, and checksum-verified RC CLI doctor passed. Independent inspection of
  the dmg then found that the app had only a linker signature: no
  `_CodeSignature/CodeResources`, unbound `Info.plist`, unsealed resources,
  and failing `codesign --verify --deep --strict`. Preserve RC3 and its draft as
  immutable failure evidence; none of its receipts can promote a final tag.
- 2026-07-12 C3/C4/G4 RC4 remediation (this change): credential-free builds
  explicitly select Tauri identity `-`; partial/mixed signing and notarization
  inputs fail closed; one reusable verifier checks the source and read-only
  mounted-dmg apps plus CDHash parity; negative regressions cover the RC3 shape,
  tampering, identifier, symlink, path, and dmg-layout failures; the release
  artifact job runs the verifier after adding notices and before upload.
- 2026-07-12 RC4 FAILURE (`f73b861`): the signing remediation passed the
  protected-main source gate, but the F6 validator accepted arbitrary numeric
  chains as the L2 and collapsed a two-transaction gas-top-up chain to one
  hash. Runtime dependency ordering also treated broadcast-only `sent` as
  success instead of requiring `confirmed`. Preserve RC4 and its receipts as
  immutable failed-contract evidence; RC5 must fix dependency finality, require
  Ethereum Sepolia (`11155111`) plus Base Sepolia (`84532`), Arbitrum Sepolia
  (`421614`), or OP Sepolia (`11155420`), require F6 schema v2 with five
  transactions, and re-run every same-SHA gate. Historical release run
  `29230844456` completed all six jobs and its six-asset unpublished draft,
  proving the signing fix but not curing the assurance/runtime failures.
- 2026-07-30 RC5 DRAFT EVIDENCE (`7e04743`, Release run `29248938476`):
  `v1.0.0-rc.5` exactly matches pre-hardening `origin/main` commit `7e04743`;
  all six workflow jobs passed and the six expected assets remain in an unpublished draft. Preserve
  the annotated tag and draft as immutable same-SHA evidence. Later PostCSS,
  ERC-5564 interoperability, session/broadcast race, discovery lifecycle, and
  FIDO2 causal-recovery hardening invalidate RC5 for current-line promotion;
  after protected merge and CI, the next eligible candidate is RC6.
