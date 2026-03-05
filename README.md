<p align="center">
  <h1 align="center">Sigillum</h1>
  <p align="center">
    <strong>Hardware-backed secret management for the paranoid.</strong>
  </p>
  <p align="center">
    <a href="https://crates.io/crates/sigillum"><img src="https://img.shields.io/crates/v/sigillum.svg" alt="crates.io"></a>
    <a href="https://docs.rs/sigillum"><img src="https://docs.rs/sigillum/badge.svg" alt="docs.rs"></a>
    <a href="https://github.com/caelator/sigillum/actions"><img src="https://github.com/caelator/sigillum/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg" alt="License"></a>
  </p>
</p>

---

Sigillum is a two-tier encrypted vault with FIDO2 hardware key unlock, a daemon mode with web UI, and a client SDK for remote access. It manages secrets the way they should be managed: encrypted at rest, unlocked by hardware you physically hold, and accessible only through audited channels.

## Why Sigillum?

Most secret managers fall into two camps: cloud-hosted services you don't control, or local tools that only work for one app. Sigillum is neither.

- **Hardware-first**: Master key is reconstructed from FIDO2 security keys via Shamir's Secret Sharing. No master password to phish.
- **Two-tier design**: API keys (Tier 1) are stored in plaintext JSON for CI/headless use. Secrets (Tier 2) are AES-256-GCM encrypted and require hardware unlock.
- **Daemon mode**: Unlock once, serve many. The daemon holds the master key in memory so connected applications never touch key material.
- **Project-agnostic**: Sigillum doesn't know or care what you're building. It stores secrets and gives them back through a clean trait interface.

## Architecture

```
sigillum/
├── sigillum-core      Core traits (SecretStore, VaultLifecycle) + file-backed vault
├── sigillum-daemon    Axum HTTP server with web UI, SSE, audit logging
├── sigillum-client    Remote vault SDK (implements SecretStore over HTTP)
├── sigillum-fido2     FIDO2 hardware key integration + Shamir SSS
├── sigillum-cli       Command-line interface
├── sigillum-sdk       Embeddable SDK for third-party integration
└── sigillum-server    Server library for custom deployments
```

### How it works

```
                    ┌─────────────┐
                    │  Your App   │
                    │  (any lang) │
                    └──────┬──────┘
                           │ SecretStore trait
                           │ (local or remote)
                    ┌──────▼──────┐
                    │   Sigillum  │
                    │   Daemon    │◄──── Web UI (browser)
                    │   (Axum)   │
                    └──────┬──────┘
                           │ master key in memory
                    ┌──────▼──────┐
                    │  FileVault  │
                    │ AES-256-GCM │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        api_keys.json  vault.enc   titan_keys.json
        (Tier 1)       (Tier 2)    (FIDO2 shards)
```

**Tier 1** (plaintext): API keys that don't need hardware unlock. Stored in `api_keys.json` with `0o600` permissions. Suitable for CI, automation, and headless environments.

**Tier 2** (encrypted): Secrets protected by AES-256-GCM. The encryption key is derived from your FIDO2 hardware keys via Shamir's Secret Sharing. You physically tap a key to unlock.

## Quick Start

### As a library (embedded)

```toml
[dependencies]
sigillum = "0.1"
```

```rust
use sigillum::{FileVault, VaultConfig, SecretStore};

// Create a vault with default config (~/.sigillum/)
let vault = FileVault::new(VaultConfig::default());

// Tier 1: no unlock needed
vault.set_api_key("github_token", "ghp_...")?;
let token = vault.get_api_key("github_token");

// Tier 2: requires unlock first
// (see FIDO2 section below)
vault.set_secret("database_password", "hunter2")?;
let password = vault.get_secret("database_password");
```

### As a daemon (remote)

```bash
# Start the daemon
sigillum daemon --port 9743

# Unlock via web UI at http://localhost:9743
# Or via CLI:
sigillum unlock
```

```rust
use sigillum_client::RemoteVault;
use sigillum_core::SecretStore;

// Connect to running daemon
let vault = RemoteVault::connect("http://localhost:9743")?;

// Same trait, same methods — transport is invisible
let token = vault.get_api_key("github_token");
```

### Custom config

```rust
use sigillum::{FileVault, VaultConfig};
use std::path::PathBuf;

let vault = FileVault::new(VaultConfig {
    base_dir: PathBuf::from("/etc/myapp/secrets"),
    tier1_file: "api_keys.json".into(),
    tier2_file: "vault.enc".into(),
});
```

## Traits

Sigillum's design is trait-based. Consumers depend on the trait, not the implementation.

### `SecretStore`

The primary interface. All vault operations go through this.

```rust
pub trait SecretStore: Send + Sync {
    // Tier 1 (plaintext, no unlock required)
    fn get_api_key(&self, key: &str) -> Option<SecretString>;
    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_api_key(&self, key: &str) -> Result<(), VaultError>;
    fn list_api_keys(&self) -> Vec<String>;

    // Tier 2 (encrypted, requires unlock)
    fn get_secret(&self, key: &str) -> Option<SecretString>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_secret(&self, key: &str) -> Result<(), VaultError>;
    fn list_secrets(&self) -> Vec<String>;

    // Common
    fn has_key(&self, key: &str) -> bool;
    fn is_unlocked(&self) -> bool;
}
```

- Object-safe: usable as `&dyn SecretStore` or `Box<dyn SecretStore>`
- `Send + Sync`: safe to share across threads via `Arc`
- Returns `SecretString` (from the `secrecy` crate): values are zeroized on drop

### `VaultLifecycle`

Lifecycle management, separated from data access. Only the unlock manager (CLI, daemon, FIDO2 module) needs this.

```rust
pub trait VaultLifecycle: SecretStore {
    fn load_master_key(&self, key: [u8; 32]);
    fn zeroize_master_key(&self);
    fn initialize(&self, master_key: &[u8; 32]) -> Result<(), VaultError>;
}
```

## Cryptography

| Primitive | Purpose | Implementation |
|-----------|---------|----------------|
| AES-256-GCM | Tier 2 encryption at rest | `aes-gcm` 0.10 (RustCrypto) |
| Argon2id | Passphrase-to-key derivation | `argon2` 0.5 (64MB, 3 iterations) |
| FIDO2/CTAP2 | Hardware key authentication | `ctap-hid-fido2` (via `sigillum-fido2`) |
| Shamir SSS | Master key quorum splitting | `sharks` (M-of-N threshold) |
| Zeroizing | Automatic key material cleanup | `zeroize` 1.8 (overwrite on drop) |
| SecretString | Prevent accidental secret exposure | `secrecy` 0.8 (no Display/Debug) |
| Random nonce | Per-encryption unique IV | `rand` 0.8 (OsRng, 12-byte nonce) |

All cryptographic operations use audited [RustCrypto](https://github.com/RustCrypto) crates.

## FIDO2 Hardware Key Unlock

Sigillum supports FIDO2 security keys (YubiKey, Google Titan, SoloKey, etc.) for vault unlock via Shamir's Secret Sharing:

1. **Registration**: The master key is split into N shards. Each shard is encrypted with a FIDO2 key's `hmac-secret` extension output.
2. **Unlock**: Tap M-of-N registered keys. Each key decrypts its shard. Shards are recombined to reconstruct the master key.
3. **In-memory only**: The reconstructed key lives in a `Mutex<Option<Zeroizing<[u8; 32]>>>`. It never touches disk.

```
User taps FIDO2 key
       │
       ▼
Decrypt shard via hmac-secret
       │
       ▼
Repeat for M keys (quorum)
       │
       ▼
Shamir reconstruction → [u8; 32]
       │
       ▼
load_master_key() → Mutex holds key
       │
       ▼
All Tier 2 operations now succeed
       │
       ▼
zeroize_master_key() → key overwritten
```

## Daemon Web UI

The daemon (`sigillum-daemon`) serves an HTTP API and web interface:

**Dashboard**: Vault status, key counts, connected clients, last backup timestamp.

**Unlock**: Passphrase input or FIDO2 WebAuthn prompt in the browser.

**Secrets Browser**: List, add, edit, delete secrets across both tiers. Values hidden by default with explicit reveal.

**Backup/Restore**: Export encrypted snapshots (passphrase or FIDO2 dual-mode). Import with diff preview.

**Audit Log**: Every `get`, `set`, `delete` operation logged with client identity and timestamp.

## Backup & Restore

Encrypted vault snapshots with dual-mode protection:

```bash
# Export (passphrase-protected)
sigillum backup --output vault.sigillum

# Export (FIDO2-protected, requires tap)
sigillum backup --fido2 --output vault.sigillum

# Import with diff preview
sigillum restore --input vault.sigillum
```

Backup format: `MAGIC || VERSION || MODE || TIMESTAMP || [Envelopes] || [Encrypted Payload]`

## Crate Map

| Crate | Purpose | When to use |
|-------|---------|-------------|
| [`sigillum`](https://crates.io/crates/sigillum) | Meta-crate, re-exports core | Default dependency for most users |
| [`sigillum-core`](https://crates.io/crates/sigillum-core) | Traits + FileVault | Building a custom vault backend |
| [`sigillum-client`](https://crates.io/crates/sigillum-client) | Remote vault SDK | Connecting to a running daemon |
| [`sigillum-daemon`](https://crates.io/crates/sigillum-daemon) | HTTP server + web UI | Running Sigillum as a service |
| [`sigillum-fido2`](https://crates.io/crates/sigillum-fido2) | Hardware key + Shamir | Adding FIDO2 unlock to your deployment |
| [`sigillum-cli`](https://crates.io/crates/sigillum-cli) | Command-line interface | Managing vault from terminal |
| [`sigillum-sdk`](https://crates.io/crates/sigillum-sdk) | Embeddable SDK | Integrating vault into any application |
| [`sigillum-server`](https://crates.io/crates/sigillum-server) | Server library | Custom deployment architectures |

## Security Model

- **Secrets never leave `SecretString`**: The `secrecy` crate prevents accidental logging, serialization, or display of secret values.
- **Master key never leaves the process**: Held in `Mutex<Option<Zeroizing<[u8; 32]>>>`, automatically overwritten on drop.
- **No master password**: FIDO2 hardware keys eliminate the weakest link in most secret managers.
- **Quorum-based unlock**: Shamir's Secret Sharing means no single key is sufficient (configurable M-of-N).
- **Audit everything**: Every secret access is logged with client identity, timestamp, and operation type.
- **Tier separation**: API keys (Tier 1) are available without unlock for CI/automation. High-value secrets (Tier 2) require hardware.

See [SECURITY.md](SECURITY.md) for the full security policy and vulnerability reporting.

## Configuration

Default configuration directory: `~/.sigillum/`

```
~/.sigillum/
├── api_keys.json       Tier 1 keys (plaintext, 0o600)
├── vault.enc           Tier 2 secrets (AES-256-GCM)
├── vault_index.json    Best-effort key name index
└── titan_keys.json     FIDO2 credential IDs + encrypted shards
```

All paths are configurable via `VaultConfig`.

## Building

```bash
git clone https://github.com/caelator/sigillum.git
cd sigillum
cargo build --release
```

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `file-backend` | Yes | File-based vault (AES-256-GCM, Argon2id) |
| `fido2` | No | FIDO2 hardware key support (pulls USB HID deps) |

```toml
# Minimal (traits only, no file backend)
sigillum-core = { version = "0.1", default-features = false }

# With FIDO2
sigillum = { version = "0.1", features = ["fido2"] }
```

## Minimum Supported Rust Version

Sigillum requires **Rust 1.85** or later (Edition 2024).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE-2.0](LICENSE-APACHE-2.0))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
