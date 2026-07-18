# Changelog

Notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
from 1.0.0 onward, per the stability policy in `docs/stability.md`.

## [Unreleased]

### Added

- **One-time receive addresses (plan task 3.3)** — receive allocations gain
  a true one-time mode: `allocate → auto-watch → auto-sweep-on-funds →
  retire → optional purge`, driven by the background scheduler and the
  maintenance cycle with no operator in the loop.
  - `POST /api/treasury/receive-addresses/allocate` accepts additive
    `one_time` plus its policy fields: `sweep_destination_address`
    (required in one-time mode, validated against the destination
    allowlist like any sweep destination), `min_sweep_amount_hex`
    (optional sweep threshold — below it funds simply sit), and
    `purge_after_sweep` (default false). One-time mode requires a signing
    (eth-seed) wallet profile; rotation carries the policy to the
    replacement address.
  - A new `one_time_receive` stage in the scheduler cycle and in
    `maintenance/run` settles (retires on sweep settle; purges when
    configured), observes (auto-watches balances on the refresh cadence,
    writing the standard inventory rows), and enqueues ONE
    `eth_seed_native_sweep` per due allocation — deduped against the
    allocation's tracked job (mirroring the stealth-deposit sweep
    dedupe), gated on the Sweep execution family
    (`allow_plan_execution` + `allow_sweep_execution`) at enqueue
    evaluation and at drain, the destination allowlist/step cap on every
    evaluation, and the default-on `block_cross_party_linkage` posture
    (a shared destination across parties hard-blocks). Gates off means
    the allocation just accrues (`watching` / `execution_gates`);
    `execution_paused` halts the drain exactly as today.
  - Allocation records surface the derived `lifecycle_state`
    (`watching`/`sweep_queued`/`swept`/`retired`; `purged` is record
    absence plus the audit event) and `sweep_blocker`
    (`awaiting_balance`, `below_threshold`, `execution_gates`,
    `destination_policy`, `step_cap`, `cross_party_linkage`,
    `sweep_failed`, `sweep_attention`) — computed at read time, never
    persisted; the store stays schema v21. `MaintenanceRunResponse`
    carries an additive `one_time_receive` summary. New audit events:
    `treasury.receive.retire`, a `one_time` flag on
    `treasury.receive.allocate`, and the existing purge/enqueue events
    cover the rest.
  - CLI: `sigillum api treasury receive-allocate` gains
    `--one-time`/`--no-one-time`, `--sweep-destination`,
    `--min-sweep-wei-hex`, `--purge-after-sweep`/`--no-purge-after-sweep`,
    and `--counterparty-id`. The console's allocate form gains the
    one-time options ("Use this address once — sweep funds here
    automatically, then retire it"), and allocation rows show the
    One-time badge, the lifecycle pill, destination/threshold in human
    terms, and plain-English blockers.
  - Retire/purge fire when the sweep job reaches its queue family's
    terminal success state — `sent` for the legacy EthSeed family
    ("broadcast, done"; those families never poll receipts to finality)
    — documented with the other scope notes in
    `docs/architecture.md` → "One-time receive addresses". One-time
    sweeps are native-only.
- **At-rest forgetting: prune, purge, and the profile-delete cascade (plan
  task 3.2)** — the at-rest linkage ledger is no longer write-only. Three
  new surfaces, each fail-closed and audited:
  - `POST /api/inventory/addresses/delete` prunes scanned-address records
    from the wallet inventory together with their holdings and per-address
    log-scan block cursors. Selectors (`address`, `wallet_family`,
    `wallet_profile`, `provider_profile`, `chain_id`, `account_index`)
    combine with AND semantics; at least one is required so a malformed
    request can never empty the store, and a selector set matching nothing
    is a 404. Pruning removes history, not derivation: a later scan that
    re-derives a pruned index re-observes it as a fresh row (new id, fresh
    `first_seen_at_unix`), while a re-scan that does not re-derive it leaves
    it gone. The `wallet_inventory.addresses.prune` audit event records the
    selector scope and counts only — never the pruned address value.
  - `POST /api/treasury/receive-addresses/purge` permanently deletes a
    RETIRED receive allocation and the counterparty binding it carried;
    active allocations are refused with 409 (rotate retires first), unknown
    ids with 404, and the counterparty record itself always remains. Audited
    as `treasury.receive.purge`.
  - Every profile delete route (`profiles/evm|eth-stealth|eth-xpub|eth-seed
    /delete`) accepts an additive optional `prune_inventory` flag. Absent or
    false preserves the legacy behavior byte-identically (profile removed,
    inventory history orphaned). When true, one guarded operation forgets —
    before the profile registry mutation lands — the profile's scanned
    addresses, holdings, scan state (checkpoints, block cursors, and
    single-profile discovery jobs), receive allocations (ACTIVE ones are
    retire-then-purged in the same save), and the counterparty bindings
    those allocations carried; counterparties themselves remain. One
    `wallet_inventory.profile_prune` audit event carries the per-store
    counts and the mutation response repeats them as `pruned_inventory`. An
    EVM provider delete prunes rows observed through that provider; a
    stealth profile reports zeros (its deposits live in the separate
    deposits store with their own delete route, deliberately out of scope).
  - CLI: `sigillum api inventory prune-addresses` (selector flags),
    `sigillum api treasury receive-purge --allocation-id <ID>`, and
    `--prune-inventory` on all four `profiles * delete` arms. The console's
    shared confirm dialog gains an optional checkbox affordance, and the
    wallet-delete flow uses it to offer the cascade with copy stating
    exactly what will be forgotten. See
    `docs/architecture.md` → "At-rest forgetting" and `docs/backup.md` →
    "What Snapshots Retain After Pruning" (snapshot archives and the
    append-only audit trail intentionally retain what they captured).
- **Provider partitioning for inventory scans (plan task 3.1)** —
  `WalletInventoryScanRequest` gains an additive optional
  `partition_providers` flag; absent/false — and any scan where each chain
  has a single selected provider — keeps the previous behavior
  byte-identical. When engaged, every probed address is assigned to exactly
  one provider per multi-provider chain by a stable
  `SHA-256(domain ‖ chain_id ‖ address) mod N` hash over the chain's
  name-sorted provider set, so each endpoint observes only a disjoint
  subset of the address set instead of the full ordered tree; the union of
  observations, per-chain coverage (an address is still probed once per
  chain), gap-limit accounting, and cancel/resume semantics are unchanged.
  Per-provider request batches are paced with a 25–150 ms `OsRng`-seeded
  jitter (`SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS=0` disables it for tests).
  `WalletDiscoveryJob` additively records `partition_providers` and
  per-provider observed counts (`provider_partition_observations`) so
  operators can verify disjoint coverage; discovery-job resume replays the
  flag so an interrupted partitioned scan keeps its assignment. The CLI's
  `inventory scan-evm` gains `--partition-providers`. The wallet-inventory
  store bumps to schema v21 (older payloads load with both job fields
  absent). This reduces but does not eliminate provider-side linkage —
  each provider still sees its subset plus the operator IP, with no
  Tor/proxy layer. See `docs/architecture.md#privacy--linkage-model`.
- **Single-key (66-hex-char) stealth meta-addresses (plan task 2.6)** —
  meta-address parsing now accepts the EIP-5564 single-key form
  (`st:<chain>:0x<key>`, one 33-byte compressed SEC1 key serving as both
  spending and viewing key) alongside the dual-key spend‖view form.
  Generation, recipient check (full-wallet and watch-only — the view
  collapses to viewing key == spending key), and stealth-key recovery handle
  it through the unchanged code paths, pinned byte-exactly by fixed
  end-to-end vectors under both hash conventions in `sigillum-core`.
  Fluidkey's 64-byte X‖Y encoding remains explicitly rejected. See
  `docs/architecture.md#stealth-addresses-erc-5564`.
- **Persisted stealth announcement-scan cursors (plan task 2.6)** —
  `deposits/eth-stealth/scan-announcements` no longer needs a manual
  `from_block` per call: a per-(wallet profile, provider profile) cursor of
  the last scanned announcement block persists in the deposits store
  (additive with a serde default — the schema stays v3). An omitted
  `from_block` resumes at cursor+1 (`earliest` on the first scan), an
  explicit `from_block` still wins for manual rescans and never drags the
  cursor backward, and a successful scan advances the cursor to the highest
  processed log block (a `limit`-capped scan re-reads the tail next call; an
  empty range anchors at the concrete `to_block`, or the chain head for the
  default `latest`). The request gains an additive optional `reset_cursor`
  (drop the stored cursor and re-anchor from this scan's range) and
  `from_block` becomes optional on the wire; the CLI's `--from-block` is now
  optional with a new `--reset-cursor` flag. The response's
  `from_block`/`to_block` report the effective range scanned. See
  `docs/architecture.md#stealth-addresses-erc-5564`.
- **Five-destination operator console (C7)** — The console is restructured
  around operator goals: Overview, Receive, Portfolio, Move, and Vault, with a
  compact single-row topbar status strip, the unlocked hero shown once on
  Overview, and a treasury-setup journey card that collapses when complete.
- **Shared confirmation dialog** — One modal component with inform / confirm /
  typed-phrase tiers replaces native `confirm()`/`prompt()` and bespoke
  two-click arms. Queue processing, deposit sweep enqueue, receive-address
  rotation, and party deletion — previously unguarded — now require
  confirmation; bulk plan enqueue and local data reset keep their typed
  phrases; all destructive deletes share the same confirm tier.
- **Stealth-generation guardrail warnings** — `wallets/eth-stealth/generate`
  returns cautionary `warnings` when a meta-address cannot be matched to the
  vault's known stealth wallets, and when a supplied ephemeral key was already
  used for a recorded deposit (reuse derives the identical stealth address).
  Warnings propagate through deposit creation
  (`EthStealthDepositMutationResponse.warnings`), print to stderr in the CLI,
  and surface in the console as toasts plus a pinned warning box.
- **xpub hygiene warnings (plan task 3.4)** — an xpub exposes a wallet's
  ENTIRE past and future receive-address tree to anyone holding it, and the
  export/copy surface now says so everywhere: `wallets/eth-xpub/export`
  responses carry a non-blocking `warning` string (additive, serde default —
  empty from older daemons) restating the exposure, the CLI prints it to
  stderr on `sigillum api wallets xpub-export`, and the console toasts the
  warning, pins it in a warning box next to the exported branch plus a static
  note in the xpub card, and gates the first xpub copy of each session behind
  an inform-tier acknowledgement dialog. Export stays gated by session +
  compartment match (already audited); policy-gating was considered and
  deliberately not added. The unauthenticated `wallets/eth-xpub/derive`
  derivation oracle is now documented as such (rustdoc +
  `docs/architecture.md#xpub-exposure-and-the-derivation-oracle`) and traces
  each use at debug level; it is otherwise unchanged.
- **Common-gas-funder risk findings (plan task 3.5)** — the linkage analysis
  now surfaces one gas funder paying for receive addresses attributed to
  different payer identities as a structured, advisory `common_gas_funder`
  risk finding built by the local risk machinery (the same finding shapes as
  risky approvals / watch-only / stranded / dormant): one finding per chain +
  funder with a stable id, `medium` level, and evidence listing the linked
  payer identities. `ConsolidationPlan.risk_findings` carries it from the
  plan-generation common-funder pass, and
  `EthStealthDepositEnqueueSweepResponse.risk_findings` carries it from the
  stealth sponsor linkage detection (both additive, serde default, omitted
  when empty). The finding is a surfacing, never a block — execution blocking
  is unchanged and stays governed by `block_cross_party_linkage`. Coverage is
  honest: daemon-planned `fund_gas` steps and recorded stealth sponsor
  top-ups, per plan or per enqueue; manual gas funding, cross-plan sharing,
  amount/timing correlation, and multi-hop flows remain operator discipline
  (see `docs/architecture.md#privacy--linkage-model`).
- **CLI seed-wallet import and mnemonic hygiene** — New
  `profiles eth-seed upsert` arm imports an existing mnemonic via
  `--mnemonic-env`/`--mnemonic-stdin` or a hidden prompt (never argv).
  `profiles eth-seed create` now redacts the mnemonic from stdout by default;
  reveal interactively with `--reveal-mnemonic` (TTY only) or file it with
  `--mnemonic-out PATH` (owner-only 0600, never overwrites).
- **UI screenshot harness** — `scripts/ui-screenshots/` captures a
  deterministic, mock-driven shot set of the console (setup, unlock, all five
  destinations) via headless Chromium for release-evidence walkthroughs.
- **Structured error codes and field-level validation** — Every daemon error
  response now carries a stable machine-readable `code`
  (`sigillum_api::error_codes`, cataloged in `docs/stability.md`) that
  disambiguates the overloaded statuses: 403 splits into `vault_locked`,
  `execution_gate_denied`, `capability_scope_denied`, `policy_violation`, and
  the generic `forbidden` fallback; 404 into `not_found` and
  `not_initialized`; 429 into `unlock_throttled` and `rate_limited`. HTTP
  statuses and top-level `error` messages are unchanged. DTO validation can
  now also return a `fields` array of per-field errors (wire paths such as
  `allowed_destinations[0].address`), implemented for treasury policy
  updates, inventory scans, provider profile upserts, and seed/xpub wallet
  profile upserts. The client exposes `code()`/`fields()` on API errors, the
  CLI prints `error[<code>]: <message>` with one line per field error, and
  the console contracts carry both through for the upcoming UX handling.
- **Background operations API** — Long-running daemon work is tracked as an
  `Operation` resource with progress counters and cooperative cancellation:
  `GET /api/operations`, `GET /api/operations/{id}`, and
  `POST /api/operations/{id}/cancel` (404 `not_found` for unknown ids, 409
  `conflict` once the operation is terminal). Operations are in-memory,
  process-lifetime records; durable progress stays in the existing persisted
  stores. The client crate grows matching
  `list_operations`/`get_operation`/`cancel_operation` methods.
- **Async EVM discovery scans with real cancel/resume** —
  `inventory/scan/evm` accepts `run_async: true` to validate synchronously,
  spawn the scan as a background operation, and return immediately with the
  accepted discovery job and its operation (an absent flag keeps the
  synchronous contract unchanged). The scan loop checks the operation's
  cancel flag at every address index: a canceled scan keeps all persisted
  progress and the job is marked `canceled`; a mid-run failure persists the
  job as `failed` with `last_error` instead of leaking a permanently
  `running` record. `discovery/jobs/cancel` now really stops the running
  scan (409 `conflict` on terminal jobs), and `discovery/jobs/resume` starts
  a new background operation continuing from the interrupted job's persisted
  checkpoints — per-index persistence and observation upserts make resume
  free of duplicate observations. The console re-enables its discovery
  Cancel/Resume controls (cancel behind the shared confirm dialog) and the
  scan form gains a "Run in background" option that surfaces the operation
  id; the CLI gains `sigillum api inventory scan-evm --run-async`.
- **Async queue drains and maintenance cycles** — `queue/process` and
  `maintenance/run` accept `run_async: true` to validate synchronously,
  spawn the work as a background operation (`queue_process` /
  `maintenance_run` kinds), and return immediately with the operation (an
  absent flag keeps the synchronous contract byte-identical, and every run
  — sync or async — registers an operation for observability). A drain
  reports jobs attempted vs the selected-job total and honors cancellation
  BETWEEN jobs only, at the same boundary as the `execution_paused` kill
  switch and never mid-broadcast: an in-flight job finishes its attempt and
  the canceled drain reports processed vs remaining. A maintenance cycle
  reports per-stage progress (`stage:treasury_automation`,
  `stage:deposit_refresh`, `stage:queue_drain` in `related_ids`) and honors
  cancellation between stages, never mid-stage, with completed stages'
  effects durably persisted. The console's Process Queue and Run
  Maintenance actions gain a "Run in background" option that surfaces the
  operation id, and the CLI gains `sigillum api queue process --run-async`
  and `sigillum api maintenance run --run-async`.
- **SSE daemon events channel** — New `GET /api/events` route streams
  daemon state over Server-Sent Events so clients subscribe instead of
  polling (plan task 1.3 / decision D-D). The versioned vocabulary (`v: 1`
  payloads, `sigillum_api::response::events`) is deliberately minimal:
  `snapshot` (lock status + live operations, sent on connect and again as a
  resync when a subscriber falls behind the bounded per-subscriber
  channel), `operation` (registry create/state/progress transitions with
  the full post-transition record), `queue` (job state transitions at
  enqueue and at the drain/broadcast writes), and `status` (`locked`,
  `unlocked`, `compartment_switched`); heartbeat comments keep the stream
  alive through intermediaries. Auth is the standard bearer session, plus a
  `?session=` query-token alternative for browser `EventSource`
  (loopback-only; CORS unchanged). The route is the first PASSIVE read:
  connecting and staying connected does not refresh the session's
  idle-activity clock, so an always-open events tab can no longer defeat
  the vault idle auto-lock — semantics for all other routes are unchanged,
  and more read-only routes can opt into the passive verify later.
  `sigillum-client` gains `subscribe_events()` returning an
  `EventSubscription` stream of typed `DaemonEvent`s (unknown event names
  are skipped for forward compatibility); console adoption of the channel
  is a later phase.
- **Pagination, filtering, and sorting on list endpoints** — The six
  previously unbounded list endpoints accept additive optional query
  parameters: `GET /api/queue/jobs` (`state`, `kind`, `chain_id`,
  `sort=created|updated`), `GET /api/inventory/wallets` (`chain_id`,
  `funded`, `sort=address|last_scanned`, applied to the `addresses` list
  only), `GET /api/deposits/eth-stealth` (`status`, `chain_id`,
  `counterparty_id`, `sort=created|updated`), `GET /api/plans/consolidation`
  (`status`, `sort=created|updated`), `GET /api/risk/findings` (`severity`,
  `kind`, `chain_id`, `sort=severity|found_at`), and
  `GET /api/discovery/jobs` (`state`, `sort=created|updated`). All six also
  accept `order=asc|desc` (default `desc` for time/severity fields, `asc`
  for `address`) and `limit`/`offset`; a paginated response gains an
  additive `pagination` envelope (`{total, limit, offset, has_more}`).
  Parameterless requests stay byte-identical to before. Unknown or malformed
  values fail with 400 `validation_failed` naming the parameter. The client
  crate grows `list_*_with_options` methods taking typed
  `sigillum_api::request::*ListOptions` structs (legacy no-arg methods
  unchanged), and the console contracts carry the additive query/response
  types.
- **Background scheduler** — Queue retries whose backoff elapsed, receipt
  confirmation for `sent` plan-step jobs, and stealth-deposit balance
  refreshes now advance without a client calling `queue/process` or
  `maintenance/run` (plan task 1.6). A daemon-side loop
  (`service/scheduler.rs`) runs bounded cycles through the identical
  request-driven code paths, so every fail-closed invariant holds unchanged:
  no vault access while locked, `execution_paused` skips the drain stage,
  execution gates gate at drain time, the durable
  `prepared`/`submitted_unknown` barriers still bracket every broadcast, and
  no job is ever re-signed. A cycle skips when the operation guard is
  contended (no queueing behind operator-driven work), runs under a 120 s
  crash-safe time budget, and backs off exponentially (to 30 min) on
  consecutive failures with a daemon log warning. Enabled by default (queue
  tick 60 s, deposit refresh 5 min, 25-job batches); configured via
  `SIGILLUM_SCHEDULER_DISABLE=1`, `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS`, and
  `SIGILLUM_SCHEDULER_REFRESH_SECS`. Treasury automation only runs in a
  cycle when `allow_treasury_automation` is on (default off). Ticks are not
  registered as operations — only cycles that advanced work appear as
  completed `scheduler_cycle` operations (with SSE events) and record a
  `maintenance.run` audit event — and `GET /api/diagnostics` exposes the
  additive `scheduler` status block (effective config, last tick/outcome,
  consecutive-failure count, due-work counters).

### Changed

- **`block_cross_party_linkage` now defaults to ON (plan task 3.5)** —
  BEHAVIOR CHANGE: a treasury policy update that omits
  `block_cross_party_linkage` previously resolved it to `false`; it now
  resolves to `true`, and the `TreasuryPolicy` wire type deserializes an
  absent field as `true` (mirroring `require_simulation`). Cross-party
  linkage blocking is the default posture — plans routing distinct payers to
  a shared destination, or funding different parties' gas from one sponsor,
  hard-block at generation, approval, and stealth-sweep enqueue unless the
  operator explicitly sets the flag to `false`. Policies persisted by older
  daemons always carry the field, so existing configurations are unaffected;
  the flip strengthens (never weakens) the fail-closed direction. The console
  policy form shows the checkbox on for a not-yet-saved policy, the policy
  hints and setup wizard copy state the default plainly, and the change is
  recorded as a pre-tag adjustment in `docs/stability.md`. See
  `docs/architecture.md#privacy--linkage-model`.

- **Stealth sweeps and transfers now sit under the treasury execution gates
  — the stealth carve-out is closed (plan task 2.5)** — BEHAVIOR CHANGE:
  `EthStealthTransfer`, `EthStealthErc20Transfer`, `EthStealthNativeSweep`,
  and `EthStealthErc20Sweep` queue jobs previously executed regardless of the
  treasury policy gates, a fail-open asymmetry against the seed family. They
  now map to the same Sweep execution family as the `EthSeed*` equivalents:
  enqueue (`/api/queue/enqueue/eth-stealth-*` and the deposit sweep paths,
  including refresh auto-enqueue) is refused with 403
  `execution_gate_denied` unless the treasury policy is enabled with
  `allow_plan_execution` AND `allow_sweep_execution` on, and the drain
  re-checks the gate per job — a job enqueued while the gate was open blocks
  (never signs or broadcasts) once the gate closes, and resumes when it
  reopens. **With every gate off (the default), stealth sweeps no longer
  execute; operators must open the sweep gate to keep stealth sweeps
  running.** The sponsor gas top-up keeps its 2.4 carve-out (policy-gated on
  `allow_gas_topups` at enqueue time; the drain-level pause checks still halt
  it), and the console policy screen now states that the sweep gate covers
  stealth deposit sweeps and transfers. Recorded as a pre-tag adjustment in
  `docs/stability.md`.
- **ERC-5564 stealth shared-secret hash switched to the compressed-point
  scheme-1 standard** — New stealth payments and deposit records now derive
  the shared-secret hash as keccak256 over the 33-byte compressed SEC1 point,
  the de-facto convention of the ScopeLift `stealth-address-sdk`
  (Umbra-compatible tooling), replacing Sigillum's pre-release x-only 32-byte
  encoding. Derivation is pinned byte-exactly by fixed external test vectors
  (SDK-published keys, independently computed expectations) in
  `sigillum-core`. Pre-switch payments stay detectable and spendable:
  detection (`wallets/eth-stealth/check`, announcer scans) dual-probes the
  standard then the legacy convention in one pass, deposit records carry a
  `stealth_hash_convention` stamp (`compressed33`/`x32`) that sweeping uses
  for key derivation, the deposits store migrates to schema v3 stamping all
  pre-existing records `x32`, and a missing or wrong stamp falls back to
  probing both conventions with derived-address verification. Sweep/transfer
  queue jobs carry the record's stamp; `StealthPaymentRef`,
  `EthStealthDeposit`, `EthStealthGenerateResponse`,
  `EthStealthCheckResponse`, and the stealth `QueueJobPayload` variants gain
  the additive optional/defaulted convention field. Fluidkey's 64-byte X‖Y
  encoding remains explicitly incompatible and unsupported. See
  `docs/architecture.md#stealth-addresses-erc-5564`.
- **Stealth deposit detection is watch-only** — The announcer scan
  (`deposits/eth-stealth/scan-announcements`), `wallets/eth-stealth/check`,
  and the meta-address export now run from the EIP-5564 `checkStealthAddress`
  key material — viewing private key + spending PUBLIC key — via the new
  `sigillum-core` watch-only API (`EthereumStealthWatchView`,
  `derive_watch_only_sigillum_ethereum_stealth_wallet`,
  `check_ethereum_stealth_address_watch_only`,
  `check_ethereum_stealth_address_any_watch_only`; the full-wallet check entry
  points delegate to the same core). The spending private key no longer
  enters the detection path at all; it is derived exclusively at
  sweep-signing time. Detection still requires the wallet compartment
  unlocked (the viewing key derives from its master key), and there is
  deliberately no viewing-key cache, so locking keeps zeroizing every path to
  key material. Behavior, responses, and detection results are unchanged.
- **Stealth deposit gas story: payer-attached gas + sponsor top-ups** —
  ERC-20 stealth deposits no longer require a manual external gas transfer to
  sweep. (a) Deposit creation accepts `request_gas` (+ optional
  `gas_amount_wei_hex`, defaulting to the provider's static sweep gas
  estimate) on `deposits/eth-stealth/create-native` and `/create-erc20`: the
  announcement metadata then follows the EIP-5564 SHOULD layouts — the
  57-byte native layout (`view tag ‖ 0xeeeeeeee ‖ sentinel ‖ payment+gas
  total`) or the token layout (`view tag ‖ transfer(address,uint256) selector
  ‖ token contract ‖ expected amount`) — so standards-aware payer wallets can
  attach the requested gas/asset info, per the EIP's "Recipients' transaction
  costs" sponsorship pattern. (b) The announcer scan parses the same layouts
  defensively (unknown layouts decode to "no hints", never an error) and
  auto-populates the created deposit's asset kind, token contract, and
  expected amount, so `--token-address` is no longer needed for
  standards-following announcements; refresh flips a `funded_needs_gas`
  deposit to `funded` once native gas arrives. (c) When the treasury policy
  allows gas top-ups, enqueueing a sweep for a gas-starved ERC-20 deposit
  plans an `eth_stealth_gas_topup` queue job ahead of the sweep: 1.5x the
  sweep's estimated gas (seed-path formula, `max_gas_topup_wei_hex` cap
  honored), paid by the stealth wallet's gas sponsor — a key derived
  deterministically from the compartment master key
  (`sigillum/eth-stealth/v1/{wallet}/sponsor`, funded out-of-band by the
  operator) and re-verified against the job at execution. The sweep records
  the top-up in `prerequisite_job_ids` and stays blocked until the top-up
  broadcasts and the gas is confirmed on-chain; sponsor funding across
  different counterparties flows through the same cross-party linkage
  accounting as seed-plan `fund_gas` (warn by default, hard-block
  `cross_party_linkage` when the policy is on). Deposit records gain
  `requested_gas_wei_hex`, `gas_topup_job_id`, and `gas_topup_job_state`; the
  console deposits view gains a "Request gas from payer" option and renders
  requested/attached gas plus what a `funded_needs_gas` deposit waits for;
  the CLI gains `--request-gas`/`--gas-amount-wei-hex`. See
  `docs/architecture.md#stealth-addresses-erc-5564`.
- **Console data displays are human-readable** — Shared formatting helpers
  render wei hex as ETH/token units, unix seconds as locale timestamps, and
  chain ids as registry names, with raw values behind a details disclosure.
- **Treasury policy form** — Every input has a visible label with units, the
  camelCase state line moves behind a "Technical state" disclosure beneath a
  plain-English summary of what the policy does, and the dense legal hints sit
  behind a "How this policy protects you" fold.
- **Console dead code removed** — The unused typed API client and session
  actions, unreachable hero setup/locked copy, stale "increment B2" roadmap
  text, and unused view helpers are gone.
- **Passive reads for console polling** — `GET /api/status`,
  `GET /api/operations`, and `GET /api/operations/{id}` join `GET /api/events`
  as passive reads: they authenticate without refreshing the session's
  idle-activity clock, so an always-open console can no longer defeat the
  15-minute vault idle auto-lock.
- **Chain-registry route canonicalization** — `/api/chains*` is the canonical
  chain-registry route set; the `/api/inventory/chains*` trio remains working
  as a deprecated legacy alias slated for removal at the next major version.

### Fixed

- **Treasury planner panics** — Malformed hot/treasury address pairs and
  undecodable policy floor values now fail closed to the default destination
  with a warning instead of unwrapping.
- **Status response contract drift** — Console contract types now match the
  daemon's `active_compartment` (`compartment_id`/`compartment_label`) and
  `unlocked_compartments` shapes exactly.
- **Stealth interoperability caveat documented** — `docs/architecture.md`
  stated the pre-release shared-secret hash convention deviation and warned
  against pointing third-party ERC-5564 senders at Sigillum meta-addresses.
  Superseded by the conformance switch above: the caveat is now a conformance
  statement.
### Documentation

- Reframed the public project landing page around Sigillum's local-first EVM
  wallet-management product, added an indexed documentation map and community
  health files, clarified the unreleased/unsupported status and invalid RC tag,
  corrected stale crate descriptions, and enabled private vulnerability
  reporting.

## [1.0.0] - 2026-07-10

### Added

- **Vault and unlock** — Two-tier local vault storage with Tier 1 plaintext API
  keys for local automation and Tier 2 AES-256-GCM encrypted secrets, Argon2id
  passphrase-derived key wrapping, FIDO2 hardware-key unlock with Shamir shard
  recovery, per-compartment keyspaces with compartment switching,
  passphrase-encrypted snapshot export/restore, and journaled destructive flows
  with tested crash-point recovery.
- **Local daemon and operator console** — Local Axum daemon with bearer
  session-token auth over loopback HTTP, full-session-by-default authorization
  with capability tokens admitted only by explicit scope checks, an embedded
  web operator console
  covering first-time setup, lock/unlock, secret management, FIDO2 key
  management, snapshots, local audit feed, and diagnostics, plus transit-style
  encrypt/decrypt/HMAC endpoints, atomic sidecar-backed persistence with
  automatic backup restore and corrupt-file quarantine, restart recovery with
  telemetry, and categorized maintenance summaries reporting failures by cause.
- **Ethereum stealth custody** — ERC-5564-style stealth meta-address export,
  one-time deposit-address derivation and receiving, bounded announcement-log
  scanning, provider-backed deposit monitoring, local EIP-1559 native and
  ERC-20 transfer signing with optional broadcast, and sweep orchestration
  through a persistent queue.
- **Discovery, inventory, and risk** — Multi-chain EVM inventory scans from one
  provider profile or all configured chains in a single request, resumable
  block-range cursors for ERC-20/ERC-721/ERC-1155 transfer-log discovery,
  ERC-1155 TransferSingle and TransferBatch decoding, operator-imported local
  token registries with provenance-recorded balance probes and no external
  feeds, opt-in per-collection NFT metadata fetching with provenance plus local
  spam heuristics that bucket suspicious assets without auto-hiding them,
  ERC-20/Permit2/NFT operator allowance discovery, last-activity block
  derivation with per-chain dormancy windows, a watch book, and a local
  operator-managed risk catalog.
- **Consolidation planning and policy-gated execution** — Plan generation for
  native/ERC-20/NFT sweeps, approval revokes for ERC-20, Permit2, and NFT
  operators, DeFi exits, Merkle claims, and sponsor-funded `fund_gas` gas
  top-ups; `eth_call` preflight simulation with a recorded fee basis using
  static fees or live EIP-1559 estimates and a policy-configurable freshness
  window; step dependency ordering; export as call manifests or Safe
  Transaction Builder batches; execution OFF by default behind fail-closed
  policy gates including `allow_plan_execution`, per-family sweep/revoke/exit/
  claim/top-up gates, an `execution_paused` kill switch, and gate-flip audit
  events carrying session fingerprints; enqueue with full server-side
  re-validation and typed confirmation naming the step count and total value;
  seed-wallet signing that re-verifies a simulation-evidence hash before
  touching key material and zeroizes derived keys; durable write-ahead of exact
  signed transaction bytes and hash in `prepared`, a durable
  `submitted_unknown` marker before RPC, and a preemptive pause latch between
  jobs and immediately before broadcast; recovery checks receipts or resubmits
  only the exact bytes and never re-signs the job; deterministic nonce-too-low,
  underpriced, and revert rejections park for operator action rather than
  creating a replacement signature; receipt confirmation reaches per-chain
  finality depth before terminal `confirmed`; and
  linkage enforcement at execution enqueue at parity with plan generation and
  approval.
- **Treasury, receiving, and linkage policy** — Purpose-labeled receive-address
  allocations with rotation, first-class counterparties, a treasury console with
  cross-wallet value/risk/plan roll-ups, treasury policy guardrails covering a
  destination allowlist, per-step and per-plan native caps, required simulation,
  and `block_cross_party_linkage` single-hop cross-party linkage blocking
  enforced at plan generation, approval, and enqueue, plus policy-driven
  hot-wallet floor/target routing and maintenance-cycle hot-wallet overflow/
  refill treasury automation behind the default-off `allow_treasury_automation`
  opt-in with floor <= target <= overflow hysteresis and distinct
  generated-versus-enqueued reporting.
- **Chain registry** — Schema-versioned registry with built-in entries for
  Ethereum, Base, Arbitrum One, OP Mainnet, and Polygon PoS plus
  operator-defined custom chains, per-chain finality depth, dormancy block
  window, Permit2 address override, and provider self-check warnings for
  unregistered chains.
- **DeFi exit adapters and Merkle claims** — Bounded exit-adapter set covering
  Aave v3 withdraw, generic ERC-4626 redeem, Uniswap v2 LP removeLiquidity with
  dependency-ordered approve then removeLiquidity, plan-time reserve-derived
  minimum amounts, and per-chain operator-supplied router addresses, and Lido
  wstETH unwrap; positions matching no adapter remain review-only.
  `merkle-distributor-v1` claim execution is behind the `allow_claim_execution`
  opt-in and gated on simulation, risk-catalog review, and explicit step
  approval; claim failures park for the operator and are never auto-retried.
- **Gateway sidecar** — Disabled-by-default local-sidecar experimental payment
  observation surface with project API keys, amount-checked observations, and
  webhook delivery that re-resolves and pins HTTPS targets at send time.
  Balance reads are not finality proof and do not claim supported 1.0 payment
  confirmation; privileged third-party invoice signing is not implemented.
- **Desktop app** — Tauri v2 macOS shell that runs the daemon in-process on a
  fresh loopback port, shares the `~/.sigillum` data directory, keeps a single
  focused instance, shows a tray with live lock state and "Lock now", locks and
  hides to tray on close, and locks before quit so loaded keys are zeroized;
  `.app`/`.dmg` bundling with a generated icon set, project-enforced full-bundle
  ad-hoc signing by default with fail-closed env-gated Developer ID signing and
  notarization; macOS is the supported desktop platform, while Linux desktop
  is compile-only.
- **CLI** — Setup flows, `sigillum doctor` host preflight, snapshot commands, a
  daemon launcher, and JSON operator commands under `sigillum api` covering
  sessions, compartment listing, provider/wallet profiles, deposits, inventory
  scans and discovery controls, token registries, chains, risk catalog, plans
  including the gated enqueue commands, receiving, treasury policy, queue
  inspection and pause/resume, maintenance runs, transit helpers, read-only EVM
  queries, and wallet xpub/stealth export and derive helpers; no sign, send, or
  broadcast commands by design.
- **Release engineering** — Single release gate at `./scripts/check-release.sh`
  running locked cargo metadata and architecture guardrails, default and no-HID
  FIDO2 coverage, daemon UI
  install/typecheck/tests/build with generated-asset freshness checks, Rust
  fmt/tests/clippy, adversarial property-based API fuzzing, a real local-daemon
  runtime smoke with vault write/read canaries, browser smoke, a desktop bundle
  check, `cargo audit`, `cargo deny`, whitespace checks, and a tracked-tree
  mutation guard; CI runs the full gate on fixed Ubuntu/macOS version lines with
  immutable action commits and a nightly deep-fuzz schedule.

### Security

- **Daemon authorization** — Full daemon sessions are required by default;
  capability tokens are admitted only through explicit `require_scope()` checks
  on the routes that declare them. Scoped sessions are hidden from optional
  observability surfaces such as `/api/status`.
- **Payment truth** — Deposit refresh and the gateway poller compare observed
  and expected amounts with full 256-bit semantics; dust transfers do not
  satisfy larger payment intents. Gateway lifecycle states report balance
  observations, not finality-backed confirmations. A privileged third-party
  invoice-signing callback path is not implemented.
- **Queue execution durability** — Every queue family durably records exact
  signed bytes and hash as `prepared`, persists `submitted_unknown` before RPC,
  and recovers by receipt lookup or exact-byte resubmission without re-signing.
- **Kill switch preemption** — Queue pause sets a lock-free latch before
  acquiring the operation mutex so an in-flight drain can be halted through the
  HTTP API without waiting for the batch to finish.

### Fixed

- **Plan-step dependency finality** — Dependent execution jobs remain unsigned
  while prerequisites are merely prepared, submitted, or broadcast, and may
  proceed only after every prerequisite reaches receipt-confirmed success.
  Crash-recovered prepared authority follows the same rule: unknown submissions
  remain receipt-observable but cannot be resubmitted early.
- **Release evidence network and gas-chain contract** — Restricted execution
  evidence to Ethereum Sepolia (`11155111`) plus Base Sepolia (`84532`),
  Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`), rejecting mainnet,
  arbitrary, and non-integer chain IDs. F6 schema v2 now proves five
  transactions for four families, including distinct ordered `fund_gas` and
  dependent-sweep legs bound to their runtime plan, job, step, receipt, address,
  prerequisite, and broadcast-time identities.
- **`sigillum-fido2` no-HID builds** — Compiling with `default-features = false`
  keeps the API surface but returns explicit errors for hardware-only
  operations; the release gate compiles, tests, and lints this configuration.
- **Chaos crash-boundary proof** — The write-ahead kill-in-flight regression
  uses a real subprocess and SIGKILL at the durable `prepared` and
  `submitted_unknown` barriers, with a test-only minimal unlock fixture so the
  proof remains reliable across CI runners.
- **Release tag contract** — Annotated-tag validation queries the authoritative
  remote tag and peeled commit instead of a runner-local ref that checkout may
  rewrite, safely recovers a locally absent tag object through a non-tag scratch
  ref, pins tag-object identity across jobs, and enforces monotonic retained RC
  numbers. The normal release gate exercises annotated, object-absent,
  lightweight, wrong-SHA, off-main, skipped-number, malformed, and
  changelog-negative fixtures before tag time.
- **macOS release bundle signature** — Replaced the weak signature-metadata
  check that allowed RC3's linker-only app to pass. Credential-free builds now
  explicitly sign the whole app bundle, partial Apple credentials fail closed,
  Developer ID mode requires notarization and stapling for both app copies and
  the dmg (including an explicit post-Tauri dmg submission), and both source-gate
  and release-artifact paths require strict app and read-only-mounted dmg
  verification with bound plist, sealed resources,
  expected identifier/mode, matching CDHash, and negative regression fixtures.

[Unreleased]: https://github.com/caelator/sigillum/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/caelator/sigillum/releases/tag/v1.0.0
