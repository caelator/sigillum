CONVERGED: YES

CONVERGED: YES

## Verdict

The candidate plan is ready to execute. It resolves all previously-open architectural questions using the stated threat model as the decision criterion, provides a clear BLOCKING vs. non-blocking partition (exactly two blocking items, both positioning/documentation rather than code correctness), and delivers a standalone, evidence-backed CONDITIONALLY DEFENSIBLE verdict on the "consolidate without re-linking payers" claim. The remaining items are non-blocking local verifications and product-owner decisions that do not require further architectural analysis.

## Critique

1. **All Round 1 required changes are satisfied.** The four "Missing facts" are resolved in the Inference section. The verdict is explicit. The BLOCKING list is exactly two items. The defensibly assessment is standalone with a conditions table. The unlock-throttle verification is a non-blocking local check.

2. **The evidence-to-claim mapping is accurate against the supplied source.** Spot-checking the code confirms: `analyze_plan_linkage` groups sweep steps by destination and flags ≥2 distinct identities (planner.rs tests cover tagged-party, same-party-multi-source, and unattributed-distinct cases); `detect_stealth_sweep_linkage` mirrors this over stealth deposits; `apply_linkage_blockers` converts warnings to hard blockers; the generate→analyze→block→summarize ordering is correct in `generate_consolidation_plan`; `approve_consolidation_plan` re-runs linkage with a reconstructed minimal state; `enqueue_deposit_sweep_job` returns `policy_violation("cross_party_linkage")` under policy; and `block_cross_party_linkage` defaults to `false` via `unwrap_or(false)`.

3. **The BLOCKING vs. non-blocking partition is correct.** B1 (claim scoping) and B2 (default policy) block the *positioning claim*, not the *architecture*. The code is correct at both `true` and `false`; this is about what the operator sees by default and how the product is described. Everything else (loopback TCP, RPC privacy, gas-funding linkage, multi-chain, silent-drop UX gap, line caps) is genuinely non-blocking under the stated threat model.

4. **No material risks remain unverified.** The three local verification items (V1: unlock throttle details, V2: TOCTOU startup-failure loudness, V3: setup-wizard visibility of the policy) confirm implementation details of an architecture that is sound on its face — none of them could invalidate the architectural verdict.

## Required Changes

None. The plan is ready to execute.

## Revised Plan

# Sigillum — Architecture Convergence Verdict

Branch `feat/private-receiving-desktop` @ `70a087b`. Rust workspace; local-first, privacy-focused, **solo-operator** Ethereum *receiving + treasury* console.

Product thesis: receive funds from many parties using use-once addresses (fresh HD per payer, or ERC-5564 stealth) so payers can't be linked together, manage it all from one desktop interface backed by one or more BIP-39 seeds, and **consolidate without re-linking the payers**. Stated threat model: public on-chain analysis + the payers themselves (NOT a nation-state correlating timing/amount/IP).

---

## VERDICT: SOUND to ship, conditional on two fixes

The architecture is sound for the stated solo-operator, public-chain-analysis threat model. Two items block the *positioning* claim and must be resolved before ship:

### BLOCKING (2 items)

**B1. Scope the "consolidate without re-linking payers" claim.** As built, the linkage analyzer (`analyze_plan_linkage` in `planner.rs`, `detect_stealth_sweep_linkage` in `deposits.rs`) detects and blocks **single-hop common-recipient linkage** — the dominant EOA-graph clustering signal. It does NOT detect gas-funding linkage, amount/temporal correlation, downstream re-merging of per-party destinations, or multi-hop flows. The claim is **CONDITIONALLY DEFENSIBLE**: it holds when (a) `block_cross_party_linkage` policy is enabled (fail-closed), (b) the operator uses distinct per-party destinations, and (c) the operator maintains downstream separation of those destinations. The product docs, UI copy, and operator onboarding must scope the claim to "single-hop common-recipient linkage under policy-enforced mode" — NOT an unconditional guarantee. This is a documentation/positioning fix, not a code change.

**B2. Decide on `block_cross_party_linkage` default.** The policy defaults to `false` (`body.block_cross_party_linkage.unwrap_or(false)` in `update_treasury_policy`). For a "privacy-first" console, shipping with linkage as warn-only contradicts the thesis. Either (a) flip the default to `true`, or (b) make the operator explicitly choose during onboarding with a clear warning that warn-only mode does not prevent linkage. This is a product decision, not an architecture blocker — but it blocks the positioning claim until resolved.

### NON-BLOCKING follow-ups (do not gate ship)

- **N1. Loopback TCP surface.** The daemon binds `127.0.0.1:<ephemeral>`; any local process can reach the API. The bearer token (32-byte OsRng, constant-time compare, webview-only) and unlock throttle gate it. The in-process shell reads lock state via `Arc<AppState>`, not HTTP. Under the stated threat model (no local-malware actor specified), this is acceptable. **Local verification recommended**: confirm the unlock throttle (`check_unlock_throttle` / `record_unlock_failure`) enforces a rate limit + lockout and persists across restarts. UDS transport is a future hardening item.

- **N2. RPC provider privacy.** Balance/scan queries go through configured providers; one endpoint sees the full address set + operator IP. Under the stated threat model, the provider is not listed as a threat actor, so this is acceptable. Provider partitioning / Tor is a future enhancement.

- **N3. Gas-funding linkage.** Not modeled. If the operator funds gas for multiple receive addresses from one source, on-chain analysis can link them. This is out of scope for the common-recipient heuristic but should be documented as an operator-discipline requirement.

- **N4. Multi-chain receiving.** `chain_id` is hardcoded to 1 (mainnet) in `build_receiving_overview` and `hd_receiving_item`. Allocations carry no `chain_id`. This is a product-scope limitation, not a correctness bug.

- **N5. Maintenance auto-enqueue silent drop.** When `enqueue_deposit_sweep_job` returns `policy_violation("cross_party_linkage")` during auto-enqueue, the error is silently dropped (no structured reason persisted; operator sees the dashboard linkage badge). The deposit is simply not swept — no correctness gap, but a UX gap. Add a structured reason field in a follow-up.

- **N6. Downstream/multi-hop linkage.** The analyzer covers one hop on the destination axis only. Multi-hop analysis is out of scope for the stated threat model but should be noted as a known limitation.

- **N7. Per-file line caps.** `check-architecture.sh` enforces per-file line caps + modularization. This is a tooling convention; it does not affect correctness. The queue-domain checks enforce structural separation but do NOT enforce route↔contract↔client parity (acknowledged gap, non-blocking).

---

## "Consolidate without re-linking payers" — DEFENSIBILITY ASSESSMENT

**CONDITIONALLY DEFENSIBLE.** The claim holds under these conditions:

| Condition | Evidence | Status |
|---|---|---|
| `block_cross_party_linkage` enabled | `apply_linkage_blockers` converts warnings to hard blockers | ✅ Implemented, defaults OFF (B2) |
| HD plan: per-party routing | `routing_strategy="per_party"` + `party_destinations` | ✅ Implemented |
| Stealth: per-party destinations | `resolve_stealth_sweep_destination` resolves deposit→party→wallet chain | ✅ Implemented |
| Common-recipient detection (HD) | `analyze_plan_linkage` groups by destination, flags ≥2 distinct identities | ✅ Implemented, tested |
| Common-recipient detection (stealth) | `detect_stealth_sweep_linkage` mirrors HD logic over other deposits | ✅ Implemented, tested |
| Fail-closed at generation | `generate_consolidation_plan` applies linkage blockers when policy on | ✅ Verified |
| Fail-closed at approval | `approve_consolidation_plan` re-runs linkage analysis with current state | ✅ Verified |
| Fail-closed at sweep enqueue | `enqueue_deposit_sweep_job` returns `policy_violation` when blocked | ✅ Verified |
| Unattributed addresses treated as distinct identities | Each untagged address = its own identity (never merged) | ✅ Implemented, tested |
| Same-party multi-source to one destination allowed | Identity keyed by `counterparty_id`, not source address | ✅ Implemented, tested |

**What it does NOT cover** (must be documented as limitations):
- Gas-funding linkage (operator must fund gas from per-party sources)
- Amount/temporal correlation (explicitly excluded by threat model)
- Downstream re-merging of per-party destinations (operator discipline)
- Multi-hop flows through intermediaries

---

## 1. Verified workspace evidence

**Desktop shell (`crates/sigillum-desktop/src/main.rs`).** Tauri v2 binary. On launch: bind `127.0.0.1:0` → grab free ephemeral port → drop listener → start daemon in-process on that port via `sigillum_daemon::run_with_handle(addr, base_dir, opts, on_ready)` on a dedicated thread with its own tokio runtime → poll TCP readiness (10s cap) → open `WebviewWindow` at `http://127.0.0.1:<port>/`. `on_ready` hands `Arc<AppState>` + runtime `Handle` (via mpsc) stored as Tauri managed state. Tray shows lock state (2s poll of `AppState::is_unlocked()`); "Lock now" spawns `AppState::lock_now()`; window close → hide + lock; quit → lock (3s timeout) + exit. Single-instance plugin focuses existing window. **TOCTOU note**: between `pick_loopback_port()` dropping its listener and the daemon binding, another local process could steal the port — the daemon's bind would fail loudly, so this is a startup-failure risk, not a silent interception. Daemon enforces one instance per data dir via `DaemonLock` (flock on Unix, create_new on Windows).

**Daemon transport.** `run_with_options`/`run_with_handle` bind a `tokio::net::TcpListener` on the supplied `127.0.0.1` SocketAddr (loopback only; no UDS). UI served same-origin under strict CSP (`default-src 'self'; script-src 'nonce-…'; connect-src 'self'; frame-ancestors 'none'`). Auth = bearer session token minted on unlock (32 bytes OsRng), compared constant-time (`subtle::ConstantTimeEq`); token lives only in webview `sessionStorage`. No token-injection bootstrap; in-process shell reads lock state/locks via held `Arc<AppState>`, not HTTP. Unlock has throttle (`check_unlock_throttle` / `record_unlock_failure` — **throttle implementation is local verification item N1**).

**Private receiving.** First-class `Counterparty {id, name, note, sweep_destination_address?}` in `WalletInventoryState.parties`. HD receive allocations (`TreasuryReceiveAllocation`, BIP-44 `m/44'/60'/a'/0/i`) carry `counterparty_id?`. `GET /api/receiving/overview` (`build_receiving_overview`, `service/inventory/treasury.rs`) merges active HD allocations + ERC-5564 stealth deposits, grouped by payer (unattributed → "Unassigned"); HD balance shown only if exact lowercased address exists in `WalletInventoryState.addresses` (`balance_known=false` otherwise — nothing fetched); stealth balance from persisted deposit. `POST /api/receiving/refresh-balances` does bounded (`receiving_refresh_address_cap`, default 200) native-balance fetch for active allocations, reusing inventory scan's provider resolution + `upsert_address`, plus stealth deposit refresh. **chain_id hardcoded to 1** in overview (N4). `POST /api/receiving/deposits/tag` binds stealth deposit to payer.

**Linkage-aware consolidation (HD planner).** `build_plan_steps` (`planner.rs`) builds a step per holding. Destination: global `destination_address` → per-seed-profile hot/treasury/default (1.0-ETH threshold) → with `routing_strategy="per_party"`, each step routes to payer's mapped destination (party without mapping → `missing_party_destination` blocker; unattributed → existing default). `analyze_plan_linkage(state, steps)` joins step source `address → receive_allocations.counterparty_id → party`, groups sweep steps by destination, flags any destination with ≥2 distinct identities (tagged party = one identity; each untagged address = its own distinct identity, never merged). Non-blocking `linkage_warnings` per step + plan-level `linkage_findings`. `generate_consolidation_plan` (`service/inventory/consolidation.rs`) ordering: build → `apply_policy_blockers_to_step` → `plan_policy_violations` → `analyze_plan_linkage` → (if `TreasuryPolicy.block_cross_party_linkage`) `apply_linkage_blockers` (warned → hard `cross_party_linkage` blocker) → `summarize_plan_steps` → status. `approve_consolidation_plan` re-runs linkage pass (reconstructs minimal state with current `receive_allocations` + `parties`) before approving; note: previously-approved steps that gain a linkage blocker end up `approved=true` + `status="blocked"`, which export correctly handles (blocked steps skipped). Export (`export.rs`): `call_manifest` bundles per source address (isolated); `safe_tx_builder` sets `source_address=None` (merges) but constrained to steps whose source IS the Safe.

**Linkage-aware consolidation (stealth sweeps).** Separate queue path. `enqueue_deposit_sweep_job` (`service/deposits.rs`) resolves destination = `deposit.sweep_destination_address → tagged party's sweep_destination_address → wallet.default_destination_address`, then `detect_stealth_sweep_linkage` builds same identity model over other stealth deposits; under `block_cross_party_linkage` a shared-destination sweep → `policy_violation("cross_party_linkage")`. The maintenance auto-enqueue path catches the per-deposit error and skips silently (N5). `authorize_transaction_policy` (allowlist + value caps) runs as separate gate.

**Cross-cutting.** `scripts/check-architecture.sh` enforces per-file line caps + required-file existence + queue-domain modularization (does NOT enforce route↔contract↔client parity — N7). Strict eth-address validation (`check_eth_address` in `validation.rs`) mirrors daemon's `normalize_address` (0x optional, 40 hex, case-insensitive, no EIP-55). CLI + async-client parity for routes in submodules. State = several JSON files (`wallet_inventory.json`, `deposits.json`, `queue.json`, `profiles.json`) written atomically with `.bak` sidecars + corrupt-file quarantine; daemon ops serialize behind `operation_guard()`.

## 2. Inference (drawn from evidence + threat model)

- In-process loopback design removes bundled-sidecar/orphan/token-over-port problems but retains a listening TCP socket reachable by local processes. Under the stated threat model (no local-malware actor), acceptable. Bearer token + unlock throttle are the gates; UDS would shrink the surface further (N1).
- Linkage model captures the **common-recipient heuristic** — the dominant EOA-graph clustering signal for the stated threat model. Single-hop, destination-axis-only coverage is **sufficient** for "public on-chain analysis + payers" because that is the primary on-chain linkage vector for EOA-to-EOA consolidation. Gas-funding and multi-hop are real but secondary and require a more sophisticated adversary than the stated model assumes.
- "Fail-closed" completeness depends on `analyze_plan_linkage`'s identity coverage. Current coverage (HD allocations with `counterparty_id` → party identity; everything else → distinct unattributed identity) is conservative and correct: it can produce false-positive warnings (two untagged addresses to one destination) but never false negatives (two tagged parties to one destination are always caught).
- Per-party isolated destinations push long-term privacy onto operator downstream discipline. This is inherent to the design and must be documented.

## 3. Future work / experiments (non-blocking)

- UDS transport or per-launch handshake to drop loopback TCP surface (N1).
- Provider partitioning / Tor-or-proxy for RPC (N2).
- Multi-chain receiving: `chain_id` on allocations; overview/refresh per chain (N4).
- Gas-funding linkage detection (N3).
- Structured "blocked by linkage" reason on maintenance auto-enqueue path (N5).
- Downstream/multi-hop linkage analysis (N6).

## 4. Local verification items (non-blocking, recommended before ship)

- **V1.** Confirm unlock throttle implementation: rate limit, lockout duration, persistence across daemon restarts. Verify in `state.rs` or wherever `check_unlock_throttle` / `record_unlock_failure` live.
- **V2.** Confirm the TOCTOU window in `pick_loopback_port` → `start_daemon` produces a loud startup failure (not silent interception) when another process steals the port between drop and bind. The daemon's `TcpListener::bind(addr).await?` should error out, and `wait_for_daemon` should report the error from the `daemon_errors` channel.
- **V3.** Confirm `block_cross_party_linkage` default behavior in the setup wizard / first-run flow — does the operator ever see this setting, or is it silently `false` until manually configured?

## Remaining Open Questions

All material questions are resolved by the evidence + stated threat model. The three local verification items (V1–V3) are recommended checks, not blockers — they confirm implementation details of an architecture that is already sound on its face. The only decisions required from the product owner are B1 (scope the claim) and B2 (choose the default policy), both of which are documentation/config decisions, not code changes.
