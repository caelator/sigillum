<p align="center">
  <h1 align="center">Sigillum</h1>
  <p align="center">
    <strong>Local, hardware-aware secret management in Rust.</strong>
  </p>
</p>

Sigillum is a Rust workspace for managing secrets with a two-tier model:

- Tier 1 API keys are stored in plaintext JSON for local automation.
- Tier 2 secrets are AES-256-GCM encrypted and require an in-memory master key.
- The master key can be loaded from a passphrase-derived wrapper or from FIDO2-backed shard recovery.
- A local daemon exposes an embedded web UI and HTTP API for single-machine use.
- A local-sidecar gateway provides a preview/payment surface that talks to the same local daemon.

This repository is strongest today as a local vault plus local daemon, with an
optional local-sidecar gateway for payment previews. It is intended to stay a
local-on-your-computer system rather than evolve into an internet-facing remote
secret-management platform.

## Current Scope

Implemented and working in this repository:

- `sigillum-core`: core traits, errors, file-backed vault, Argon2 helpers, wrapped-key helpers
- `sigillum-api`: shared daemon request/response contract used by the daemon and async client
- `sigillum-fido2`: FIDO2 registration/unlock support and Shamir-based shard handling
- `sigillum-daemon`: local Axum daemon with embedded UI, compartment switching, passphrase/FIDO2 unlock, snapshot import/export, local audit feed, transit-style crypto endpoints, Ethereum stealth wallet helpers, provider-backed deposit monitoring, and sweep orchestration
- `sigillum-client`: async client for the local daemon API, including session handling and snapshots
- `sigillum-cli`: setup flows, local management commands, snapshot commands,
  daemon launcher, and daemon-backed JSON operator commands
- `sigillum-gateway`: local-sidecar payment preview surface with project API keys, payment intent creation, and webhook delivery
- `sigillum-sdk`: integration surface that combines core types with the async daemon client
- `sigillum-server`: thin facade over the daemon crate for server-side embedding
- `sigillum`: meta-crate that re-exports the file-backed core

Still missing as a polished product surface:

- more local operator polish around the daemon, gateway sidecar, and desktop workflow

## Architecture

```text
sigillum/
├── crates/sigillum-core      Traits + file-backed vault
├── crates/sigillum-api       Shared daemon transport types
├── crates/sigillum-client    Async local-daemon client
├── crates/sigillum-fido2     FIDO2 + Shamir support
├── crates/sigillum-daemon    Local Axum daemon + embedded web UI
├── crates/sigillum-cli       CLI and setup flows
├── crates/sigillum-gateway   Local-sidecar payment preview surface
├── crates/sigillum-sdk       Combined client/core integration surface
├── crates/sigillum-server    Server-facing daemon facade
└── crates/sigillum           Meta-crate
```

The core trait split is intentional:

- `SecretStore` is the consumer-facing interface for reading and writing secrets.
- `VaultLifecycle` handles initialization, loading a master key, and zeroizing it.

See `crates/sigillum-core/src/traits.rs`.

## Storage Model

### Tier 1

- Plaintext JSON
- No unlock required
- Intended for local automation and low-sensitivity tokens

### Tier 2

- AES-256-GCM encrypted JSON blob
- Requires the master key to be loaded in memory
- Intended for higher-sensitivity secrets

The default standalone file layout for `FileVault` is:

```text
~/.sigillum/
├── api_keys.json
└── vault.enc
```

The daemon/FIDO2 flows use a compartment layout under `~/.sigillum/compartments/`.

The daemon's non-vault state files (`profiles.json`, `deposits.json`, and
`queue.json`) are written atomically, mirrored to `.bak` sidecars, and restored
from backup if the live file is missing or corrupted. Broken live files are
quarantined next to the restored state for inspection.

## Quick Start

### As a library

```toml
[dependencies]
sigillum = "0.1"
```

```rust
use sigillum::{FileVault, SecretStore, VaultConfig};

let vault = FileVault::new(VaultConfig::default());

vault.set_api_key("github", "ghp_...")?;

// Tier 2 requires the vault to be initialized and unlocked first.
```

### As a local daemon

```bash
cargo run -p sigillum-cli -- daemon --port 9743
```

Then open `http://localhost:9743`.

The daemon is meant to stay local on one machine. It is session-gated and
compartment-aware, and it is not intended to become a remote multi-tenant
service. Today that means unlock state is process-global inside the daemon, while active-
compartment selection is tracked per session token through bearer session-token
auth over local HTTP.

For scriptable daemon operations, the CLI now exposes JSON-oriented commands
under `sigillum api`, including session unlock, compartment switching, provider
and wallet profile management, deposit management, queue inspection, and
maintenance runs. These commands default to `http://127.0.0.1:9743`, accept
`--url`, `SIGILLUM_BASE_URL`, or `SIGILLUM_DAEMON_URL`, and use `--session` or
`SIGILLUM_SESSION_TOKEN` for authenticated calls.

Run `sigillum doctor` before treating a machine as ready. It checks the local
data directory, loopback daemon URL, daemon reachability, session-token state,
audit database readability, and common Sigillum environment variables.

The `sigillum-gateway` crate is a companion local-sidecar surface for payment
preview and webhook flow testing. It is designed to sit beside the local daemon
and should not be treated as an internet-facing boundary for this project.
When the gateway needs authenticated daemon operations, provide a pre-
established local daemon session token through `SIGILLUM_DAEMON_SESSION_TOKEN`
or `SIGILLUM_SESSION_TOKEN`.

### First-time setup

```bash
cargo run -p sigillum-cli -- setup
```

Or use the browser UI when the daemon starts with an uninitialized data directory.

## Cryptography and Key Handling

Sigillum currently uses:

- `aes-gcm` for Tier 2 encryption
- `argon2` for passphrase-derived wrapping keys
- `zeroize` and `secrecy` for safer key and secret handling
- RustCrypto `hmac` + `sha1` for RFC-compatible TOTP generation
- `ctap-hid-fido2` for USB HID FIDO2 operations

FIDO2 support is based on protecting randomly generated vault master keys with encrypted shard material. The hardware keys protect shard recovery; they do not directly derive the vault master key.

Generated passphrases use the bundled BIP-39 English word list and default to
8 words. Use `sigillum generate passphrase --words N` to override that length.

## Ethereum Stealth Flow

Sigillum now supports a local-first Ethereum stealth custody flow:

1. Export a wallet-scoped stealth meta-address from an unlocked compartment.
2. Let an external service or Sigillum itself derive one-time deposit addresses from that public meta-address.
3. Keep spend authority local by using Sigillum to verify announcements, monitor balances through provider profiles, and sign or broadcast full EIP-1559 transfer payloads.

The daemon routes are:

- `POST /api/wallets/eth-stealth/export`
- `POST /api/wallets/eth-stealth/generate`
- `POST /api/wallets/eth-stealth/check`
- `POST /api/wallets/eth-stealth/sign`
- `POST /api/wallets/eth-stealth/sign-transfer`
- `POST /api/wallets/eth-stealth/sign-erc20-transfer`
- `POST /api/wallets/eth-stealth/send-transfer`
- `POST /api/wallets/eth-stealth/send-erc20-transfer`
- `POST /api/wallets/eth-stealth/send-with-profile`
- `POST /api/wallets/eth-stealth/send-erc20-with-profile`
- `POST /api/deposits/eth-stealth/scan-announcements`

On top of that, the daemon now includes:

- EVM provider helpers for nonce, balance, ERC-20 balance, and raw-transaction broadcast
- persistent EVM provider and stealth wallet profiles, each bound to an explicit unlocked compartment
- persistent stealth deposit records for native ETH and ERC-20 flows, including bounded ERC-5564 announcement-log discovery
- wallet inventory scans for native balances, manually supplied ERC-20 probes,
  bounded ERC-20 transfer-log token discovery, operator-bounded ERC-20
  allowance probes, bounded ERC-721 transfer-log discovery with `ownerOf`
  confirmation, bounded ERC-1155 transfer discovery with `balanceOf`
  confirmation, and operator-bounded NFT `isApprovedForAll` approval probes
- persistent queue jobs for direct sends and sweep jobs
- atomic sidecar-backed persistence for profile, deposit, and queue state with
  automatic restore/quarantine behavior
- a maintenance cycle that refreshes deposits, auto-enqueues sweeps, and drains queued work

This means the current boundary is no longer “sign only.” Sigillum can now keep provider credentials internal, monitor deposit balances, sign locally, and optionally broadcast without exposing private wallet material to upstream web services.

## Web UI

The embedded UI currently supports:

- status view
- first-time passphrase or FIDO2 setup
- lock/unlock and session logout
- compartment switching
- Tier 1 and Tier 2 secret management
- FIDO2 key listing, registration, and removal
- passphrase-encrypted snapshot export and restore
- recent local audit events
- authenticated daemon diagnostics
- shared daemon/client transport schema via `sigillum-api`
- persistent pending-operation journal for destructive daemon flows
- transit-style encrypt/decrypt/HMAC operations derived from the active compartment keyspace
- ERC-5564-style Ethereum stealth meta-address export, deposit derivation, announcement scanning, local announcement checks, and local digest signing
- local EIP-1559 native ETH and ERC-20 transfer signing from derived stealth keys
- EVM provider profile management
- stealth wallet profile management
- wallet inventory scans with ERC-20, ERC-20 allowance, ERC-721, ERC-1155, and NFT operator-approval discovery controls
- stealth deposit creation, refresh, sweep enqueue, and registry browsing
- queue inspection and batch processing
- maintenance runs for deposit refresh plus queue draining
- push/copy between unlocked compartments

It intentionally does not provide:

- SSE streams
- remote client administration
- connected-client monitoring

## Feature Flags

Actual feature flags in this workspace today:

- `sigillum-core/file-backend`
- `sigillum-fido2/hid`

Example:

```toml
sigillum-core = { version = "0.1", default-features = false }
sigillum-fido2 = { version = "0.1", default-features = false }
```

## Development

The application release contract is a clean checkout with the committed
`Cargo.lock` and the pinned Rust toolchain in `rust-toolchain.toml`.

Metadata:

```bash
cargo metadata --no-deps --format-version 1
```

Build:

```bash
cargo build
```

Test:

```bash
cargo test --workspace
```

Format:

```bash
cargo fmt --all --check
```

Lint:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Supply-chain checks:

```bash
cargo audit
cargo deny check
```

## Status

Sigillum now has a shared daemon/client API contract, a service-layer split
inside the daemon, and a coherent local wallet/deposit/sweep control plane for
Ethereum stealth custody. The next architectural work is deeper crash recovery,
richer chain indexing, broader policy automation, and the wallet discovery and
consolidation roadmap in
[`docs/wallet-management-roadmap.md`](docs/wallet-management-roadmap.md). That
roadmap covers seed/xpub gap-limit discovery, old-wallet classification, L1/L2
holdings, ERC-20 transfer-log token discovery, the first bounded ERC-721 NFT
discovery slice, bounded ERC-1155 transfer discovery, DeFi positions,
airdrops/rewards, ERC-20 allowance probing, bounded NFT operator-approval
probing, and reviewable consolidation planning. Broader token registries, full
ERC-1155 batch coverage, NFT metadata/spam classification, DeFi adapters,
Permit2 discovery, spender registries, and revoke execution remain roadmap
work. The
product strategy and market comparison are captured in
[`docs/wallet-competitive-landscape.md`](docs/wallet-competitive-landscape.md).
This is not another round of ad hoc transport or route growth and not a shift
toward internet deployment.

## License

Licensed under either of:

- [LICENSE-APACHE-2.0](LICENSE-APACHE-2.0)
- [LICENSE-MIT](LICENSE-MIT)
