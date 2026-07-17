# Changelog

Notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
from 1.0.0 onward, per the stability policy in `docs/stability.md`.

## [Unreleased]

### Added

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

### Changed

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

### Fixed

- **Treasury planner panics** — Malformed hot/treasury address pairs and
  undecodable policy floor values now fail closed to the default destination
  with a warning instead of unwrapping.
- **Status response contract drift** — Console contract types now match the
  daemon's `active_compartment` (`compartment_id`/`compartment_label`) and
  `unlocked_compartments` shapes exactly.
- **Stealth interoperability caveat documented** — `docs/architecture.md` now
  states the shared-secret hash convention deviation (keccak256 over the
  32-byte x-only encoding vs the 33-byte compressed-point scheme-1 SDK
  convention) and warns against pointing third-party ERC-5564 senders at
  Sigillum meta-addresses until the conformance switch lands.

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
  `.app`/`.dmg` bundling with a generated icon set, ad-hoc signing by default
  with env-gated full signing/notarization; macOS is the supported desktop
  platform, while Linux desktop is compile-only.
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

- **`sigillum-fido2` no-HID builds** — Compiling with `default-features = false`
  keeps the API surface but returns explicit errors for hardware-only
  operations; the release gate compiles, tests, and lints this configuration.
- **Chaos crash-boundary proof** — The write-ahead kill-in-flight regression
  uses a real subprocess and SIGKILL at the durable `prepared` and
  `submitted_unknown` barriers, with a test-only minimal unlock fixture so the
  proof remains reliable across CI runners.

[Unreleased]: https://github.com/caelator/sigillum/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/caelator/sigillum/releases/tag/v1.0.0
