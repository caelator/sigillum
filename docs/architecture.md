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
  `queued`, `blocked`, `retrying`, `sent`, `failed_terminal`, and
  `operator_action_required`; legacy `deferred` inputs normalize to
  `blocked` during recovery with a recorded reason, while
  `operator_action_required` is terminal until a future explicit operator
  action moves it out of that state
- keeps queue ownership split inside `service/queue/*`: the façade owns public
  enqueue/list methods, `payloads` owns job construction, `processing` owns the
  drain loop and retry transitions, `sweeps` owns native/ERC-20 sweep execution,
  and `state` owns normalization, legacy recovery, retry classification, and
  retry backoff tests
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
- generates reviewable approval revoke plan steps for ERC-20 allowances,
  Permit2 allowances, and NFT operator approvals
- records operator-configured DeFi receipt/share token probes as first-class
  `defi` holdings with protocol provenance, so lending, vault, staking, and LP
  positions can be surfaced locally before protocol-specific exit adapters exist
- records operator-configured trusted claim candidates as first-class `reward`
  or `airdrop` holdings keyed by claimant address, asset contract, claim
  contract, amount, protocol, optional Merkle proof evidence, and source
  provenance; these surface in inventory and consolidation planning. Standard
  `claim(uint256,address,uint256,bytes32[])` Merkle distributor candidates can
  be simulated with provider-backed `eth_call`, but claim execution remains
  blocked behind explicit review and a disabled execution gate.
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
  step source address; exports are audited and remain unsigned execution
  evidence rather than automatic queue execution
- classifies discovered inventory addresses with signer, gas, value, approval,
  stranded-value, watch-only, and dormant-candidate labels so risk and
  consolidation views can distinguish recoverable value from merely visible
  value
- keeps the gateway surface local-sidecar-only rather than treating it as an internet-facing service boundary

What it intentionally does not do today:

- remote SDK/client abstraction
- polished remote multi-host client/server story
- multi-host coordination
- SSE streams
- remote audit aggregation pipeline
- deep on-chain indexing beyond provider RPC balance checks, bounded ERC-20
  transfer-log token discovery, bounded ERC-721 transfer-log discovery with
  `ownerOf` confirmation, bounded ERC-1155 transfer discovery with `balanceOf`
  confirmation, operator-bounded ERC-20 allowance probes, operator-bounded
  Permit2 allowance probes, operator-bounded NFT approval probes,
  operator-configured DeFi receipt/share token probes, operator-configured
  trusted claim candidates, standard Merkle claim preflight simulation,
  claim-contract risk findings, and local operator-managed
  spender/operator/claim-contract risk catalog overrides
- historical receive-address discovery beyond the current EVM seed-account
  receive-branch scanner, project-xpub gap-limit scanner, imported
  receive-branch xpub scanner, imported custom receive-path xpub scanner, and
  imported account-level xpub scanner with optional custom account paths, or
  rich dormant-wallet classification with last-activity timestamps
- full token registry/indexer scraping, full ERC-1155 batch/history coverage,
  NFT metadata and spam classification, Permit2 expiration-aware risk scoring,
  external spender/operator registries, queued execution for approval revokes,
  NFT claim/swap/exit transaction
  simulation, dynamic network fee estimation, protocol-specific DeFi exit
  adapters, or claim execution adapters for airdrops/rewards
- queued execution of consolidation plans for discovered holdings outside the
  current stealth deposit sweep flow; consolidation plan exports are the current
  execution handoff boundary

## Privacy & Linkage Model

The receiving/treasury model centers on keeping payers unlinkable. Receive
allocations and stealth deposits attribute to a first-class `Counterparty`, and
consolidation is linkage-aware: `analyze_plan_linkage` (HD planner) and
`detect_stealth_sweep_linkage` (stealth sweeps) flag when funds for *different*
payers would route to the **same destination** — the single-hop common-recipient
heuristic on Ethereum's account model. The identity model is conservative: a
tagged party is one identity and each unattributed address is its own distinct
identity, so the analysis never produces a false negative (two tagged payers to
one destination are always caught) at the cost of possible false positives. When
the `block_cross_party_linkage` treasury policy is enabled (an explicit
fail-closed opt-in surfaced in onboarding, default off), warnings become hard
blockers at plan generation, approval, and stealth-sweep enqueue.

This protection is deliberately scoped to **single-hop, destination-axis**
linkage. It does not model gas-funding linkage, amount/timing correlation,
downstream re-merging of per-party destinations, or multi-hop flows, and RPC
provider calls expose queried addresses to the configured endpoint. The full
threat model and operator-discipline requirements are documented in the README's
"Privacy Model — Scope and Limitations" section; the claim is intentionally not
an unconditional unlinkability guarantee.

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
