# Architecture

## Current Shape

Sigillum is currently a local Rust workspace with one strong implementation path:

- `sigillum-core` provides the traits, errors, file-backed vault, and passphrase helpers.
- `sigillum-fido2` handles local FIDO2/HID registration and shard recovery.
- `sigillum-daemon` exposes a local Axum API plus an embedded browser UI.
- `sigillum-api` defines the shared transport DTOs consumed by the daemon and async client.
- `sigillum-client` is an async adapter for that local daemon API.
- `sigillum-cli` provides setup, unlock, daemon launch, and local management commands.
- `sigillum-gateway` is a local-sidecar payment preview surface that talks to the daemon over local HTTP.

`sigillum-sdk` and `sigillum-server` still exist in the workspace, but the repo's
product direction remains local-on-your-computer rather than hosted or internet-facing.

`sigillum-desktop` is a Tauri v2 desktop shell for the same local boundary: it
starts `sigillum-daemon` in-process through `run_with_handle` on a background
Tokio runtime, chooses an ephemeral loopback port, waits for the daemon to
accept TCP connections, and then opens a native WKWebView window at
`http://127.0.0.1:<port>/`. The webview navigates directly to the daemon origin
so the existing same-origin CSP remains intact; the crate does not serve UI
assets through `tauri://`, inject tokens, or create a separate frontend surface.
`run_with_handle` is an additive variant of `run_with_options` that passes the
launcher the daemon's `Arc<AppState>` plus its Tokio runtime handle once the
listener is bound, so the shell can drive lock state in-process without a new
HTTP auth surface. On top of that the shell adds a single-instance guard
(avoiding a second daemon contending for the per-data-dir lock), native menus
with working clipboard items, persisted window geometry, and a system tray that
shows live lock state and a "Lock now" action. Closing the window auto-locks
(via `AppState::lock_now`) and hides to the tray; quitting zeroizes keys before
exit.

## Dependency Direction

```text
sigillum-core
├── sigillum
├── sigillum-client
├── sigillum-fido2
├── sigillum-daemon
├── sigillum-cli
└── sigillum-gateway
```

The important boundary is still `sigillum-core`: the rest of the workspace should depend on its traits and error model rather than on transport details.

## Core Interfaces

Two traits define the local vault boundary:

- `SecretStore` for Tier 1 and Tier 2 reads and writes
- `VaultLifecycle` for initialization, loading a master key, and zeroizing it

That split is the main architectural seam worth preserving as the project grows.

## Storage Model

Sigillum uses a two-tier secret model:

- Tier 1 stores API keys in plaintext JSON for local automation.
- Tier 2 stores secrets in an AES-256-GCM encrypted blob that requires an in-memory master key.

Standalone `FileVault` defaults to:

```text
~/.sigillum/
├── api_keys.json
└── vault.enc
```

`FileVault` treats a poisoned master-key or write-serialization mutex as an
unrecoverable invariant failure and aborts the process. It never consumes the
poisoned inner value; deployments therefore need a supervisor for clean
restart.

FIDO2 and multi-compartment flows use:

```text
~/.sigillum/
├── .initialized
├── fido2_keys.json
└── compartments/
    ├── 0/
    │   ├── api_keys.json
    │   ├── vault.enc
    │   ├── meta.enc
    │   ├── passphrase.salt
    │   └── passphrase_wrapped_key.enc
    └── ...
```

`meta.enc` is compartment metadata encrypted with that compartment's master key. `fido2_keys.json` stores registered key material and shard blobs, but not plaintext compartment definitions.

## Unlock Flows

Today there are two real unlock paths:

- Passphrase: Argon2-derived wrapping key decrypts a stored master key for one or more compartments.
- Local FIDO2: HID access recovers enough encrypted shards to reconstruct one or more compartment master keys.

Both flows end by loading compartment master keys into local process memory.

## Daemon Model

The daemon is a local service, not a distributed secrets platform.

The full HTTP route surface and its UI/CLI parity decisions are enumerated in [operator-surface-parity.md](./operator-surface-parity.md).

```text
Browser UI / local HTTP client
            |
            v
      sigillum-daemon
            |
            v
   unlocked compartment vaults
            |
            v
      ~/.sigillum/...
```

Current daemon behavior:

- runs on `localhost`
- serves an embedded HTML/JS UI
- issues bearer session tokens for session-gated access over local HTTP
- tracks active compartment per session
- keeps unlock state process-global inside the local daemon
- supports per-session logout without forcing a global lock
- keeps unlocked master keys in daemon memory until locked
- can export and restore passphrase-encrypted whole-tree snapshots
- keeps a local append-only audit log for state-changing operations
- journals destructive operations so pending work is visible after interruption
- records those pending operations as typed, schema-versioned journal documents rather than free-form JSON payloads
- records audit history as typed, schema-versioned line documents while keeping the public audit API stable for clients and the embedded UI
- exposes authenticated daemon diagnostics for operational visibility
- loads a startup-time runtime policy so queue limits, refresh limits, retry timing, and provider observation concurrency live behind one explicit seam instead of scattered literals
- persists non-vault operator state behind schema-versioned JSON documents so storage evolution can add explicit migrations instead of implicit file-shape drift
- composes the HTTP route surface from domain routers so endpoint wiring stays aligned with lifecycle, storage, wallet, deposit, queue, and FIDO2 service boundaries
- renders the embedded operator UI from checked-in frontend assets under
  `crates/sigillum-daemon/ui/src`; the Rust host only assembles HTML/CSS/script
  assets and injects the CSP nonce, while `app.ts` imports TypeScript API,
  session, render, status, refresh, action, shell, and domain-view modules and
  Vite writes the checked-in `app.js` runtime and generated `styles.css` that
  the daemon embeds; authored CSS now lives under `ui/src/styles/*` in ordered
  token, layout, form, component, workspace, responsive, and polish modules,
  while the UI domain modules own setup shell states, FIDO2 controls,
  wallet/profile operations, inventory/risk/plans, and
  deposit/queue/maintenance rendering with lightweight DOM smoke tests covering
  those seams
- exposes transit-style encrypt/decrypt/HMAC operations derived from the active compartment master key
- centralizes daemon business rules behind an application-service layer instead of spreading them across route handlers
- keeps wallet inventory discovery, risk derivation, and consolidation planning
  separated inside the daemon service so future asset/protocol adapters have
  explicit homes rather than accumulating in one inventory module
- stores provider profiles, stealth wallet profiles, and Ethereum seed/xpub
  receive profiles for internal EVM integration, with explicit compartment
  binding so queued work does not depend on the session's currently active
  compartment; xpub profiles can either use Sigillum's project-derived receive
  branch, an imported external `external_receive_xpub` branch with optional
  operator-supplied `external_receive_path`, or an imported account-level
  `external_account_xpub` that is normalized into a receive branch; imported
  xpub profiles remain watch-only and non-executable
- keeps profile-backed send construction and provider/wallet lookup helpers in
  `service/profiles/sends.rs` and `service/profiles/resolution.rs`, leaving
  `profiles.rs` centered on profile CRUD and seed/xpub import handling
- tracks stealth deposit records, discovers matching ERC-5564 announcements from
  bounded provider log scans, and refreshes balances against configured providers
- queues direct sends and sweep jobs with explicit persisted states:
  `queued`, `blocked`, `retrying`, `prepared`, `submitted_unknown`, `sent`,
  `confirmed`, `failed_terminal`, and `operator_action_required`; legacy
  `deferred` inputs normalize to
  `blocked` during recovery with a recorded reason, while
  `operator_action_required` is terminal until a future explicit operator
  action moves it out of that state. Every queue family first signs to a
  durably persisted `prepared` record containing the exact raw transaction and
  its locally derived hash plus payload/binding hashes. The drain persists
  `submitted_unknown` before the first RPC submission, verifies provider and
  receipt transaction identity, and recovery may check the receipt or resubmit
  only those exact bytes; it must never re-sign that job. API responses redact
  signed bytes, and terminal or affirmatively broadcast states clear them from
  the live queue document. Backup synchronization is best-effort, so an older
  `queue.json.bak` can retain signed bytes after a backup-write failure until a
  later successful refresh. The data directory and every retained backup are
  transaction-execution authority and require owner-only permissions plus
  host/full-disk protection. `sent`/`confirmed` are
  `PlanStepExecution`-only distinctions (W7.4): `sent` means
  broadcast-and-awaiting-confirmation, and `confirmed` means the receipt
  reached the chain registry's `finality_blocks` depth (W1.1) with a success
  status; `EthSeed*`/`EthStealth*` jobs keep `sent` as their terminal success
  state after the same prepare/submission barriers
- keeps queue ownership split inside `service/queue/*`: the façade owns public
  enqueue/list methods, `payloads` owns job construction, `processing` owns the
  drain loop and both durable submission barriers, `serialization` limits
  in-flight `PlanStepExecution` work per source address + chain id, `sweeps`
  owns native/ERC-20 sweep preparation, `plan_steps/signing.rs` signs without
  network I/O, `broadcast` owns exact-byte submission and crash recovery for
  every queue family, `plan_steps/receipts.rs` owns receipt confirmation, and
  `state` owns normalization and recovery invariants
- keeps queue transport ownership aligned across crates: queue request and
  response DTOs live under `sigillum-api/src/request/queue.rs` and
  `sigillum-api/src/response/queue.rs` with top-level re-exports preserved,
  while the async client and CLI queue bridge live in
  `sigillum-client/src/queue.rs` and `sigillum-cli/src/daemon_api/queue.rs`
- keeps EVM provider JSON-RPC transport and provider error classification in
  `service/evm/rpc.rs`, while the parent service module retains the higher
  blast-radius wallet signing, route-facing balance operations, and shared
  address/quantity helpers
- exposes a maintenance cycle that refreshes deposits, auto-enqueues sweeps, and processes queue work
- runs a background scheduler (`service/scheduler.rs`, plan task 1.6) so
  queue retries whose backoff elapsed, receipt confirmation for `sent`
  plan-step jobs, and stealth-deposit refreshes advance without a client
  calling `queue/process` or `maintenance/run`. Each tick runs a bounded
  cycle through the SAME code paths as the request-driven endpoints:
  treasury automation only when the persisted policy has
  `enabled && allow_treasury_automation` (both default off), a bounded
  deposit refresh at most once per refresh interval (default 5 min), the
  one-time receive lifecycle (plan task 3.3 — settle retired/purged
  allocations, observe one-time balances on the same refresh cadence,
  enqueue due one-time sweeps; see "One-time receive addresses" below),
  and a bounded 25-job queue drain (default cadence 60 s). Fail-closed
  invariants are preserved by construction: the cycle skips outright while
  the vault is locked (no vault access without unlock), `execution_paused`
  skips the drain stage while the drain loop still re-checks the kill switch
  between jobs, and the execution gates gate at drain time exactly as today. A cycle
  acquires the daemon's `operation_guard` like every other mutating path,
  but a cycle that cannot take it within 500 ms SKIPS rather than queueing
  up behind operator-driven work, so per-(source, chain) serialization is
  never violated and no drain storms form; each cycle is bounded by a 120 s
  time budget (abandonment is crash-equivalent under the durable queue
  barriers), and consecutive failures back off exponentially (to 30 min)
  with a daemon log warning. The loop mints an ephemeral full session per
  cycle and revokes it on every exit, so background work cannot defeat the
  idle auto-lock. Ticks are not registered as operations (the registry
  retains 50): only a cycle that actually advanced work (processed > 0 jobs
  or refreshed > 0 deposits) registers a completed `scheduler_cycle`
  operation (with SSE events from the registry) and records a
  `maintenance.run` audit event, keeping background value movement
  accountable; `GET /api/diagnostics` exposes the loop's status (effective
  config, last tick time and outcome, consecutive-failure count, due-work
  counters) under its additive `scheduler` block. The loop is enabled by
  default and configured via `SIGILLUM_SCHEDULER_DISABLE`,
  `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS`, and
  `SIGILLUM_SCHEDULER_REFRESH_SECS`
- generates reviewable approval revoke plan steps for ERC-20 allowances,
  Permit2 allowances, and NFT operator approvals
- records operator-configured DeFi receipt/share token probes as first-class
  `defi` holdings with protocol provenance, so lending, vault, staking, and LP
  positions can be surfaced locally; the implemented exit-adapter set covers
  Aave v3 withdraw, ERC-4626 redeem, Uniswap v2 LP `removeLiquidity`, and Lido
  wstETH unwrap, while unsupported positions remain review-only
- records operator-configured trusted claim candidates as first-class `reward`
  or `airdrop` holdings keyed by claimant address, asset contract, claim
  contract, amount, protocol, optional Merkle proof evidence, and source
  provenance; these surface in inventory and consolidation planning. Standard
  `merkle-distributor-v1` claim candidates can be simulated with provider-backed
  `eth_call`; execution is a fail-closed policy opt-in under
  `allow_claim_execution` (default off), gated on a simulation pass,
  risk-catalog review, and explicit approval. A claim that reverts parks as
  `operator_action_required` and is never auto-retried
- emits local risk findings for `reward` and `airdrop` claim candidates, using
  the claim contract as the review subject and applying risk-catalog overrides
  so trusted, high-risk, or critical claim contracts are visible before any
  claim transaction is considered
- preflights native sweeps, ERC-20 sweeps, NFT sweeps, ERC-20 approval revokes,
  Permit2 allowance revokes, NFT operator revokes, and standard Merkle claim
  candidates with provider-backed `eth_call` evidence; native sweep preflight
  reserves gas from the transfer value using the provider profile's max-fee
  policy, and zero-value token/NFT/revoke/claim preflights verify inventoried
  native gas against the same provider fee policy
- exports approved, simulated, and unblocked consolidation plan steps as
  source-address-aware call manifests, with optional Safe Transaction
  Builder-compatible batches only when a supplied Safe address matches the
  step source address; audited unsigned exports remain available, and Sigillum
  additionally supports controlled, policy-gated, fail-closed queue execution
  of approved, simulated, and unblocked plan steps, default off
- maintains an implemented chain registry with built-ins for Ethereum, Base,
  Arbitrum One, OP Mainnet, and Polygon PoS, plus operator-defined custom EVM
  chains
- keeps consolidation-plan execution fail closed under the
  default-off `allow_plan_execution` master gate and the default-off per-family
  `allow_sweep_execution`, `allow_revoke_execution`, `allow_exit_execution`,
  `allow_claim_execution`, and `allow_gas_topups` gates. The stealth
  transfer/sweep queue families (`EthStealthTransfer`,
  `EthStealthErc20Transfer`, `EthStealthNativeSweep`, `EthStealthErc20Sweep`)
  gate under the same Sweep family as their `EthSeed*` equivalents (plan task
  2.5): enqueue is refused with `execution_gate_denied` and the drain
  re-checks the gate per job, so with the gates off (the default) no stealth
  funds move either. The
  `execution_paused` kill switch is controlled through
  `POST /api/queue/pause|resume`, and every gate or pause flip emits a typed
  audit event. Pause sets an in-memory `AtomicBool` latch before waiting for the
  queue's operation mutex; the drain checks it between jobs and as the final
  step before network submission. Durable policy persistence still serializes
  behind the mutex, and startup restores the latch from that policy
- enqueues plan steps only after server-side re-validation of approval,
  simulation evidence and freshness, blockers, treasury policy, linkage at
  parity with plan generation and approval, and execution gates; single-step
  enqueue requires explicit confirmation, while bulk enqueue requires typed
  confirmation naming the step count and total value
- re-verifies the simulation-evidence hash at drain time before any key
  material is touched; a mismatch parks as `operator_action_required` and is
  never signed. Signing resolves the nonce once, then the exact signed bytes and
  hash cross a durable `prepared` barrier; `submitted_unknown` crosses a second
  durable barrier before RPC. A restart checks for a receipt and may resubmit
  those bytes, but never derives another signature for the job. Deterministic
  `nonce too low` and underpriced rejections park as
  `operator_action_required`; no replacement transaction is signed. Receipt
  polling still confirms to the chain registry's finality depth before entering
  terminal `confirmed`, with single-in-flight serialization per (source
  address, chain)
- runs hot-wallet overflow/refill treasury automation only behind the
  `allow_treasury_automation` opt-in (default off); generated work still passes
  through the standard policy, linkage, simulation, approval, and execution
  gates
- classifies discovered inventory addresses with signer, gas, value, approval,
  stranded-value, watch-only, and dormant-candidate labels so risk and
  consolidation views can distinguish recoverable value from merely visible
  value
- tracks long-running work as in-memory `Operation` records
  (`GET /api/operations`, `GET /api/operations/{id}`,
  `POST /api/operations/{id}/cancel`) with progress counters and cooperative
  cancellation. Operations are a process-lifetime observability/control view;
  durable progress lives in the persisted stores (discovery-job checkpoints
  and block cursors for scans; the queue and deposit stores for drains and
  maintenance cycles), so a restart loses only the live view, never the
  ability to resume or re-drive
- streams daemon state to subscribed clients over `GET /api/events`
  (Server-Sent Events; plan task 1.3 / decision D-D, ratified 2026-07-17 —
  see the amended non-goal below). The vocabulary is deliberately minimal and
  versioned (`v: 1` in every payload; `sigillum_api::response::events`):
  - `snapshot` — first frame on every connection, carrying the lock status
    and the live (non-terminal) operations so a client syncs without a
    second request; also re-sent as a resync frame when a subscriber falls
    behind (bounded per-subscriber channel, 256 events — on lag the oldest
    events are dropped and the subscriber is resynced rather than stalled;
    emitters never block)
  - `operation` — create/state/progress transitions from the operation
    registry; payload is the full post-transition `Operation`
  - `queue` — queue-job state transitions (`{job_id, state, last_error?}`)
    emitted at the drain/broadcast writes (`service/queue/`) and on enqueue;
    deliberately not the full job record (no payload or receipt material)
  - `status` — `locked`, `unlocked`, `compartment_switched`
  Heartbeat comments (`:hb`) are emitted every 25 s so intermediaries do not
  kill the stream. Auth is the same bearer session model; because browser
  `EventSource` cannot set headers, the token is also accepted as `?session=`
  — loopback-only by design (CORS stays pinned to the loopback origin), and
  non-browser clients should prefer the `Authorization` header since URLs
  are leak-prone (logs, history, proxies). The connect verify is a PASSIVE
  read (`AppState::verify_token_passive` via
  `service::require_passive_full_session_token`): the stream neither bumps
  the session's idle clock at connect nor over its lifetime, so an
  always-open events tab cannot defeat the vault auto-lock — the session is
  evicted on the idle timeout exactly as if the stream were not there
  (proven by `crates/sigillum-daemon/tests/events_idle.rs`). The stream
  performs no re-verifies after connect, so a subscriber whose session was
  evicted keeps receiving events until it disconnects (loopback-local, and
  it learns of the lock via the `locked` status event). Passive reads (plan
  task 1.7) now also cover the console's polling trio — `GET /api/status`,
  `GET /api/operations`, and `GET /api/operations/{id}` — so an always-open
  console cannot defeat the vault auto-lock either; routes that perform work
  must stay on the active verify
- runs EVM discovery scans either synchronously inside the request (default,
  unchanged for existing clients) or as a background operation
  (`inventory/scan/evm` with `run_async: true`, or a discovery-job resume).
  Both paths share one pipeline: validation and wallet/provider/watch
  resolution happen synchronously up front, then the scan loop runs under the
  operation mutex (mutation serialization is identical to the historical
  synchronous path) and persists inventory state after every address index.
  The loop checks the operation's cancel flag at every index; on cancel it
  stops before the next index, keeps all persisted progress, and marks the
  job and operation `canceled`. Mid-run errors persist the job as `failed`
  with `last_error` instead of leaking a permanently `running` record
- resumes canceled/failed/interrupted discovery jobs as NEW background
  operations that continue from the interrupted job's persisted
  per-wallet/provider checkpoints (`discovery/jobs/resume`). Because every
  index is persisted before the next starts and observation records upsert on
  wallet/provider/chain/address keys, resume never produces duplicate
  observations
- runs queue drains (`queue/process`) and maintenance cycles
  (`maintenance/run`) either synchronously (default, unchanged for existing
  clients) or as background operations (`run_async: true`). Both paths share
  one pipeline with the historical endpoints: the request is authenticated
  synchronously, then the same work runs under the operation mutex, so
  mutation-serialization semantics are identical. A drain operation
  (`queue_process`) reports jobs attempted vs the selected-job total — the
  selection mirrors the loop's own admission decisions and adjusts live when
  per-source serialization parks or admits a job mid-drain. Drain
  cancellation is honored BETWEEN jobs only, at the same boundary as the
  `execution_paused` kill switch and never mid-broadcast (the durable
  prepared/submitted_unknown barriers bracket that region): an in-flight
  job finishes its current attempt, and the canceled drain reports
  processed vs remaining in its progress. A maintenance operation
  (`maintenance_run`) reports per-stage progress — `related_ids` carries
  `stage:treasury_automation`, `stage:deposit_refresh`, and
  `stage:queue_drain` in execution order, `progress.total` is the stage
  count, and `progress.processed` counts completed stages — and honors
  cancellation between stages, never mid-stage, with every completed
  stage's effects durably persisted. The drain stage inside a maintenance
  cycle registers no nested operation and is not canceled mid-run
- keeps the gateway surface local-sidecar-only rather than treating it as an internet-facing service boundary

What it intentionally does not do today:

- remote SDK/client abstraction
- remote or hosted operating modes, including a polished multi-host
  client/server story
- multi-host coordination
- ~~SSE streams~~ — amended 2026-07-17 per decision D-D
  (`docs/operator-surface-and-privacy-plan.md`): a minimal, loopback-only SSE
  channel (`GET /api/events`; status/queue/operation/snapshot, `v: 1`
  payloads) is ratified as a 1.x addition so clients stop polling — polling
  with the session token reset idle activity and silently defeated the
  15-minute vault auto-lock, which the channel's passive-read auth fixes.
  Anything beyond that minimal vocabulary (general pub/sub, remote
  subscribers) remains a non-goal
- remote audit aggregation pipeline
- deep on-chain indexing beyond the implemented provider-RPC balance checks,
  bounded EVM transfer-log discovery, and operator-bounded or
  operator-configured probe surfaces
- historical receive-address discovery beyond the current EVM seed-account
  receive-branch scanner, project-xpub gap-limit scanner, imported
  receive-branch xpub scanner, imported custom receive-path xpub scanner, and
  imported account-level xpub scanner with optional custom account paths, or
  rich dormant-wallet classification with last-activity timestamps
- full token registry/indexer scraping, full ERC-1155 batch/history coverage,
  and Permit2 expiration-aware risk scoring
- external runtime token, spender/operator, and spam registries or feeds (D-15)
- swap execution or DEX routing (D-13)
- non-EVM chains (roadmap phase 10)
- fiat or NFT valuation (D-16)

## Stealth Addresses (ERC-5564)

Sigillum implements ERC-5564-style stealth addresses: senders derive one-time
addresses from a recipient's stealth meta-address, and recipients detect
payments by scanning announcements with their viewing key. Meta-addresses are
derived from the compartment master key plus a wallet label and short name,
so each compartment/wallet pair yields a distinct meta-address.

**Meta-address key forms.** Both EIP-5564 meta-address forms parse: the
dual-key form (`st:<chain>:0x<spending‖viewing>`, two 33-byte compressed
SEC1 keys) and the single-key form (`st:<chain>:0x<key>`, one 33-byte
compressed key serving as BOTH spending and viewing key). Sigillum-derived
wallets always use the dual-key form; the single-key form matters for
interoperability with external meta-addresses. No path special-cases it:
generation runs the shared secret against the viewing key and the address
offset against the spending key — the same point here — and a recipient
wallet (or watch view) whose spending and viewing keys are equal checks and
sweeps through the unchanged full/watch-only paths. Fixed vectors pin a
single-key payment end-to-end (parse → generate → check → stealth-key
recovery) under both hash conventions. Fluidkey's 64-byte X‖Y encoding
remains rejected.

**Convention conformance.** Sigillum derives the shared-secret hash as
keccak256 over the 33-byte compressed SEC1 encoding of the ECDH shared point —
the de-facto scheme-1 convention implemented by the ScopeLift
`stealth-address-sdk` (the reference implementation used by Umbra-style
tooling): `keccak256(getSharedSecret(...))`, where `@noble/secp256k1`'s
`getSharedSecret` returns the compressed point by default; the view tag is the
most significant byte of that hash, and the stealth private key is
`(spendingPrivateKey + hash) mod n`. Fluidkey's variant (keccak256 over the
64-byte uncompressed X‖Y encoding) remains **incompatible** and is not
supported: wallets using it will not detect or spend payments made under the
compressed-point convention, and vice versa. Derivation is pinned byte-exactly
by fixed external test vectors in `crates/sigillum-core/src/ethereum_stealth.rs`
(SDK-published keys, independently computed expectations).

**Dual-decode for pre-switch deposits.** Before this conformance switch,
Sigillum hashed the 32-byte x-only encoding (`x32` legacy convention).
Payments created then stay detectable and spendable forever: detection
(the check endpoint and the announcer scan) probes the standard convention
first, then the legacy one, and every deposit record carries a
`stealth_hash_convention` stamp (`compressed33` or `x32`) that the sweep uses
to re-derive the stealth key. The deposits-store migration (schema v3) stamps
all pre-existing records `x32`; a matched detection re-stamps the record with
the actual convention. If a record's stamp is missing or wrong (corrupt or
hand-edited store), signing re-probes both conventions — safe because the
derived stealth address is always verified before key use. Queue jobs carry
the record's stamp from enqueue time; jobs enqueued before the switch have no
stamp and get the same probe treatment.

**Watch-only detection.** Recipient-side detection follows the EIP-5564
`checkStealthAddress` key signature: it needs only the stealth address, the
ephemeral public key, the viewing private key, and the spending PUBLIC key.
The announcer scan, `wallets/eth-stealth/check`, and the meta-address export
therefore derive the watch-only view
(`derive_watch_only_sigillum_ethereum_stealth_wallet`): the spending private
key is touched only inside the core derivation helper — to compute its public
half — and zeroized there, so no spending secret material ever enters the
detection path. The spending private key is derived exclusively at
sweep-signing time. Detection still requires the wallet compartment
**unlocked** — the viewing key derives from the compartment master key just
like the spending key does — so the win is privilege reduction in the scan
path, not scanning a locked vault. Deliberately, there is no viewing-key
cache: watch views are re-derived per operation so that locking the
compartment keeps zeroizing every path to key material.

To keep the boundary visible, the generate endpoint returns cautionary
`warnings` when a meta-address cannot be matched to any of the vault's known
stealth wallets, and when a supplied ephemeral key was already used for a
recorded deposit — reusing an ephemeral key with the same meta-address derives
the identical stealth address, which is linkable and constitutes address
reuse.

**Announcement-scan cursors.** The announcer scan persists a per-(wallet
profile, provider profile) cursor (`announcement_scan_cursors` in the
deposits store — additive with a serde default, so the schema stays v3)
holding the highest announcement block scanned. Calling
`deposits/eth-stealth/scan-announcements` without `from_block` resumes at
cursor+1; the first scan (or one with `reset_cursor: true`) starts at
`earliest`, and an explicit `from_block` always wins for manual rescans and
never drags the cursor backward. After a successful scan the cursor advances
to the highest PROCESSED log block — the same conservative semantics as the
ERC-20 transfer-log cursors, so a `limit`-capped scan re-reads the tail on
the next call — or, when the range held no logs at all, to the concrete
upper bound (a numeric `to_block`, or the chain head for the default
`latest`; other block tags leave the cursor untouched). The response's
`from_block`/`to_block` always report the effective range scanned.

**The gas story.** An ERC-20 stealth deposit cannot sweep itself: the sweep's
token transfer needs native gas on a fresh stealth address that, by design,
holds only the token. Sigillum supports two funding paths, composable per
deposit.

*Payer-attached gas (EIP-5564 metadata).* The EIP-5564 `announce` metadata's
first byte MUST be the view tag; beyond it the EIP recommends two 57-byte
SHOULD layouts (byte offsets 0-indexed here; the EIP numbers from 1): for
native sends, byte 0 the view tag, bytes 1-5 `0xeeeeeeee`, bytes 5-25 the
sentinel address `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`, bytes 25-57 the
amount of ETH being sent; for token sends, byte 0 the view tag, bytes 1-5 the
function identifier (the function selector whenever one is available), bytes
5-25 the token contract, bytes 25-57 the token amount (or NFT id). The EIP's
"Recipients' transaction costs" security note is the companion pattern: the
sender attaches a small amount of ETH to each stealth transaction, sponsoring
the recipient's subsequent transactions. Deposit creation with `request_gas`
(+ optional `gas_amount_wei_hex`, defaulting to the provider profile's static
sweep gas estimate) builds the announcement metadata accordingly
(`encode_erc5564_metadata_native` / `encode_erc5564_metadata_erc20_transfer`):
native deposits announce the payment+gas total, ERC-20 deposits announce the
`transfer(address,uint256)` layout and record the requested gas on the deposit
(`requested_gas_wei_hex`) as payer instructions. On the consume side the
announcer scan parses the same layouts defensively
(`decode_erc5564_metadata_hints` — anything that is not exactly one of the two
57-byte layouts, or carries an unrecognized native marker or selector, yields
"no hints", never an error): a token-layout announcement auto-creates an
ERC-20 deposit with the hinted contract and expected amount, so the operator
no longer passes `--token-address` for standards-following payers. Refresh
notices native gas arriving on a `funded_needs_gas` deposit and flips it to
`funded`/sweep-ready on the next pass.

*Sponsor top-ups (`fund_gas` for stealth deposits).* When the treasury policy
allows sponsor gas top-ups (`allow_gas_topups`, the same flag the seed-plan
path uses), enqueueing a sweep for an ERC-20 deposit whose last observed
native balance is short of the sweep's estimated gas plans an
`eth_stealth_gas_topup` queue job ahead of the sweep: 1.5x the sweep's
estimated gas (the seed-path formula, still capped by
`max_gas_topup_wei_hex`), sponsor solvency re-verified at execution (balance ≥
top-up + the sponsor's own transfer gas), and the sweep job carries the
top-up's id in `prerequisite_job_ids`. The drain loop mirrors the W6.4
plan-step dependency semantics for these jobs — an unmet prerequisite defers
the sweep (`blocked`, retried next drain), a failed or missing one halts it —
and the sweep's own on-chain gas check stays the authoritative gate, so the
sweep remains blocked until the top-up is not just broadcast but confirmed.
The sponsor is derived, not configured: stealth wallets have no seed phrase or
control branch, so the sponsor key derives deterministically from the
compartment master key on its own HMAC chain
(`sigillum/eth-stealth/v1/{wallet}/sponsor`,
`derive_sigillum_ethereum_stealth_gas_sponsor`), recoverable from the vault
alone; the operator funds the sponsor address out-of-band. Execution
re-derives the key, checks it against the recorded sponsor address
(defense-in-depth, like the seed signer), and treats a locked compartment as a
retryable `blocked`, never a signature. If any planning precondition fails
(policy off, cap exceeded, sponsor unavailable or insolvent, gas already
sufficient, or a live top-up already tracked) no top-up is emitted and the
deposit keeps its historical behavior: the sweep blocks on gas until the
operator funds the address manually. The top-up job keeps a deliberate
carve-out from the plan gates: it is not treasury-plan execution, its enqueue
is already policy-gated on `allow_gas_topups`, and the drain-level pause
checks still halt it. The other `EthStealth*` families no longer share that
carve-out — since plan task 2.5, stealth transfers and sweeps gate under the
`allow_sweep_execution` family (with the `allow_plan_execution` master gate)
at enqueue and drain, exactly like the `EthSeed*` equivalents.

## Privacy & Linkage Model

The receiving/treasury model centers on keeping payers unlinkable. Receive
allocations and stealth deposits attribute to a first-class `Counterparty`, and
consolidation is linkage-aware: `analyze_plan_linkage` (HD planner) and
`detect_stealth_sweep_linkage` (stealth sweeps) flag when funds for *different*
payers would route to the **same destination** — the single-hop common-recipient
heuristic on Ethereum's account model. The identity model is conservative: a
tagged party is one identity and each unattributed address is its own distinct
identity, so within any one plan two tagged payers routed to one destination are
always caught, at the cost of possible false positives. When the
`block_cross_party_linkage` treasury policy is enabled, warnings become hard
blockers at plan generation, approval, and stealth-sweep enqueue. Since plan
task 3.5 this policy **defaults to ON**: a policy update that omits the field
keeps linkage blocking engaged (an explicit `false` is now required to turn
it off), and the policy response deserializes an absent field as `true`;
policies persisted by older daemons carry their explicit value and are
unaffected. Plan-step
execution enqueue (W7.2) enforces this at parity (W7.5): linkage is
re-evaluated fresh against current state at enqueue time exactly as at
generation and approval — warnings are always recomputed and surfaced
regardless of the policy value, and the hard block applies whenever
`block_cross_party_linkage` is on, including when a plan was approved while
the policy was off and the policy is flipped on before enqueue. This is the
same single-hop, destination-axis claim described below, carried through to
execution — no additional amount/timing or multi-hop guarantee is made at
this stage either.

The F5 N1 residual caveat is that `analyze_plan_linkage` is per-plan. Two
counterparties swept to a shared destination through steps in different
consolidation plans are not clustered, nor are counterparties using a shared
`fund_gas` sponsor through steps in different plans. This is a
detection-completeness gap in the advisory analysis; when
`block_cross_party_linkage` is enabled, the hard block still fires for a
collision within any single plan. Operators should run one consolidation plan
per review cycle or set distinct per-party destinations. Cross-plan analysis is
a candidate for post-1.0 work.

This protection is deliberately scoped to **single-hop, destination-axis**
linkage. Sigillum-generated `fund_gas` top-ups are modeled by a common-funder
pass in `analyze_plan_linkage`: cross-party sponsor funding warns and hard-blocks
under the same policy. Stealth-deposit sponsor top-ups flow through the same
accounting (`detect_stealth_gas_sponsor_linkage`): one derived sponsor funding
stealth deposits tagged to *different* counterparties links those parties
on-chain via the common gas funder, so the enqueue warns by default and
hard-blocks (`policy_violation: cross_party_linkage`) when
`block_cross_party_linkage` is on. Because the stealth sponsor is derived per
wallet, any two differently-tagged gas-sponsored deposits on the same stealth
wallet trip this analysis — the mitigations are payer-attached gas
(`request_gas`), distinct stealth wallets per party, or manual funding.
Since plan task 3.5 both passes also surface the detection as a structured,
advisory `common_gas_funder` **risk finding** built by the local risk
machinery (`service/inventory/risk.rs`, the same finding shapes used for
risky approvals, watch-only value, stranded, and dormant addresses):
`ConsolidationPlan.risk_findings` carries it on generated plans (one finding
per chain + funder, stable id, `medium` level, evidence listing the linked
payer identities) and `EthStealthDepositEnqueueSweepResponse.risk_findings`
carries it on stealth sweep enqueues. The finding is a surfacing, never a
block — execution blocking stays governed exactly as above by
`block_cross_party_linkage`. What it covers: gas funding this daemon planned
(`fund_gas` steps) or recorded (stealth sponsor top-up jobs), per plan or per
enqueue. What it does NOT cover: manual gas funding done outside the daemon,
funding shared across DIFFERENT plans (the F5 N1 per-plan caveat above),
amount/timing correlation, downstream re-merging of per-party destinations,
and multi-hop flows — those remain operator discipline. RPC provider calls
also expose queried addresses to the configured endpoint. The
full threat model and operator-discipline requirements are documented in the
README's "Privacy Model — Scope and Limitations" section; the claim is
intentionally not an unconditional unlinkability guarantee.

**Provider partitioning (plan task 3.1)** narrows that RPC boundary on an
opt-in basis. When an inventory scan sets `partition_providers: true` and more
than one selected provider profile serves the same chain, each probed address
is assigned to exactly one of that chain's providers by a stable hash —
`SHA-256(domain ‖ chain_id ‖ address) mod N` over the chain's name-sorted
provider set — so every endpoint observes only a disjoint subset of the
address set instead of the full ordered tree. The union of observations,
per-chain coverage (an address is still probed once per chain), gap-limit
accounting, and cancel/resume semantics are unchanged. Assignment is stable
across scans for a fixed provider set, which keeps resume and result caching
exact; per-provider request batches are paced with a 25–150 ms CSPRNG-seeded
jitter (`SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS=0` disables it, used by tests).
Discovery jobs record `partition_providers` plus per-provider observed counts
(`provider_partition_observations`) so an operator can verify disjoint
coverage, and discovery-job resume replays the flag so an interrupted
partitioned scan keeps its assignment. When the flag is absent/false — or
every chain has a single selected provider — scan behavior is byte-identical
to the pre-flag pipeline.

The residual is stated plainly: partitioning reduces but does not eliminate
provider-side linkage. Each provider still sees its assigned subset — a
coherent, stable subset it can track across scans, observed in derivation
order within the subset — together with the operator's IP address, and there
is no Tor or proxy layer: all endpoints are contacted from one origin, so
colluding providers or a network-level observer can still cluster the subsets
by source IP and timing (the jitter only blurs intra-scan timing). Operators
who need network-layer unlinkability still need their own node or distinct
network paths per provider.

### At-rest forgetting: prune, purge, and the profile-delete cascade

On-chain unlinkability is only as strong as the at-rest linkage ledger, and
the ledger is not write-only (plan task 3.2). The wallet-inventory store
(`wallet_inventory.json`) records every probed address — even empty
gap-limit ones — with its derivation path, wallet profile, and per-provider
observation rows; receive allocations bind address → purpose → counterparty.
Three operator surfaces delete from it, each fail-closed and each writing an
audit event:

- **Scanned-address prune** — `POST /api/inventory/addresses/delete`.
  Selectors (`address`, `wallet_family`, `wallet_profile`,
  `provider_profile`, `chain_id`, `account_index`) combine with AND
  semantics; at least one is required so a malformed request can never empty
  the store, and a selector set matching nothing is a 404. Deleting an
  address row also deletes the holdings recorded for it and the per-address
  log-scan block cursors past jobs carried for it. The audit event
  (`wallet_inventory.addresses.prune`) records the selector SCOPE and counts
  only — never the pruned address value, which would re-create the linkage
  being removed (the same discipline as `treasury.receive.allocate` omitting
  derived addresses).
- **Retired-allocation purge** — `POST /api/treasury/receive-addresses/purge`.
  Permanently deletes a RETIRED receive allocation and the counterparty
  binding it carries. Active allocations are refused with 409 (rotate
  retires first; the profile cascade below retire-then-purges in one
  operation); unknown ids are a 404. The counterparty record itself always
  remains — parties are operator-managed entities; only bindings die. The
  audit event (`treasury.receive.purge`) records the allocation id and
  whether a binding was removed.
- **Profile-delete cascade** — every profile delete route
  (`profiles/evm|eth-stealth|eth-xpub|eth-seed/delete`) accepts an additive
  optional `prune_inventory` (absent/false preserves the legacy behavior
  byte-identically: only the profile, and for seed wallets the vault secret,
  is removed and inventory history is orphaned). When true, one guarded
  operation forgets, BEFORE the profile registry mutation lands: the
  profile's scanned-address rows and holdings, its scan state (resume
  checkpoints, per-address block cursors, and discovery jobs that covered
  only that profile), its receive allocations — ACTIVE ones included, which
  are retire-then-purged in the same save so no half-retired record can
  persist — and the counterparty bindings those allocations carried. One
  audit event (`wallet_inventory.profile_prune`) carries the per-store
  counts, and the mutation response carries the same counts as
  `pruned_inventory`. Scope notes: an EVM PROVIDER delete prunes the rows
  observed through that provider (its still-referenced-by-a-wallet 409 is
  unchanged); a stealth profile has no wallet-inventory surface, so its
  cascade reports zeros — stealth deposit monitors live in the separate
  deposits store with their own delete route, deliberately out of
  `prune_inventory` scope.

**What a re-scan does after a prune.** Pruning removes history, not
derivation. A later scan that re-derives a pruned index re-observes it and
records a FRESH row (new id, fresh `first_seen_at_unix`, current balances) —
that is expected and keeps the allocator and gap-limit logic honest; a
re-scan that does not re-derive the index leaves it gone. What never
resurrects: receive allocations and counterparty bindings are operator
actions, not scan products, so no scan recreates them. One candid
consequence of deterministic derivation: once every record of an index is
gone (rows pruned AND allocations purged), the next allocation re-derives
the lowest unused index again — the same address a counterparty may have
seen before. Forgetting is local; it cannot unsend an address.

**What stays.** The audit log is append-only by design: prune/purge/cascade
EVENTS (scope and counts, never address values) remain in the trail
forever. Snapshot archives and `setup/reset` archives taken before a prune
retain everything they captured — see `docs/backup.md`.

### One-time receive addresses (plan task 3.3)

A receive allocation created with `one_time: true` carries its own sweep
policy — `sweep_destination_address` (required), `min_sweep_amount_hex`
(optional threshold; unset means any nonzero balance), and
`purge_after_sweep` (default false) — and runs the full
`allocate → auto-watch → auto-sweep-on-funds → retire → optional purge`
lifecycle with no operator in the loop. Creation validates the destination
against the destination allowlist/policy rules like any sweep destination
and requires a signing (eth-seed) wallet profile (the auto-sweep signs
locally; xpub/watch-only profiles are rejected up front). Rotation carries
the one-time policy to the replacement address ("same promise, fresh
address").

**The state machine**, derived at read time from the record's `status`, its
tracked `sweep_job_id`, the queue, the freshest observed balance, and the
treasury policy — never persisted as duplicated state:

| `lifecycle_state` | Derivation |
|---|---|
| `watching` | Active record, no live sweep job. `sweep_blocker` says why no sweep yet: `awaiting_balance` (no observation), `below_threshold`, `execution_gates`, `destination_policy`, `step_cap`, `cross_party_linkage`, `sweep_failed`, `sweep_attention` |
| `sweep_queued` | The tracked sweep job is active (queued, blocked, retrying, prepared, submitted_unknown) |
| `swept` | The tracked job reached its queue family's terminal success state — `sent` for the legacy EthSeed family, `confirmed` under W7.4 finality. The next settle pass retires the allocation |
| `retired` | Terminal record state (auto-retired on settle, or rotated). The index is never re-issued, exactly like rotate-retire |
| `purged` | Not a record state: the record is GONE (3.2 purge semantics) and the `treasury.receive.purge` audit event is the trail |

**How the automation reuses the existing sweep path.** The scheduler and
maintenance cycles run a one-time stage (`service/inventory/one_time.rs`,
wired into `service/scheduler.rs` and `service/maintenance.rs` as the
`one_time_receive` stage) with three passes. SETTLE retires every active
one-time allocation whose tracked sweep job settled — same index semantics
as rotate-retire but no replacement is issued — and, with
`purge_after_sweep`, deletes the record through the 3.2 purge semantics
(both audited: `treasury.receive.retire`, `treasury.receive.purge`; the
auto-watch observation row keeps the index reserved against re-issue even
after the record is gone). OBSERVE (the auto-watch, on the deposit-refresh
cadence) queries the wallet profile's provider for each active one-time
address and upserts the standard inventory address row — the same row
shape the manual receiving refresh writes, one provider per allocation
(profile-routed, compatible with provider partitioning). ENQUEUE evaluates
each allocation and pushes ONE `EthSeedNativeSweep` job when the observed
balance reaches the threshold, deduped against the allocation's tracked
job exactly like the stealth-deposit sweep dedupe (a live, broadcast, or
settled job suppresses re-enqueue; a terminally failed or parked job does
not auto-retry — the record shows `sweep_failed`/`sweep_attention` and the
operator rotates or purges).

**Policy and gate interactions are fail-closed at every step.** The
enqueue evaluation requires the Sweep execution family
(`allow_plan_execution` + `allow_sweep_execution` on an enabled policy,
plus the unlatched kill switch) — gates off means nothing enqueues and the
allocation simply accrues with `execution_gates` as its blocker; the drain
re-checks the same gates per job exactly as for operator-enqueued work,
and `execution_paused` halts the drain as today. The destination is
re-checked against the allowlist and the per-step native cap on EVERY
evaluation (a policy edit after creation can hold a sweep). The auto-sweep
is a destination-axis linkage input exactly like a plan sweep: two
one-time allocations bound to DIFFERENT parties (or left unattributed —
each is its own identity, mirroring the stealth identity rule) sweeping to
the SAME destination hard-block under the default-on
`block_cross_party_linkage` posture, and the plan linkage analyzer covers
the same axis for generated plan steps — per-party destinations remain the
mitigation. Every enqueue, retire, and purge is audited.

**What this does and does not protect.** It automates the receive-side
hygiene that was previously manual — one address per payment, funds moving
to a per-party destination without the address aging into a reuse point —
under the same gates and auditability as operator-driven sweeps. It does
NOT widen the privacy claim: the linkage check stays single-hop and
destination-axis (amount/timing correlation, downstream re-merging, and
multi-hop flows remain operator discipline); the provider observes the
balance queries (partitioning narrows but does not eliminate that — see
above); and the legacy EthSeed queue family treats `sent` as terminal
("broadcast, done", no receipt-to-finality polling), so retire/purge fire
on provider-accepted broadcast — a dropped or reorganized transaction
leaves the allocation retired with funds unswept, the same blind spot the
legacy sweep families already carry. One-time sweeps are native-only;
ERC-20 balances on a one-time address are out of scope (no sponsor gas
machinery is engaged).

### xpub exposure and the derivation oracle

An xpub is watch-only material — it cannot sign — but it is **not**
privacy-neutral: anyone holding a wallet's receive-branch xpub can derive and
watch that wallet's ENTIRE past and future receive-address tree, collapsing the
unlinkability that per-counterparty allocation is built to provide. The export
surface therefore restates the exposure at every layer instead of treating the
xpub as an innocuous copy target (plan task 3.4): `wallets/eth-xpub/export`
carries a non-blocking `warning` string on its response (additive, serde
default — empty from older daemons) restating the exposure, the CLI prints it
to stderr on `sigillum api wallets xpub-export` (JSON stdout stays clean), and
the console toasts the warning, pins it in a warning box next to the exported
branch, shows a static exposure note in the xpub card, and gates the FIRST
xpub copy of each session behind an inform-tier acknowledgement dialog rather
than nagging on every render. Export remains gated only by session +
compartment match (already audited as `WalletEthXpubExport`); policy-gating it
was considered and deliberately not added.

`POST /api/wallets/eth-xpub/derive` is a deliberately **unauthenticated**
derivation oracle: it is pure local math over caller-supplied public material
and touches no vault, session, or compartment state, so any local process able
to reach the loopback listener can derive addresses from an xpub it already
holds. This is accepted because the xpub is not secret to the caller and the
daemon is loopback-only; it is documented here so the shape is a conscious
contract, not an oversight, and each use is traced at debug level. The path
must never gain secret-dependent behavior.

## Architectural Priorities

The next clean architecture step is not adding more crates. It is tightening invariants around the existing local system:

1. Keep corruption handling strict and fail closed.
2. Turn the current pending-operation journal into full recovery for restore/init/remove flows.
3. Keep operational policy centralized and observable so limit changes happen through one documented policy layer rather than through scattered ad hoc constants.
4. Build richer chain/indexing and policy automation on top of the shared `sigillum-api` contract instead of route-local JSON drift.
5. Grow that indexing layer into the local wallet inventory and consolidation
   model described in [Comprehensive Wallet Management Roadmap](wallet-management-roadmap.md).
6. Preserve architecture boundaries with lightweight CI checks for known
   monoliths, embedded UI asset placement, daemon UI TypeScript type-checks,
   required typed UI and authored CSS modules, externalized API/client tests,
   and the module ownership rules in [Refactor Notes](refactor-notes.md).
