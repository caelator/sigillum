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
  session, render, status, refresh, action, and domain-view modules and Vite
  writes the checked-in `app.js` runtime that the daemon embeds
- exposes transit-style encrypt/decrypt/HMAC operations derived from the active compartment master key
- centralizes daemon business rules behind an application-service layer instead of spreading them across route handlers
- keeps wallet inventory discovery, risk derivation, and consolidation planning
  separated inside the daemon service so future asset/protocol adapters have
  explicit homes rather than accumulating in one inventory module
- stores provider profiles and stealth wallet profiles for internal EVM integration, with explicit compartment binding so queued work does not depend on the session's currently active compartment
- tracks stealth deposit records and refreshes them against configured providers
- queues direct sends and sweep jobs, including deferred jobs that need more balance or gas
- keeps queue state normalization, legacy recovery, retry classification, and
  retry backoff tests in a focused service submodule so the queue service file
  can remain centered on enqueueing and execution
- exposes a maintenance cycle that refreshes deposits, auto-enqueues sweeps, and processes queue work
- keeps the gateway surface local-sidecar-only rather than treating it as an internet-facing service boundary

What it intentionally does not do today:

- remote SDK/client abstraction
- polished remote multi-host client/server story
- multi-host coordination
- SSE streams
- remote audit aggregation pipeline
- deep on-chain indexing beyond provider RPC balance checks
- seed/xpub gap-limit discovery, historical receive-address scanning, or
  dormant-wallet classification
- token scraping, NFT inventory, allowance scanning, DeFi position discovery, or
  airdrop/reward discovery
- consolidation planning for discovered holdings outside the current stealth
  deposit sweep flow

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
   required typed UI migration modules, and externalized API/client tests.
