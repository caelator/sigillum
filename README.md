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

Sigillum 1.0 is the local-first wallet-management workstation. The
wallet-management product in phases 1-9 of the
[`wallet-management roadmap`](docs/wallet-management-roadmap.md) is shipped for
EVM networks, except swap execution, which is deferred per D-13.
Consolidation-plan execution ships as a policy-gated, fail-closed opt-in that
defaults off. The local-first, single-machine, not-internet-facing boundary is
unchanged. Non-EVM chains (roadmap phase 10) and fiat/NFT valuation remain
post-1.0.

## Current Scope

Implemented and working in this repository:

- `sigillum-core`: core traits, errors, file-backed vault, Argon2 helpers, wrapped-key helpers
- `sigillum-api`: shared daemon request/response contract used by the daemon and async client
- `sigillum-fido2`: FIDO2 registration/unlock support and Shamir-based shard handling
- `sigillum-daemon`: local Axum daemon with an embedded operator console (sidebar shell with a dark design system, wallet manager for creating wallets with server-generated one-time mnemonics and importing seed/xpub/watch wallets), compartment switching, passphrase/FIDO2 unlock, snapshot import/export, local audit feed, transit-style crypto endpoints, Ethereum stealth wallet helpers, provider-backed deposit monitoring, sweep orchestration, a treasury console (cross-wallet value/risk/plan roll-up), treasury policy guardrails (destination allowlist and native value caps enforced at plan generation and approval), and locally derived purpose-labeled receive-address allocations with rotation
- `sigillum-client`: async client for the local daemon API, including session handling and snapshots
- `sigillum-cli`: setup flows, local management commands, snapshot commands,
  daemon launcher, and daemon-backed JSON operator commands
- `sigillum-gateway`: local-sidecar payment preview surface with project API keys, payment intent creation, and webhook delivery
- `sigillum-sdk`: integration surface that combines core types with the async daemon client
- `sigillum-server`: thin facade over the daemon crate for server-side embedding
- `sigillum`: meta-crate that re-exports the file-backed core

**1.0 scope boundary:** Sigillum 1.0 targets EVM networks. Non-EVM chains, swap
execution, fiat/NFT valuation, and remote or hosted operation are explicitly out
of scope.

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

These crates are not published to crates.io and must be consumed as git dependencies or from a path/source checkout.

```toml
[dependencies]
sigillum = { git = "https://github.com/caelator/sigillum.git" }
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
under `sigillum api`, including session unlock, compartment switching,
compartment listing, provider and wallet profile management, deposit
management, queue inspection, transit encryption helpers, read-only EVM RPC
queries (no broadcast), wallet xpub/stealth export and derive helpers (no
sign/send), and maintenance runs. These commands default to `http://127.0.0.1:9743`, accept
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

### Desktop app

`sigillum-desktop` is a Tauri v2 shell that runs the Sigillum daemon in-process
and shows the existing web console in a native window. Launch it during
development with `cargo run -p sigillum-desktop`. The app uses the same data
directory as the CLI and daemon (`~/.sigillum` by default, overridable with
`SIGILLUM_BASE_DIR`), opens the daemon on a fresh loopback port on each launch,
keeps a single instance focused, and adds a tray with live lock state plus
"Lock now". Closing the window locks and hides to the tray; quitting locks
first so loaded master keys are cleared before exit.

To build an installable macOS bundle:

```bash
cargo install tauri-cli --version '^2' --locked
cd crates/sigillum-desktop
cargo tauri build
```

The build writes `target/release/bundle/macos/Sigillum.app` and
`target/release/bundle/dmg/Sigillum_<version>_<arch>.dmg` under the workspace
root. Builds without Apple signing credentials are ad-hoc signed by design;
install, Gatekeeper, full signing, notarization, and troubleshooting details
live in `docs/deployment.md`.

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
  resumable bounded ERC-20 transfer-log token discovery, optional
  multi-account Ethereum seed receive-branch discovery, operator-bounded
  ERC-20 allowance probes, operator-bounded Permit2 allowance probes,
  resumable bounded ERC-721 transfer-log discovery with `ownerOf`
  confirmation, resumable bounded ERC-1155 transfer discovery with
  `balanceOf` confirmation, and operator-bounded NFT `isApprovedForAll`
  approval probes across one provider profile or every configured EVM provider
- a local operator-managed risk catalog for spender/operator labels and
  approval-risk overrides
- reviewable consolidation-plan revoke steps for discovered ERC-20, Permit2,
  and NFT operator approvals, with signer and simulation gates before execution
- persistent queue jobs for direct sends and sweep jobs
- atomic sidecar-backed persistence for profile, deposit, and queue state with
  automatic restore/quarantine behavior
- a maintenance cycle that refreshes deposits, auto-enqueues sweeps, and drains queued work

This means the current boundary is no longer “sign only.” Sigillum can now keep
provider credentials internal, monitor deposit balances, sign locally, and
optionally broadcast without exposing private wallet material to upstream web
services. It also offers controlled, policy-gated, fail-closed queue execution
of consolidation plans, default off, which supersedes the earlier export-only
handoff while keeping all private wallet material local.

## Privacy Model — Scope and Limitations

Sigillum is designed for a **solo operator** who receives funds from many parties
and does not want those payers tied together. Its linkage protection is built for
**public on-chain analysis and the payers themselves**. It is explicitly *not*
hardened against a well-resourced adversary correlating timing, amounts, and IPs,
nor against malware already running on your machine.

**What linkage protection covers.** When consolidating (HD plan) or sweeping
stealth deposits, Sigillum detects when funds belonging to *different* payers
would land at the **same destination address** — the dominant
"common-recipient" clustering signal on Ethereum's account model. It surfaces
this as a warning, supports routing each payer to a **distinct** destination, and
— when the `block_cross_party_linkage` treasury policy is enabled (a fail-closed
opt-in offered during onboarding) — **blocks** any consolidation step or stealth
sweep that would link payers. This is enforced at plan generation, at approval,
at stealth-sweep enqueue, and — proven at parity (W7.5) — at plan-step
execution enqueue: the same single-hop destination-axis claim holds all the
way to the point a step becomes a signable queue job, with the same scope and
the same limitations below (no amount/timing or multi-hop claims).

**What it does NOT cover** (operator discipline required):

- **Manual gas funding.** Sigillum-generated gas top-ups (policy-gated
  `fund_gas` plan steps, off by default behind `allow_gas_topups`) run the same
  linkage analysis as sweeps: one sponsor funding different payers' addresses
  always warns and is hard-blocked when `block_cross_party_linkage` is on.
  Funding gas manually from a shared/known address still links payers and
  remains operator discipline - fund per-payer gas from per-payer sources.
- **Amount and timing correlation.** Out of scope for this threat model.
- **Downstream re-merging.** Sigillum checks one hop. If you later move
  per-payer destinations into one address, they re-link. Keep them separate.
- **Multi-hop flows** through intermediaries.

**Other privacy costs, surfaced in the UI.** RPC provider calls (balance refresh,
inventory scans) reveal the queried addresses and your IP to the configured
endpoint — prefer your own node or a dedicated/partitioned endpoint. The local
daemon listens on a loopback TCP port; any local process can reach the API, gated
by the per-session bearer token (held only in the desktop webview) and an unlock
throttle.

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
- wallet inventory scans across one provider or all configured EVM chains, with resumable ERC-20, ERC-721, and ERC-1155 transfer-log cursors plus ERC-20 allowance, Permit2 allowance, and NFT operator-approval discovery controls
- local risk catalog management for approval spender/operator labels
- single-chain dry-run consolidation plans that surface approval revokes alongside sweeps
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
sigillum-core = { git = "https://github.com/caelator/sigillum.git", default-features = false }
sigillum-fido2 = { git = "https://github.com/caelator/sigillum.git", default-features = false }
```

## Development

The application release contract is a clean checkout with the committed
`Cargo.lock` and the pinned Rust toolchain in `rust-toolchain.toml`.
Current readiness evidence and caveats are tracked in
[`docs/production-readiness-audit.md`](docs/production-readiness-audit.md).

The stable and unstable surfaces for 1.0, the SemVer policy, and the recorded execution residual risk are documented in [`docs/stability.md`](docs/stability.md).

Full release gate:

```bash
./scripts/check-release.sh
```

The release gate runs Cargo metadata, architecture guardrails, daemon UI
install/typecheck/tests/build, generated UI asset freshness, Rust
fmt/check/test/clippy, a real local daemon runtime smoke test with vault
write/read canaries, `cargo audit`, `cargo deny check`, and whitespace checks.
The commands below are useful for targeted local iteration.

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

Sigillum 1.0 ships the local-first wallet-management workstation for EVM
networks, with multi-chain discovery, inventory, and risk assessment; a chain
registry; consolidation planning and policy-gated, fail-closed execution that
defaults off; DeFi exit adapters; and gas top-ups plus hot-wallet overflow/refill
treasury automation. The completed EVM scope is documented in
[`docs/wallet-management-roadmap.md`](docs/wallet-management-roadmap.md).

Wallet management is complete for EVM except swap execution, which is deferred
per D-13. Non-EVM chains (roadmap phase 10), swap execution (D-13), and fiat/NFT
valuation (D-16) remain post-1.0. The product strategy and market comparison are
captured in
[`docs/wallet-competitive-landscape.md`](docs/wallet-competitive-landscape.md).
This is not another round of ad hoc transport or route growth and not a shift
toward internet deployment.

## License

Licensed under either of:

- [LICENSE-APACHE-2.0](LICENSE-APACHE-2.0)
- [LICENSE-MIT](LICENSE-MIT)
