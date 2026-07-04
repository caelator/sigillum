# Operator-Surface Parity Matrix

Source of truth: `crates/sigillum-daemon/src/routes/mod.rs` — the router is
assembled in `api_router()` and the API surface in `api_routes()`. All route
registrations live in that one file (submodules under `src/routes/` contain
handlers only, no registrations).

**Counts (cross-checked against the router):** 124 route registrations
(`grep -c '\.route(' crates/sigillum-daemon/src/routes/mod.rs` -> 124), which
is 125 method endpoints because `/api/treasury/parties` registers both GET
and POST. The 32 family rows below cover 124/124 registrations; each route
appears in exactly one row.

**Maintenance rule (release-1.0 plan §0.1.5 / D2):** every W-task that adds
or changes routes MUST update this file in the same PR chain. A route family
may only be UI-less and CLI-less when the Decision column records an explicit
rationale.

Column legend:

- **Routes** — registered paths; all rows except the UI root are under `/api/`.
- **UI** — the view module under `crates/sigillum-daemon/ui/src/views/` that
  calls the routes (`app.ts` = the UI shell, which owns settings, secrets,
  audit, backup, and reset panels), or `no`.
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
| 3 | Session lifecycle | `status`, `unlock`, `lock`, `session/revoke` | `session.ts` (+ `app.ts` status polling) | `sigillum api status\|unlock\|unlock-fido2\|lock\|revoke-session` | Fully covered. |
| 4 | Capability sessions | `auth/capability` | no | no | Decision (D2): internal session plumbing for scoped machine tokens; exercised by `crates/sigillum-daemon/tests/integration.rs`; deliberately no interactive surface. |
| 5 | Biometric unlock | `biometric/challenge`, `biometric/unlock`, `biometric/enroll` | no | `sigillum biometric ...` (`crates/sigillum-cli/src/biometric.rs`) | CLI/native-only: the challenge/sign ceremony binds to the local OS keychain and cannot run from a browser page. |
| 6 | Diagnostics and self-check | `diagnostics`, `selfcheck/run` | `app.ts` (diagnostics), `selfcheck.ts` | `sigillum api diagnostics`, `sigillum api selfcheck` | Fully covered. |
| 7 | Maintenance | `maintenance/run` | `operations.ts` | `sigillum api maintenance run` | Fully covered. |
| 8 | Audit | `audit`, `audit/verify`, `audit/run` | `app.ts` (recent events) | `sigillum audit` (query + verify); `audit/run` is written by `sigillum run` | `audit/run` is a programmatic recorder used by the `sigillum run` wrapper, not an interactive action; query/verify are covered on both surfaces. |
| 9 | Setup reset | `setup/reset` | `app.ts` typed-confirmation reset flow | no | Decision (D2): destructive; UI typed-confirmation only — never scriptable, so a reset cannot land in shell history. |
| 10 | Backup snapshot | `backup/export`, `backup/restore` | `app.ts` | local equivalent: `sigillum backup export\|restore` (offline, direct data-dir; does not call these routes) | CLI stays offline-local by design so restore works without a running daemon; the daemon routes are UI-covered. |
| 11 | Compartments (visible ops) | `compartment/list`, `compartment/init`, `compartment/switch` | `app.ts` (list/switch), `setup.ts` (init wizard) | `sigillum api switch`; local `sigillum compartment list\|switch`; `sigillum api compartment list` | Decision (D2): `init` is destructive-adjacent and runs inside the guided UI setup wizard. |
| 12 | Compartments (add/remove) | `compartment/add`, `compartment/remove` | no (today) | no | Decision (D2): destructive; the operator path is a UI typed-confirmation flow (same pattern as setup/reset), deliberately not CLI. API-only until that UI flow lands. |
| 13 | API-key CRUD (daemon-side) | `api-keys`, `api-keys/get`, `api-keys/set`, `api-keys/delete` | `app.ts` | no remote CRUD; local `sigillum set-api\|get-api\|delete-api` covers the standalone FileVault | Decision (D2): UI covers daemon-side CRUD; local CLI covers the standalone FileVault; no `sigillum api` CRUD so secret values stay out of shell history. |
| 14 | Secret CRUD (daemon-side) | `secrets`, `secrets/get`, `secrets/set`, `secrets/delete`, `secrets/push` | `app.ts` | no remote CRUD; local `sigillum set\|get\|delete\|list\|push` covers the standalone FileVault | Decision (D2): same as api-keys — UI covers daemon-side CRUD; local CLI covers the FileVault. |
| 15 | Secret batch resolution | `secrets/resolve-batch` | no | consumed by `sigillum run` (env injection) | Programmatic-only endpoint; its operator surface is `sigillum run`, which resolves secrets into a child-process environment. |
| 16 | Generate-and-store | `generate/store` | no | `sigillum generate` | Covered by CLI; generated material is stored server-side. |
| 17 | Transit crypto | `transit/encrypt`, `transit/decrypt`, `transit/hmac` | no | `sigillum api transit encrypt\|decrypt\|hmac` | Machine-to-machine crypto operations; CLI is the right surface, no UI need. |
| 18 | EVM read-only | `evm/nonce`, `evm/balance`, `evm/erc20-balance`, `evm/fees/estimate` | no | `sigillum api evm nonce\|balance\|erc20-balance\|fees estimate` | Read-only chain queries are scriptable; D1 adds them and explicitly excludes `broadcast`. |
| 19 | EVM broadcast | `evm/broadcast` | no | no — permanent | Decision (D2): no CLI — hazard: signed payloads in shell history and no plan review. Programmatic API/SDK callers only; interactive sends go through plans + queue. |
| 20 | Wallet profiles | `profiles/evm` (+`/upsert`,`/delete`), `profiles/eth-stealth` (+`/upsert`,`/delete`), `profiles/eth-xpub` (+`/upsert`,`/delete`), `profiles/eth-seed` (+`/upsert`,`/create`,`/delete`) — 13 routes | `wallets.ts`, `walletManager.ts`, `journey.ts` | `sigillum api profiles <evm\|stealth\|eth-xpub\|eth-seed> <list\|upsert\|create\|delete>` | Fully covered. |
| 21 | Wallet key ops (read/derive, no spend) | `wallets/eth-xpub/export`, `wallets/eth-xpub/derive`, `wallets/eth-stealth/export`, `wallets/eth-stealth/generate`, `wallets/eth-stealth/check` | `wallets.ts` (both exports + derive); generate/check not in UI yet | `sigillum api wallets xpub-export\|xpub-derive\|stealth-export\|stealth-generate\|stealth-check` | No sign/send in this family, so it is safe to script; D1 adds the CLI. |
| 22 | Wallet sign/send | `wallets/eth-stealth/sign`, `.../sign-transfer`, `.../sign-erc20-transfer`, `.../send-transfer`, `.../send-erc20-transfer`, `.../send-with-profile`, `.../send-erc20-with-profile` | no | no — permanent | Decision (D2): same hazard as `evm/broadcast` — shell history plus no plan review. Interactive spending goes through consolidation plans + queue (review/approve); programmatic callers use the API/SDK. |
| 23 | Inventory registry | `inventory/wallets`, `chains` (+`/upsert`,`/delete`), `inventory/chains` (+`/upsert`,`/delete` legacy alias), `inventory/scan/evm`, `inventory/watch-addresses` (+`/upsert`,`/delete`) | `inventory.ts` (+ `journey.ts`, `walletManager.ts`) | `sigillum api chains <list\|upsert\|delete>`; `sigillum api inventory <list\|chains\|watch\|scan-evm>` | Fully covered. |
| 24 | Discovery jobs | `discovery/jobs`, `discovery/jobs/cancel`, `discovery/jobs/resume` | `inventory.ts` (cancel/resume; the job list itself is surfaced via the scan flow, not fetched directly) | `sigillum api discovery jobs <list\|cancel\|resume>` | Covered; the UI job-list gap is cosmetic and the CLI lists jobs. |
| 25 | Risk | `risk/findings`, `risk/catalog` (+`/upsert`,`/delete`) | `inventory.ts` | `sigillum api risk <list\|catalog\|catalog-upsert\|catalog-delete>` | Fully covered. |
| 26 | Consolidation plans | `plans/consolidation` (+`/generate`,`/approve`,`/simulate`,`/export`) | `inventory.ts` | `sigillum api plans <list\|generate\|approve\|simulate\|export>` | Fully covered — this is the reviewed path for spending. |
| 27 | Treasury | `treasury/overview`, `treasury/policy` (+`/update`), `treasury/receive-addresses` (+`/allocate`,`/rotate`), `treasury/parties` (GET+POST) (+`/update`,`/delete`) — 9 registrations | `treasury.ts` (+ `journey.ts`, `walletManager.ts`, `setup.ts` policy update) | `sigillum api treasury <overview\|policy\|policy-update\|receive-list\|receive-allocate\|receive-rotate\|parties ...>` | Fully covered. |
| 28 | Receiving | `receiving/overview`, `receiving/refresh-balances`, `receiving/deposits/tag` | `receiving.ts` | `sigillum api receiving <overview\|refresh-balances\|tag-deposit>` | Fully covered. |
| 29 | Stealth deposits | `deposits/eth-stealth` (+`/create-native`,`/create-erc20`,`/scan-announcements`,`/delete`,`/refresh`,`/enqueue-sweep`) — 7 routes | `operations.ts` (+ `receiving.ts` list) | `sigillum api deposits <list\|create-native\|create-erc20\|scan-announcements\|refresh\|enqueue-sweep\|delete>` | Fully covered. |
| 30 | Queue (observe/process) | `queue/jobs`, `queue/process` | `operations.ts` (drives fetches and process summaries, including `operator_action_required`; `queue.ts` renders the queue view) | `sigillum api queue <list\|process>` (JSON includes per-job `state`/`last_error` and process counts) | Fully covered. |
| 31 | Manual queue enqueue | `queue/enqueue/eth-stealth-transfer`, `.../eth-stealth-erc20-transfer`, `.../eth-stealth-native-sweep`, `.../eth-stealth-erc20-sweep` | no (direct) | no | Decision (D2): maintenance auto-enqueues, and the UI covers the manual path via the deposit sweep action (`deposits/eth-stealth/enqueue-sweep`). Direct enqueue endpoints remain programmatic/API-only. |
| 32 | FIDO2 admin | `fido2/status`, `fido2/detect`, `fido2/pin/set`, `fido2/list`, `fido2/setup`, `fido2/register`, `fido2/unlock`, `fido2/remove` | `fido2.ts` + `setup.ts` (all except `status`) | `sigillum api unlock-fido2` (unlock only); local `sigillum fido2 <register\|list\|remove\|status\|unlock>` manages the same store offline | Decision (D2): admin requires a physical touch ceremony on the security key at the machine — the interactive UI flow is the operator surface; `status` is a programmatic probe. |

## Verification

- Route registrations in `crates/sigillum-daemon/src/routes/mod.rs`: **124**
  (`grep -c '\.route('`). Method endpoints: **125** (`/api/treasury/parties`
  is GET+POST on one registration).
- Sum of routes across the 32 rows above: **124** — every registration
  appears in exactly one row.
- No row is UI=no and CLI=no without an explicit decision (rows 4, 12, 19,
  22, 31 carry D2 decisions; rows 2, 15 are machine-plumbing decisions).
