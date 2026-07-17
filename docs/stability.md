# Stability Policy

This document declares which Sigillum surfaces are stable as of 1.0.0, which
surfaces are explicitly unstable, and how versions evolve. It applies from the
`v1.0.0` tag onward.

## Versioning

- The workspace follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
  from 1.0.0: breaking changes to a stable surface require a major version,
  backward-compatible additions require a minor version, and fixes require a
  patch version.
- Crates are NOT published to crates.io; releases are annotated git tags (`v*`)
  plus GitHub Release artifacts (macOS desktop bundle, macOS/Linux CLI binaries),
  and library consumers use git or path dependencies. The version promises attach
  to the tagged releases.
- macOS is the supported desktop platform at 1.0; the daemon and CLI are
  supported on macOS and Linux; the Linux desktop build is compile-only; Windows
  is unsupported.

### Pre-tag adjustments

No valid `v1.0.0` tag or GitHub Release has published yet, so the stability
promises above are not yet in force. Until the first valid tag, release
candidates may adjust stable-candidate surfaces with the change recorded in
`CHANGELOG.md`. Current adjustments since `1.0.0-rc.5`:

- `sigillum api profiles eth-seed create` redacts the mnemonic from stdout by
  default (new `--reveal-mnemonic` and `--mnemonic-out PATH` flags control
  delivery), and `profiles eth-seed upsert` imports an existing mnemonic.
- `EthStealthGenerateResponse` and `EthStealthDepositMutationResponse` gained
  a backward-compatible `warnings` array (additive, defaults to empty).
- `ErrorResponse` gained a required `code` (stable machine-readable string
  from the catalog below; envelopes serialized before this change deserialize
  with the fallback `unknown`) and an optional `fields` array carrying
  per-field validation errors (`{field, message}`). Daemon errors now
  disambiguate the overloaded 403/404/429 statuses through the catalog, and
  CLI daemon errors print as `error[<code>]: <message>` (with one indented
  line per field error) when the daemon supplied a code.
- New routes `GET /api/operations`, `GET /api/operations/{id}`, and
  `POST /api/operations/{id}/cancel` expose the daemon's background-operation
  registry (in-memory, process-lifetime). `WalletInventoryScanRequest` gained
  an optional `run_async` flag (absent/false preserves the synchronous
  behavior exactly); `WalletInventoryScanResponse` and
  `DiscoveryJobMutationResponse` gained an additive optional `operation`
  field. `sigillum api inventory scan-evm` gained a matching `--run-async`
  flag.
- Discovery-job cancel/resume semantics are now real and tightened: cancel
  cooperatively stops the running scan (or marks an orphaned `running` job
  canceled) and conflicts (409 `conflict`) on terminal jobs; resume starts a
  NEW background operation and discovery job continuing from the interrupted
  job's persisted checkpoints and conflicts on completed or still-running
  jobs. Previously both verbs merely rewrote the stored status string.
- A discovery scan that fails mid-run now persists the job as `failed` with
  `last_error` (previously the record stayed `running` forever), which also
  makes it resumable.
- `WalletInventoryScanRequest` gained an optional `partition_providers` flag
  (absent/false — and any scan with a single selected provider per chain —
  preserves the previous behavior exactly): when engaged, same-chain address
  probes are distributed across that chain's provider profiles by a stable
  per-address hash so each endpoint observes a disjoint subset.
  `WalletDiscoveryJob` gained additive optional `partition_providers` and
  `provider_partition_observations` fields (both absent for non-partitioned
  jobs; wallet-inventory store schema v21), and discovery-job resume
  replays the flag.
  `sigillum api inventory scan-evm` gained a matching
  `--partition-providers` flag.
- New route `GET /api/events` exposes the daemon's SSE event channel (plan
  task 1.3 / decision D-D): `snapshot` (on connect and on lag resync),
  `operation`, `queue`, and `status` events with `v: 1` versioned payloads
  from `sigillum-api` (`DaemonEvent` and friends). Auth is the standard
  bearer session (plus a `?session=` query-token alternative for
  `EventSource`, loopback-only), but the route is the first PASSIVE read:
  it authenticates without refreshing the session's idle-activity clock, so
  an always-open stream cannot defeat the idle auto-lock. Session semantics
  for every other route are unchanged.
- The six previously unbounded list endpoints gained additive optional query
  parameters for filtering, sorting, and offset pagination (plan task 1.5).
  A parameterless request is byte-identical to before: full list in store
  order, no `pagination` key. When `limit` and/or `offset` is supplied, the
  response gains an additive `pagination` envelope
  (`{total, limit, offset, has_more}`). Parameters: `GET /api/queue/jobs`
  accepts `state`, `kind`, `chain_id`, `sort=created|updated`;
  `GET /api/inventory/wallets` accepts `chain_id`, `funded`,
  `sort=address|last_scanned` (applies to the `addresses` list only);
  `GET /api/deposits/eth-stealth` accepts `status`, `chain_id`,
  `counterparty_id`, `sort=created|updated`; `GET /api/plans/consolidation`
  accepts `status`, `sort=created|updated`; `GET /api/risk/findings` accepts
  `severity` (matches `risk_level`), `kind` (matches `category`, free-form),
  `chain_id`, `sort=severity|found_at`; `GET /api/discovery/jobs` accepts
  `state`, `sort=created|updated` (`updated` = `completed_at_unix`, falling
  back to `started_at_unix`). All six also accept `order=asc|desc`
  (requires `sort`; default `desc` for time/severity fields, `asc` for
  `address`) and `limit`/`offset` (non-negative integers). Unknown or
  malformed values fail with 400 `validation_failed` naming the parameter.
  `sigillum-client` grows matching `list_*_with_options` methods taking
  typed `sigillum_api::request::*ListOptions` structs; the legacy no-arg
  methods are unchanged.
- A background scheduler (plan task 1.6) now advances queue retries whose
  backoff elapsed, receipt confirmation for `sent` plan-step jobs, and
  stealth-deposit balance refreshes without a client calling
  `POST /api/queue/process` or `POST /api/maintenance/run`. The loop runs
  through the identical drain/refresh code paths (durable
  `prepared`/`submitted_unknown` barriers, never re-sign, execution gates
  and the `execution_paused` kill switch re-checked at drain, no vault
  access while locked) and is enabled by default: queue tick 60 s, deposit
  refresh 5 min, bounded batches (25 jobs / the policy deposit-refresh
  limit), guard acquisition with skip-on-contention, a 120 s cycle budget,
  and exponential backoff (to 30 min) on consecutive failures. Env
  overrides: `SIGILLUM_SCHEDULER_DISABLE=1` (also `true`/`yes`) turns the
  loop off, `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS` and
  `SIGILLUM_SCHEDULER_REFRESH_SECS` retune the cadences (clamped to >= 1).
  Treasury automation runs in a cycle only when the persisted policy has
  `enabled && allow_treasury_automation` (both default off). Observability:
  cycles that actually advanced work register a completed `scheduler_cycle`
  operation (a new `Operation::kind`; kinds remain free-form strings clients
  must treat as opaque) and a `maintenance.run` audit event, and
  `DiagnosticsResponse` gained an additive `scheduler` block
  (`SchedulerStatusResponse`: effective config, last tick time, last cycle
  outcome, consecutive-failure count, due-queue-job count, next retry
  timestamp) that deserializes with defaults from older payloads.
- The passive-read set (plan task 1.7) extends beyond `GET /api/events` to
  the console's polling trio: `GET /api/status`, `GET /api/operations`, and
  `GET /api/operations/{id}` now authenticate without refreshing the
  session's idle-activity clock, so an always-open console cannot defeat the
  idle auto-lock. Mutations and all other reads are unchanged.
- Route duplication cleanup (plan task 1.8): `/api/chains`,
  `/api/chains/upsert`, and `/api/chains/delete` are designated the CANONICAL
  chain-registry routes (they are what `sigillum-client`, the CLI, and the
  console call); the `/api/inventory/chains*` trio is a legacy alias kept
  working and deprecated — new integrations must use `/api/chains*`, and the
  alias is scheduled for removal at the next major version. Collection-route
  convention going forward: `GET` reads plus `POST` mutations
  (`…/upsert`, `…/delete` verb forms), matching the rest of the daemon API.
- **ERC-5564 stealth hash-convention switch (plan tasks 2.1+2.2)**: new
  stealth payments and deposit records derive the shared-secret hash as
  keccak256 over the 33-byte compressed SEC1 point (the ScopeLift
  `stealth-address-sdk` scheme-1 convention) instead of the pre-release
  x-only 32-byte encoding — a behavior change to
  `wallets/eth-stealth/generate` and everything derived from it. On-disk
  migration: the deposits store advances to schema v3 and stamps all
  pre-existing records `x32` (legacy); new records are stamped `compressed33`.
  Dual-decode keeps pre-switch payments detectable and spendable: the check
  endpoint and announcer scans probe standard-then-legacy, a match re-stamps
  the record, sweeping derives the key with the record's stamp, and a missing
  or corrupt stamp re-probes both conventions (derived-address verification
  makes probing fail-safe). `StealthPaymentRef` gains an optional
  `stealth_hash_convention`; `EthStealthDeposit`,
  `EthStealthGenerateResponse`, and `EthStealthCheckResponse` gain it with a
  standard-convention serde default; the four stealth `QueueJobPayload`
  variants gain it optionally (absent = probe). Fluidkey's 64-byte X‖Y
  encoding remains unsupported.
- Single-key (66-hex-char) EIP-5564 meta-addresses are now accepted anywhere
  a meta-address is parsed (plan task 2.6); previously they failed `invalid
  meta-address format`. The dual-key spend‖view form is unchanged.
- **Stealth announcement-scan cursors (plan task 2.6)**:
  `EthStealthAnnouncementScanRequest.from_block` changed from required
  `String` to optional `Option<String>` (wire-compatible: callers that send
  it are unaffected; omitting it now resumes from the persisted per-(wallet,
  provider) cursor instead of failing validation) and the request gained an
  optional `reset_cursor`. The deposits store gained the additive
  serde-defaulted `announcement_scan_cursors` list (schema stays v3) with the
  new `EthStealthAnnouncementScanCursor` DTO.
- **Stealth execution-gate carve-out closed (plan task 2.5)**: the stealth
  transfer/sweep queue jobs (`EthStealthTransfer`, `EthStealthErc20Transfer`,
  `EthStealthNativeSweep`, `EthStealthErc20Sweep`) no longer bypass the
  treasury execution gates — a pre-switch behavior change. They gate under
  the Sweep execution family exactly like the `EthSeed*` equivalents:
  enqueue (`/api/queue/enqueue/eth-stealth-*` and the deposit sweep paths)
  returns 403 `execution_gate_denied` unless the treasury policy is enabled
  with `allow_plan_execution` and `allow_sweep_execution` on, and the drain
  re-checks the gate per job. Stealth sweeps are therefore BLOCKED BY DEFAULT
  for existing installs until the operator opens the sweep gate; jobs already
  `sent` (broadcast, pre-terminal) are unaffected. The `EthStealthGasTopup`
  job keeps its `allow_gas_topups` enqueue-time gate unchanged.
- `EthXpubExportResponse` gained a backward-compatible `warning` string
  (additive, serde default — empty from older daemons) restating that an xpub
  exposes the wallet's entire past and future receive-address tree (plan task
  3.4).
- **`block_cross_party_linkage` defaults to ON (plan task 3.5)**: a
  `TreasuryPolicyUpdateRequest` that omits the field now resolves to `true`
  (previously `false`), and the `TreasuryPolicy` wire type deserializes an
  absent field as `true` — cross-party linkage blocking is the default
  posture and turning it off requires an explicit `false`. Policies persisted
  by older daemons always carry the field explicitly, so their chosen value
  is unaffected; the flip strengthens (never weakens) the fail-closed
  direction. Behavior change for hand-written or older clients that relied
  on the implicit off default: plans routing distinct payers to a shared
  destination (or sharing a gas sponsor across parties) are now hard-blocked
  unless the operator opts out.
- `ConsolidationPlan` and `EthStealthDepositEnqueueSweepResponse` gained a
  backward-compatible `risk_findings` array (additive, serde default, omitted
  when empty) carrying structured `common_gas_funder` findings from the
  linkage analysis (plan task 3.5): one advisory `RiskFinding` (category
  `common_gas_funder`, `medium`, stable per-(chain, funder) id) when one gas
  sponsor funds receive addresses attributed to different payer identities.
  Advisory only — execution blocking is unchanged and stays governed by
  `block_cross_party_linkage`.
- **At-rest forgetting (plan task 3.2)**: two new routes —
  `POST /api/inventory/addresses/delete` (prune scanned-address rows plus
  their holdings and per-address block cursors; selectors
  `address`/`wallet_family`/`wallet_profile`/`provider_profile`/`chain_id`/
  `account_index` combine with AND semantics, at least one required, no match
  → 404) and `POST /api/treasury/receive-addresses/purge` (permanently
  delete a RETIRED receive allocation and its counterparty binding; active →
  409, unknown → 404; the party record always remains). `EvmProfileDeleteRequest`
  (shared by all four profile delete routes) gained an additive optional
  `prune_inventory` flag: absent/false preserves the legacy delete behavior
  byte-identically; true runs the forget cascade (the profile's inventory
  rows, scan state, receive allocations — active ones retire-then-purged —
  and bindings) in the same operation, and the four profile-mutation
  responses carry the per-store counts in an additive optional
  `pruned_inventory` field. New audit event kinds:
  `wallet_inventory.addresses.prune`, `treasury.receive.purge`,
  `wallet_inventory.profile_prune` (scope and counts only, never pruned
  address values). CLI: `sigillum api inventory prune-addresses`,
  `sigillum api treasury receive-purge`, and `--prune-inventory` on all four
  `profiles * delete` arms.
- **One-time receive addresses (plan task 3.3)**: `TreasuryReceiveAllocateRequest`
  gained additive optional `one_time`, `sweep_destination_address`,
  `min_sweep_amount_hex`, and `purge_after_sweep` (omitted behaves exactly
  as before; one-time fields without `one_time` are a 400). The
  `TreasuryReceiveAllocation` wire type gained additive serde-defaulted
  fields (`one_time`, `sweep_destination_address`, `min_sweep_amount_hex`,
  `purge_after_sweep`, `sweep_job_id`, and the read-time derived
  `lifecycle_state`/`sweep_blocker` — absent on older records and on
  non-one-time allocations); the wallet-inventory store stays schema v21
  (additive fields load with defaults, no migration). One-time allocations
  are advanced by a new `one_time_receive` stage in the scheduler cycle and
  in `maintenance/run` (the maintenance operation's stage list and progress
  total grow from 3 to 4 — additive for clients, which already see the
  stage names as opaque markers), and `MaintenanceRunResponse` carries an
  additive optional `one_time_receive` summary. Auto-sweeps enqueue as
  ordinary `eth_seed_native_sweep` jobs under the existing Sweep
  execution-family gates (no gate semantics change). New audit event kind:
  `treasury.receive.retire` (id + reason); `treasury.receive.allocate`
  details gained an additive `one_time` flag (absent on pre-3.3 events).
  CLI: `treasury receive-allocate` gained `--one-time`/`--no-one-time`,
  `--sweep-destination`, `--min-sweep-wei-hex`,
  `--purge-after-sweep`/`--no-purge-after-sweep`, and `--counterparty-id`.

## Stable at 1.0

Breaking any of these is a major-version event:

1. **`sigillum-api` wire shapes** - the request/response JSON contracts shared
   by the daemon, client, CLI, and UI. Enumerated values serialize to their exact
   current strings, and unknown inbound values deserialize into a
   forward-compatible catch-all variant instead of failing, so newer daemons never
   break older clients on values alone.
2. **Daemon route paths and semantics** - the HTTP route paths, methods, auth
   expectations (bearer session tokens over loopback), and fail-closed validation
   semantics of the local daemon API. This includes the `GET /api/events` SSE
   channel: its event names (`snapshot`, `operation`, `queue`, `status`), the
   `v: 1` versioned payload shapes, and its passive-read idle semantics
   (connecting does not defer the idle auto-lock) are part of the stable
   surface; new event names or optional payload fields may be added within 1.x,
   and clients must ignore ones they do not recognize.
3. **CLI syntax** - command names, arguments, environment variables, and JSON
   output shapes of `sigillum` and the `sigillum api` operator commands.
4. **On-disk formats** - every persisted daemon store is schema-versioned and
   evolves by migration only: a newer daemon reads every older schema version and
   migrates it forward; formats are never rewritten incompatibly in place;
   downgrading a data directory to an older daemon is not supported.
5. **`sigillum-core` public traits** - the `SecretStore` and `VaultLifecycle`
   trait contracts.
6. **`TreasuryPolicy` fail-closed defaults** - every execution and automation
   capability (`allow_plan_execution` and its per-family gates,
   `allow_claim_execution`, `allow_gas_topups`, `allow_treasury_automation`)
   defaults to OFF and requires an explicit operator opt-in. New capabilities
   ship default-off behind their own opt-ins. The `block_cross_party_linkage`
   privacy protection takes the opposite posture: it defaults to ON (since
   plan task 3.5) and turning it off requires an explicit `false`. Weakening
   a fail-closed default is treated as a breaking change and will not happen
   within 1.x; strengthening one (as the linkage flip did) is a pre-tag
   adjustment recorded above.

### Error code catalog

Every non-2xx daemon response carries the `ErrorResponse` envelope from
`sigillum-api`: `code` (one of the strings below), `error` (human-readable
message), optional `action` (machine-readable remediation payload), and
optional `fields` (per-field validation breakdown, `{field, message}` with
wire field paths such as `allowed_destinations[0].address`).

Codes are stable strings, deliberately not an enum: a newer daemon may add
codes within 1.x, and clients must treat unrecognized codes as opaque and
fall back to the HTTP status. Removing or repurposing an existing code is a
major-version event.

| Code | HTTP | Meaning |
| --- | --- | --- |
| `validation_failed` | 400 | Request body failed DTO validation; `fields` carries the per-field breakdown when the DTO reports one. |
| `bad_request` | 400 | Malformed or inconsistent request outside DTO validation. |
| `typed_confirmation_mismatch` | 400 | Typed-confirmation phrase mismatch; `action` carries the exact expected phrase. |
| `unauthorized` | 401 | Missing/invalid session token, or credential (passphrase, snapshot key) did not authenticate. |
| `forbidden` | 403 | Generic refusal not covered by a more specific code (e.g. plan step-state refusals). The 403 fallback; may gain more specific siblings over time. |
| `vault_locked` | 403 | Vault or relevant compartment is locked, or no compartment is active; unlock (or switch compartment) and retry. |
| `execution_gate_denied` | 403 | A treasury execution gate denied the operation: `execution_paused` kill switch, a per-family `allow_*_execution` gate, a per-profile `execution_enabled` flag, or a claim/gas-topup gate. |
| `capability_scope_denied` | 403 | Session is valid but lacks the required capability scope (or the endpoint requires a full daemon session). |
| `policy_violation` | 403 | A treasury transaction-policy rule blocked the action; `action` carries the policy reason. |
| `not_found` | 404 | The requested resource does not exist. |
| `not_initialized` | 404 | The daemon vault has not been initialized; complete first-run setup. |
| `conflict` | 409 | Operation conflicts with current daemon state (e.g. already unlocked, duplicate profile). |
| `locked_in_progress` | 423 | The daemon is draining unlocked state; retry once the lock completes. |
| `rate_limited` | 429 | An upstream provider (EVM RPC) rate-limited the request. |
| `unlock_throttled` | 429 | Too many failed unlock attempts; the daemon enforces a cooldown. |
| `internal` | 500 | Unexpected internal failure. |
| `unavailable` | 503 | The daemon is up but not ready to serve (startup recovery running). |

## Unstable at 1.0

These may change in any release without a major-version bump:

- Internal daemon Rust modules - `sigillum-daemon`'s module layout and non-route
  internals are implementation details.
- The operator console DOM - element IDs, selectors, markup structure, and the
  generated `app.js`/`styles.css` assets.
- The gateway sidecar - `sigillum-gateway` keeps its local-sidecar preview
  positioning; its API keys, payment-intent, and webhook surfaces are preview
  quality. Payment creation is disabled by default behind
  `GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS=1`; balance observations are not finality
  proof and are exposed only as the latest balance observation. Privileged
  third-party invoice-signing callbacks are not implemented.
- The `sigillum-sdk` and `sigillum-server` facade crates.
- Anything in the documented 1.0 non-goals: non-EVM chains (Bitcoin/UTXO,
  Solana, Tron, Cosmos), swap execution and DEX routing, price/valuation feeds,
  external runtime registries/feeds (token lists, spender reputation, and spam
  lists stay operator-imported local files), Lido withdrawal-queue exits (wstETH
  unwrap only), remote/hosted/multi-host/internet-facing modes, crates.io
  publishing, and Windows support. These do not exist at 1.0 and no compatibility
  promise attaches to them.

## Assurance boundary

The 1.0 claims rest on the source-verified local-first release gate
(`./scripts/check-release.sh`: tests, adversarial property-based coverage,
runtime smoke, supply-chain audits) plus recorded soak and testnet evidence. No
external penetration test has been performed, and the release does not claim one.

## Recorded residual risk: local execution surface

Sigillum 1.0 adds policy-gated execution of consolidation plans. With
`allow_plan_execution` (and the relevant per-family execution gates) enabled, a
stolen session token on the local machine can move funds. The shipped mitigations
— typed confirmation at enqueue, per-family fail-closed policy gates, gate-flip
audit events carrying session fingerprints, the `execution_paused` kill switch
latched between jobs and immediately before broadcast, durable exact-byte
submission recovery that never re-signs a prepared job, and policy re-reads at
both enqueue time and queue-drain time — detect and bound
that misuse; they do not prevent it. FIDO2 tap-to-execute is the named post-1.0
hardening candidate for closing this gap. Operators who do not accept this risk
should leave the execution gates off (their default), which preserves the
export-only plan handoff.
