# Sigillum — Architecture Convergence Candidate

Branch `feat/private-receiving-desktop` @ `70a087b`. Rust workspace; local-first,
privacy-focused, **solo-operator** Ethereum *receiving + treasury* console.

Product thesis: receive funds from many parties using use-once addresses (fresh
HD per payer, or ERC-5564 stealth) so payers can't be linked together, manage it
all from one desktop interface backed by one or more BIP-39 seeds, and
**consolidate without re-linking the payers**. Stated threat model: public
on-chain analysis + the payers themselves (NOT a nation-state correlating
timing/amount/IP).

We want an architect's convergence verdict: **is this architecture sound for that
positioning, and what must change?** Drive to CONVERGED (YES / NO / BLOCKED).

---

## 1. Verified workspace evidence (confirmed by reading the code this session)

**Desktop shell (`crates/sigillum-desktop/src/main.rs`).** Tauri v2 binary. On
launch it binds `127.0.0.1:0` to grab a free ephemeral port, drops the listener,
then starts the daemon IN-PROCESS on that port via
`sigillum_daemon::run_with_handle(addr, base_dir, opts, on_ready)` on a dedicated
thread with its own tokio runtime; polls TCP readiness (10s cap); opens a
`WebviewWindow` at `http://127.0.0.1:<port>/`. `on_ready` hands the launcher
`Arc<AppState>` + the runtime `Handle` (via mpsc) stored as Tauri managed state.
System tray shows live lock state (2s poll of `AppState::is_unlocked()`), "Lock
now" spawns `AppState::lock_now()`; window close intercepts → hides + spawns
`lock_now`; quit spawns `lock_now` (3s timeout) then exits. Single-instance
plugin focuses the existing window. The daemon enforces one instance per data
dir via `DaemonLock`.

**Daemon transport.** `run_with_options`/`run_with_handle` bind a
`tokio::net::TcpListener` on the supplied `127.0.0.1` SocketAddr (loopback only;
no UDS support). UI served same-origin under a strict CSP
(`default-src 'self'; script-src 'nonce-…'; connect-src 'self';
frame-ancestors 'none'`). Auth = bearer session token minted on unlock (32 bytes
OsRng), compared constant-time (`subtle::ConstantTimeEq`); token lives only in
the webview's `sessionStorage`. There is NO token-injection bootstrap; the
in-process shell reads lock state / locks via the held `Arc<AppState>`, not over
HTTP. Unlock has a throttle (`check_unlock_throttle` / `record_unlock_failure`).

**Private receiving.** First-class `Counterparty {id,name,note,
sweep_destination_address?}` persisted in `WalletInventoryState.parties`. HD
receive allocations (`TreasuryReceiveAllocation`, BIP-44 `m/44'/60'/a'/0/i`)
carry `counterparty_id?`. `GET /api/receiving/overview`
(`build_receiving_overview`, `service/inventory/treasury.rs`) merges active HD
allocations + ERC-5564 stealth deposits (`deposits.json`) grouped by payer
(unattributed/unknown → "Unassigned"); HD balance shown only if the exact
lowercased address exists in `WalletInventoryState.addresses`
(`balance_known=false` otherwise — nothing fetched); stealth balance from the
persisted deposit. `POST /api/receiving/refresh-balances` does a bounded
(`receiving_refresh_address_cap`, default 200) native-balance fetch for active
allocation addresses, reusing the inventory scan's provider resolution +
`upsert_address`, plus the existing stealth deposit refresh. **chain_id is
hardcoded to 1 (mainnet)** in the overview because allocations carry no chain id.
`POST /api/receiving/deposits/tag` binds a stealth deposit to a payer.

**Linkage-aware consolidation (HD planner).** `build_plan_steps` (`planner.rs`)
builds a step per holding; destination today is a global `destination_address`,
else per-seed-profile hot/treasury/default (1.0-ETH threshold), else — with
`routing_strategy="per_party"` — each step routes to its payer's mapped
destination (party without a mapping → `missing_party_destination` blocker;
unattributed → existing default routing). `analyze_plan_linkage(state, steps)`
joins step source `address → receive_allocations.counterparty_id → party`,
groups sweep steps by destination, and flags any destination with ≥2 distinct
identities (a tagged party is one identity; each untagged address is its own
distinct identity, never merged) — non-blocking `linkage_warnings` per step +
plan-level `linkage_findings`. `generate_consolidation_plan`
(`service/inventory/consolidation.rs`) ordering: build → `apply_policy_blockers_to_step`
→ `plan_policy_violations` → `analyze_plan_linkage` → (if
`TreasuryPolicy.block_cross_party_linkage`) `apply_linkage_blockers` (turns each
warned step into a hard `cross_party_linkage` blocker, status/risk=blocked) →
`summarize_plan_steps` → status. `approve_consolidation_plan` re-runs the
linkage block pass before approving (approve only flips `review_required`).
Export (`export.rs`): `call_manifest` bundles per source address (isolated);
`safe_tx_builder` sets `source_address=None` (merges) but is constrained to steps
whose source IS the Safe.

**Linkage-aware consolidation (stealth sweeps).** Separate queue path.
`enqueue_deposit_sweep_job` (`service/deposits.rs`) resolves destination =
`deposit.sweep_destination_address → tagged party's sweep_destination_address →
wallet.default_destination_address`, then `detect_stealth_sweep_linkage` builds
the same D1 identity model over the OTHER stealth deposits; under
`block_cross_party_linkage` a shared-destination sweep → `policy_violation`
("cross_party_linkage"); otherwise a non-blocking warning on the response /
`ReceivingItem.linkage_warning`. The maintenance auto-enqueue path catches the
per-deposit error and skips that deposit (no structured reason recorded; the
operator sees the dashboard linkage badge). `authorize_transaction_policy`
(allowlist + value caps) still runs as a separate gate.

**Cross-cutting.** `scripts/check-architecture.sh` enforces per-file line caps +
required-file existence + queue-domain modularization (it does NOT enforce
route↔contract↔client parity). Strict eth-address validation (`check_eth_address`
in `validation.rs`) mirrors the daemon's `normalize_address` (0x optional, 40
hex, case-insensitive, no EIP-55 enforcement). CLI + async-client parity for the
new routes lives in submodules (`client/receiving.rs`, `cli/daemon_api/receiving.rs`).
State is several JSON files (`wallet_inventory.json`, `deposits.json`,
`queue.json`, `profiles.json`) written atomically with `.bak` sidecars +
corrupt-file quarantine; daemon ops serialize behind `operation_guard()`.

## 2. Inference (drawn from the evidence)

- The in-process loopback design removes the bundled-sidecar/orphan/token-over-port
  problems but does NOT remove the listening TCP socket: any local process/loopback
  client can reach the API; only the bearer token (in the webview only) and the
  unlock throttle gate it. A UDS or an authenticated handshake would shrink that
  surface.
- The linkage model captures the **common-recipient heuristic** (the dominant
  EOA-graph clustering signal) but only one hop and only the destination axis — it
  does not model gas-funding linkage, amount/temporal correlation, or downstream
  re-merging of per-party destinations.
- "Fail-closed" is only as complete as `analyze_plan_linkage`'s identity coverage;
  a gap there silently lets a linking step through even with the policy on.
- Per-party isolated destinations push privacy onto the operator's downstream
  discipline (they must keep those destinations separate forever).

## 3. Future work / experiments (not yet done)

- UDS transport (or per-launch handshake) to drop the loopback TCP surface.
- Provider partitioning / Tor-or-proxy for RPC so balance/scan queries don't hand
  one endpoint the full address set + IP correlation (currently warn-only).
- Multi-chain receiving (chain_id on allocations; overview/refresh per chain).
- Downstream/multi-hop linkage analysis; gas-funding linkage detection.
- Structured "blocked by linkage" reason on the maintenance auto-enqueue path.

## 4. Missing facts that block a decision

- Is the unlock throttle strong enough to make local brute-force against the
  loopback unlock endpoint a non-issue? (rate, lockout, persistence across restart)
- For the stated threat model, is "warn on RPC privacy cost" acceptable, or does
  the privacy positioning REQUIRE provider partitioning before shipping?
- Is single-hop common-recipient linkage analysis sufficient to claim "consolidate
  without re-linking payers," or is that claim overstated without downstream/gas
  coverage?
- Does enforcing arbitrary per-file line caps as a gate improve or distort the
  architecture (e.g., extractions done to fit a number rather than a domain)?

## Convergence question

Given the solo-operator, public-chain-analysis threat model: is this architecture
**sound to ship as a privacy-first receiving/treasury console**, what are the
**blocking** correctness/security/privacy risks (vs. non-blocking follow-ups),
and is the "consolidate without re-linking payers" claim **defensible** as built?
