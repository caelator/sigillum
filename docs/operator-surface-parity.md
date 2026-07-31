# Operator-Surface Parity Matrix

Source of truth: `crates/sigillum-daemon/src/routes/mod.rs` — the router is
assembled in `api_router()` and the API surface in `api_routes()`. All route
registrations live in that one file (submodules under `src/routes/` contain
handlers only, no registrations).

**Counts (cross-checked against the router):** 142 route registrations
(`grep -c '\.route(' crates/sigillum-daemon/src/routes/mod.rs` -> 142), which
is 143 method endpoints because `/api/treasury/parties` registers both GET
and POST. The 35 family rows below cover 142/142 registrations; each route
appears in exactly one row.

**Maintenance rule (release-1.0 plan §0.1.5 / D2):** every W-task that adds
or changes routes MUST update this file in the same PR chain. A route family
may only be UI-less and CLI-less when the Decision column records an explicit
rationale.

**Query parameters (plan task 1.5):** the list routes in rows 23
(`inventory/wallets`), 24 (`discovery/jobs`), 25 (`risk/findings`), 26
(`plans/consolidation`), 29 (`deposits/eth-stealth`), and 30 (`queue/jobs`)
accept additive optional filter/sort/`limit`/`offset` query parameters
(enumerated in `docs/stability.md`). A parameterless request keeps the
legacy response exactly, so no UI/CLI surface above changes: the client
gains `list_*_with_options` variants, and CLI flags for these parameters
are deliberately deferred until a CLI consumer needs them.

Column legend:

- **Routes** — registered paths; all rows except the UI root are under `/api/`.
- **UI** — the active destination controller under
  `crates/sigillum-daemon/ui/src/destinations/` when it owns the operator
  surface. A `views/*` name identifies residual setup/session plumbing or a
  legacy surface outside a controller takeover. `app.ts` is the composition
  shell, not the owner of every panel.
- **CLI** — the `sigillum api` subcommand dispatched from
  `crates/sigillum-cli/src/daemon_api.rs` (and its `daemon_api/` submodules),
  a local `sigillum` command where noted, or `no`.
- **Decision** — why any missing surface is deliberate.

The D1 CLI additions (`sigillum api transit`, read-only `sigillum api evm`,
`sigillum api wallets` export/derive/check/generate, and
`sigillum api compartment list`) have landed; the rows below name the actual
commands.

| # | Family | Routes (`/api/...` unless noted) | UI | CLI | Decision / rationale |
|---|--------|----------------------------------|----|-----|----------------------|
| 1 | Embedded UI root | `/` (GET, no `/api` prefix) | is the UI (`serve_ui`) | no | Serves the embedded operator console itself; not an API surface. |
| 2 | Health | `health` | no | internal: daemon-autostart readiness poll (`ensure_daemon_ready`), `sigillum doctor` | Unauthenticated liveness probe for machine clients; no interactive surface needed. |
| 3 | Session lifecycle | `status`, `unlock`, `lock`, `session/revoke` | `Vault.ts` (lock/revoke) + shell/session unlock and status plumbing | `sigillum api status\|unlock\|unlock-fido2\|lock\|revoke-session` | Fully covered. Status is SSE-backed with passive polling fallback. |
| 4 | Capability sessions | `auth/capability` | no | no | Decision (D2): internal session plumbing for scoped machine tokens; exercised by `crates/sigillum-daemon/tests/integration.rs`; deliberately no interactive surface. |
| 5 | Biometric unlock | `biometric/challenge`, `biometric/unlock`, `biometric/enroll` | no | `sigillum biometric ...` (`crates/sigillum-cli/src/biometric.rs`) | CLI/native-only: the challenge/sign ceremony binds to the local OS keychain and cannot run from a browser page. |
| 6 | Diagnostics and self-check | `diagnostics`, `selfcheck/run` | `Vault.ts` (full diagnostics/self-check) + `Overview.ts` (attention action) | `sigillum api diagnostics`, `sigillum api selfcheck` | Fully covered. |
| 7 | Maintenance | `maintenance/run` | `Move.ts` (foreground/background maintenance and operation progress) | `sigillum api maintenance run` (`--run-async`) | Fully covered. |
| 8 | Audit | `audit`, `audit/verify`, `audit/run` | `Overview.ts` (recent activity) + `Vault.ts` (security/snapshot audit views) | `sigillum audit` (query + verify); `audit/run` is written by `sigillum run` | `audit/run` is a programmatic recorder used by `sigillum run`; query/verify are covered. |
| 9 | Setup reset | `setup/reset` | `Vault.ts` typed-confirmation reset flow | no | Decision (D2): destructive; UI typed-confirmation only — never scriptable, so a reset cannot land in shell history. |
| 10 | Backup snapshot | `backup/export`, `backup/restore` | `Vault.ts` | local equivalent: `sigillum backup export\|restore` (offline, direct data-dir; does not call these routes) | CLI stays offline-local by design so restore works without a running daemon; browser file handling remains part of real-browser smoke. |
| 11 | Compartments (visible ops) | `compartment/list`, `compartment/init`, `compartment/switch` | `Vault.ts` (list/switch) + `setup.ts` (init wizard) | `sigillum api switch`; local `sigillum compartment list\|switch`; `sigillum api compartment list` | Decision (D2): `init` is destructive-adjacent and runs inside the guided UI setup wizard. |
| 12 | Compartments (add/remove) | `compartment/add`, `compartment/remove` | partial: `Vault.ts` supports add; remove has no UI | no | Decision (D2): remove is destructive and remains API-only pending a typed-confirmation UI. It is deliberately not CLI-scriptable. |
| 13 | API-key CRUD (daemon-side) | `api-keys`, `api-keys/get`, `api-keys/set`, `api-keys/delete` | `Vault.ts` | no remote CRUD; local `sigillum set-api\|get-api\|delete-api` covers the standalone FileVault | Decision (D2): UI covers daemon-side CRUD; local CLI covers the standalone FileVault; no `sigillum api` CRUD so secret values stay out of shell history. |
| 14 | Secret CRUD (daemon-side) | `secrets`, `secrets/get`, `secrets/set`, `secrets/delete`, `secrets/push` | `Vault.ts` | no remote CRUD; local `sigillum set\|get\|delete\|list\|push` covers the standalone FileVault | Decision (D2): same as api-keys — UI covers daemon-side CRUD; local CLI covers the FileVault. |
| 15 | Secret batch resolution | `secrets/resolve-batch` | no | consumed by `sigillum run` (env injection) | Programmatic-only endpoint; its operator surface is `sigillum run`, which resolves secrets into a child-process environment. |
| 16 | Generate-and-store | `generate/store` | no | `sigillum generate` | Covered by CLI; generated material is stored server-side. |
| 17 | Transit crypto | `transit/encrypt`, `transit/decrypt`, `transit/hmac` | no | `sigillum api transit encrypt\|decrypt\|hmac` | Machine-to-machine crypto operations; CLI is the right surface, no UI need. |
| 18 | EVM read-only | `evm/nonce`, `evm/balance`, `evm/erc20-balance`, `evm/fees/estimate` | no | `sigillum api evm nonce\|balance\|erc20-balance\|fees estimate` | Read-only chain queries are scriptable; D1 adds them and explicitly excludes `broadcast`. |
| 19 | EVM broadcast | `evm/broadcast` | no | no — permanent | Decision (D2): no CLI — hazard: signed payloads in shell history and no plan review. Programmatic API/SDK callers only; interactive sends go through plans + queue. |
| 20 | Wallet profiles | `profiles/evm` (+`/upsert`,`/delete`), `profiles/eth-stealth` (+`/upsert`,`/delete`), `profiles/eth-xpub` (+`/upsert`,`/delete`), `profiles/eth-seed` (+`/upsert`,`/create`,`/delete`) — 13 routes | `portfolio.ts` and `Receiving.ts`, with residual setup/wallet-manager flows | `sigillum api profiles <evm\|stealth\|eth-xpub\|eth-seed> <list\|upsert\|create\|delete>` | Fully covered. Plan task 3.2 (additive): every delete accepts an optional `prune_inventory` flag (CLI `--prune-inventory`; the console's shared confirm dialog offers the cascade as an explicit checkbox) that forgets the profile's scanned history, receive allocations, and counterparty bindings in the same operation — absent/false keeps the legacy orphaning behavior. |
| 21 | Wallet key ops (read/derive, no spend) | `wallets/eth-xpub/export`, `wallets/eth-xpub/derive`, `wallets/eth-stealth/export`, `wallets/eth-stealth/generate`, `wallets/eth-stealth/check` | `Receiving.ts` owns stealth export/payer instructions; residual `wallets.ts` owns xpub export/derive; generate/check remain API/CLI | `sigillum api wallets xpub-export\|xpub-derive\|stealth-export\|stealth-generate\|stealth-check` | No sign/send in this family, so it is safe to script; D1 adds the CLI. Xpub exports carry a whole-tree exposure warning and first-copy acknowledgement; `eth-xpub/derive` is the documented unauthenticated local derivation oracle. |
| 22 | Wallet sign/send | `wallets/eth-stealth/sign`, `.../sign-transfer`, `.../sign-erc20-transfer`, `.../send-transfer`, `.../send-erc20-transfer`, `.../send-with-profile`, `.../send-erc20-with-profile` | no | no — permanent | Decision (D2): same hazard as `evm/broadcast` — shell history plus no plan review. Interactive spending goes through consolidation plans + queue (review/approve); programmatic callers use the API/SDK. |
| 23 | Inventory registry | `inventory/wallets`, `chains` (+`/upsert`,`/delete`), `inventory/chains` (+`/upsert`,`/delete` legacy alias), `inventory/scan/evm`, `inventory/addresses/delete`, `inventory/watch-addresses` (+`/upsert`,`/delete`), `inventory/token-registry` (+`/import`,`/delete`) | `portfolio.ts`; residual setup/wallet-manager flows remain | `sigillum api chains <list\|upsert\|delete>`; `sigillum api inventory <list\|chains\|watch\|token-registry\|scan-evm\|prune-addresses>` (`scan-evm --all-configured-chains --probe-token-registry --run-async --partition-providers`) | Fully covered. Plan task 3.2 (additive): `inventory/addresses/delete` forgets scanned-address rows (plus their holdings and per-address block cursors) by AND-combined selectors (`prune-addresses --address\|--wallet-profile\|--provider-profile\|--chain-id\|--account-index`, at least one required); pruning removes history, not derivation — a re-scan re-observes still-derived indices as fresh rows. |
| 24 | Discovery jobs | `discovery/jobs`, `discovery/jobs/cancel`, `discovery/jobs/resume` | `portfolio.ts` (list, progress, cancel, and resume) | `sigillum api discovery jobs <list\|cancel\|resume>` | Covered. Cancel cooperatively stops the live scan (409 on terminal jobs); resume starts a new background operation from persisted checkpoints. |
| 25 | Risk | `risk/findings`, `risk/catalog` (+`/upsert`,`/delete`) | `portfolio.ts` | `sigillum api risk <list\|catalog\|catalog-upsert\|catalog-delete>` | Fully covered. |
| 26 | Consolidation plans | `plans/consolidation` (+`/generate`,`/approve`,`/simulate`,`/export`) | `Move.ts` | `sigillum api plans <list\|generate\|approve\|simulate\|export>` (`generate --chain-id` optional) | Fully covered — this is the reviewed path for spending. Plan task 3.5 (additive): plans carry advisory `risk_findings` (`common_gas_funder` when one gas sponsor funds distinct parties' receive addresses); `block_cross_party_linkage` now defaults ON when a policy update omits it. |
| 26a | Plan-step enqueue (W7.2) | `plans/enqueue-step`, `plans/enqueue-plan` | `Move.ts` (per-step Execute and typed-confirmed Execute All Eligible) | `sigillum api plans enqueue-step --plan-id --step-id --confirm`, `sigillum api plans enqueue-plan --plan-id --confirmation "<PHRASE>"` (run without `--confirmation` to have the daemon print the exact phrase) | Fully covered. Every check is re-validated server-side at enqueue time (W7.1 gates, treasury allowlist/caps, linkage, simulation pass + freshness, W5 claim gate, W6.1 gas-topup opt-in, idempotency, W6.4 dependency order). Enqueued `plan_step_execution` jobs stay hard-blocked at drain time until W7.3 enables execution. |
| 27 | Treasury | `treasury/overview`, `treasury/policy` (+`/update`), `treasury/receive-addresses` (+`/allocate`,`/rotate`,`/purge`), `treasury/parties` (GET+POST) (+`/update`,`/delete`) — 10 registrations | `Receiving.ts` owns parties/allocations; `Move.ts` owns policy and party lookup | `sigillum api treasury <overview\|policy\|policy-update\|receive-list\|receive-allocate\|receive-rotate\|receive-purge\|parties ...>` | Fully covered. Counterparty update treats omitted `sweep_destination_address` as retain and explicit blank as clear. One-time allocation lifecycle and purge rules remain fail-closed. |
| 28 | Receiving | `receiving/overview`, `receiving/refresh-balances`, `receiving/deposits/tag` | `Receiving.ts` | `sigillum api receiving <overview\|refresh-balances\|tag-deposit>` | Fully covered. Items expose optional per-item freshness; refresh and overview matching use wallet family/profile, chain, and case-insensitive address identity. |
| 29 | Stealth deposits | `deposits/eth-stealth` (+`/create-native`,`/create-erc20`,`/scan-announcements`,`/delete`,`/refresh`,`/enqueue-sweep`) — 7 routes | `Receiving.ts` | `sigillum api deposits <list\|create-native\|create-erc20\|scan-announcements\|refresh\|enqueue-sweep\|delete>` | Fully covered. Payer-gas metadata, sponsor state, and sweep prerequisites are surfaced. Sponsor top-ups are emitted only by the deposit sweep flow, not a public enqueue route. |
| 30 | Queue (observe/process) | `queue/jobs`, `queue/process`, `queue/pause`, `queue/resume` | `Move.ts` | `sigillum api queue <list\|process\|pause\|resume>` (`process --run-async`; JSON includes per-job `state`/`last_error` and process counts) | Fully covered. Pause is latched before the drain mutex and rechecked immediately pre-broadcast; resume clears it only after durable policy persistence. Scheduler work uses the same paths and appears under operations. |
| 31 | Manual queue enqueue | `queue/enqueue/eth-stealth-transfer`, `.../eth-stealth-erc20-transfer`, `.../eth-stealth-native-sweep`, `.../eth-stealth-erc20-sweep` | no (direct) | no | Decision (D2): maintenance auto-enqueues, and the UI covers the manual path via the deposit sweep action (`deposits/eth-stealth/enqueue-sweep`). Direct enqueue endpoints remain programmatic/API-only. |
| 32 | FIDO2 admin | `fido2/status`, `fido2/detect`, `fido2/pin/set`, `fido2/list`, `fido2/setup`, `fido2/register`, `fido2/unlock`, `fido2/remove` | `Vault.ts` + `setup.ts` unlock/setup ceremony | `sigillum api unlock-fido2` (unlock only); local `sigillum fido2 <register\|list\|remove\|status\|unlock>` manages the same store offline | Decision (D2): admin requires a physical touch ceremony at the machine. Modal cancellation is distinct from explicit blank input and sends no remove request. |
| 33 | NFT metadata (opt-in fetch) | `inventory/nft-metadata/opt-ins` (+`/upsert`,`/delete`), `inventory/nft-metadata/settings`, `inventory/nft-metadata/fetch` — 5 routes | `portfolio.ts` | no | Deliberate UI+API surface at 1.0: metadata fetching is an interactive, privacy-sensitive review action, so no CLI is provided; scripting goes through the API/SDK. |
| 34 | Background operations | `operations`, `operations/{id}`, `operations/{id}/cancel` | `Move.ts` and `portfolio.ts` expose relevant starts/progress/cancel; all five controllers consume live operation state where relevant | partial: `sigillum api inventory scan-evm --run-async`, `sigillum api queue process --run-async`, and `sigillum api maintenance run --run-async` start operations; no list/get/cancel CLI | UI workflows are covered; direct generic list/get/cancel stays API-only until a CLI consumer exists. |
| 35 | Events (SSE) | `events` | `core/events.ts` feeds store slices consumed by Overview, Move, Receiving, Portfolio, and Vault; residual legacy cards retain their refresh loop, but no destination controller adds a new poller | no — but the `sigillum-client` SDK exposes `subscribe_events` | Decision (plan task 1.3 / D-D): versioned `snapshot`/`operation`/`queue`/`status` events with authoritative live snapshots, bounded terminal-history enrichment, generation/revision guards, retry without SSE degradation, and passive polling fallback. Connecting never defers idle auto-lock. |

## Verification

- Route registrations in `crates/sigillum-daemon/src/routes/mod.rs`: **142**
  (`grep -c '\.route('`). Method endpoints: **143** (`/api/treasury/parties`
  is GET+POST on one registration).
- Sum of routes across the 35 rows above: **142** — every registration
  appears in exactly one row.
- No row is UI=no and CLI=no without an explicit decision (rows 4, 12, 19,
  22, 31 carry D2 decisions; row 12 is explicitly partial; rows 2, 15 are
  machine-plumbing decisions; row 33 records the deliberate UI+API-only NFT
  metadata decision; row 34 records
  the partial-CLI operations decision; row 35 records complete destination
  adoption and the residual legacy refresh boundary for the SSE channel).
