# Architecture

## Overview

Sigillum is a layered system. Each layer has a single responsibility and communicates through well-defined trait boundaries.

```
┌─────────────────────────────────────────────────────────┐
│                    Your Application                      │
│                                                         │
│  Uses: &dyn SecretStore                                 │
│  Doesn't know: where secrets live, how they're encrypted │
└──────────────────────────┬──────────────────────────────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
              ▼            ▼            ▼
         ┌─────────┐ ┌──────────┐ ┌──────────┐
         │FileVault│ │RemoteVault│ │ YourImpl │
         │ (local) │ │ (daemon) │ │ (custom) │
         └────┬────┘ └─────┬────┘ └──────────┘
              │            │
              ▼            ▼
         ~/.sigillum/   HTTP/Unix Socket
         (files)        to sigillum-daemon
```

## Crate Dependency Graph

```
sigillum-core (foundation)
    │
    ├── sigillum (meta-crate, re-exports core)
    │
    ├── sigillum-fido2 (hardware key unlock)
    │
    ├── sigillum-client (remote vault SDK)
    │       └── reqwest, tokio
    │
    ├── sigillum-daemon (HTTP server + web UI)
    │       └── axum, tower-http, tokio
    │
    ├── sigillum-cli (terminal interface)
    │
    ├── sigillum-sdk (embeddable SDK)
    │
    └── sigillum-server (server library)
```

All arrows point downward. No circular dependencies. `sigillum-core` is the only crate that every other crate depends on.

## Core Traits

### SecretStore

The primary interface all consumers use. Object-safe and thread-safe.

```rust
pub trait SecretStore: Send + Sync {
    fn get_api_key(&self, key: &str) -> Option<SecretString>;
    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_api_key(&self, key: &str) -> Result<(), VaultError>;
    fn list_api_keys(&self) -> Vec<String>;

    fn get_secret(&self, key: &str) -> Option<SecretString>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_secret(&self, key: &str) -> Result<(), VaultError>;
    fn list_secrets(&self) -> Vec<String>;

    fn has_key(&self, key: &str) -> bool;
    fn is_unlocked(&self) -> bool;
}
```

**Design decisions:**
- `Option<SecretString>` for reads: missing keys are not errors, they're absence.
- `Result<(), VaultError>` for writes: mutations can fail (locked, IO, encryption).
- `Vec<String>` for lists: key names are not secret; values are never returned in bulk.
- No generic methods: the trait must be object-safe for `dyn SecretStore`.

### VaultLifecycle

Separated from `SecretStore` because 90% of consumers only need read/write. Only the unlock manager (CLI, daemon, FIDO2 module) needs lifecycle control.

```rust
pub trait VaultLifecycle: SecretStore {
    fn load_master_key(&self, key: [u8; 32]);
    fn zeroize_master_key(&self);
    fn initialize(&self, master_key: &[u8; 32]) -> Result<(), VaultError>;
}
```

This separation means you can hand application code `&dyn SecretStore` and it physically cannot call `zeroize_master_key()`.

## Two-Tier Secret System

### Tier 1: API Keys (Plaintext)

- Stored in `api_keys.json` (JSON file, `0o600` permissions)
- No encryption, no unlock required
- **Use case**: CI pipelines, headless servers, automation

### Tier 2: Secrets (Encrypted)

- Stored in `vault.enc` (AES-256-GCM ciphertext)
- Requires master key in memory to read/write
- Master key loaded via FIDO2 hardware tap or passphrase
- **Use case**: Database passwords, private keys, signing secrets

### Why two tiers?

A vault that requires hardware unlock for every operation is unusable in CI/CD. A vault that stores everything in plaintext isn't a vault. The two-tier model lets you choose per-secret.

## File Layout

```
~/.sigillum/                  (configurable base_dir)
├── api_keys.json             Tier 1 store
├── vault.enc                 Tier 2 store (encrypted)
├── vault_index.json          Key name index (best-effort cache)
└── titan_keys.json           FIDO2 credentials + encrypted shards
```

### vault.enc Format

```
┌──────────────┬──────────────┬──────────────────────────┐
│ Nonce (12B)  │ Ciphertext   │ Auth Tag (16B)           │
└──────────────┴──────────────┴──────────────────────────┘
```

- Nonce: 12 bytes from `OsRng` (fresh per write)
- Ciphertext: `serde_json::to_string(HashMap<String, String>)` encrypted with AES-256-GCM
- Auth tag: 16-byte GCM authentication tag (integrity + authenticity)

## Encryption Flow

### Encrypt (set_secret)

```
plaintext map ──► serde_json::to_vec()
                       │
                       ▼
               ┌───────────────┐
               │  AES-256-GCM  │◄── master_key [u8; 32]
               │   encrypt()   │◄── random nonce [u8; 12]
               └───────┬───────┘
                       │
                       ▼
               nonce || ciphertext || tag
                       │
                       ▼
               write to vault.enc (atomic)
```

### Decrypt (get_secret)

```
read vault.enc
       │
       ▼
parse nonce (first 12 bytes)
       │
       ▼
┌───────────────┐
│  AES-256-GCM  │◄── master_key [u8; 32]
│   decrypt()   │◄── nonce
└───────┬───────┘
       │
       ▼
serde_json::from_slice() ──► HashMap<String, String>
       │
       ▼
lookup key ──► SecretString
```

## Master Key Lifecycle

```
                    SEALED
                      │
                      │ initialize() — first-time setup
                      ▼
                    LOCKED
                      │
                      │ load_master_key([u8; 32])
                      │ (from FIDO2 Shamir, passphrase Argon2id, or direct)
                      ▼
                   UNLOCKED
                      │
                      │ All Tier 2 operations succeed
                      │
                      │ zeroize_master_key()
                      ▼
                    LOCKED
```

The master key is held in:
```rust
static MASTER_KEY: Mutex<Option<Zeroizing<[u8; 32]>>>
```

- `None` = locked. All Tier 2 reads return `None`, writes return `Err(VaultError::Locked)`.
- `Some(key)` = unlocked. `Zeroizing` overwrites the key bytes when dropped.
- The key is accessed via closure (`with_master_key(|k| ...)`) so it never escapes the Mutex guard.

## Daemon Architecture

The daemon wraps `FileVault` in an HTTP server. It is the **key custodian** — the master key lives in this single process.

```
┌─────────────────────────────────────────────┐
│                sigillum-daemon               │
│                                             │
│  ┌─────────┐  ┌──────────┐  ┌───────────┐  │
│  │  Axum   │  │ FileVault│  │  Audit    │  │
│  │ Router  │──│ (core)   │──│  Logger   │  │
│  └────┬────┘  └──────────┘  └───────────┘  │
│       │                                     │
│  ┌────┴────────────────────────────────┐    │
│  │            Routes                   │    │
│  │  /api/vault/*    Secret CRUD        │    │
│  │  /api/fido/*     FIDO2 unlock       │    │
│  │  /api/auth/*     Session management │    │
│  │  /api/backup/*   Export/import      │    │
│  │  /api/status     Health + metrics   │    │
│  │  /api/stream     SSE event stream   │    │
│  │  /*              Static web UI      │    │
│  └─────────────────────────────────────┘    │
│                                             │
│  ┌─────────────────────────────────────┐    │
│  │         Middleware Stack             │    │
│  │  CORS · Security Headers · Auth     │    │
│  │  Audit Logging · Rate Limiting      │    │
│  └─────────────────────────────────────┘    │
└─────────────────────────────────────────────┘
         │                    │
    Unix Socket          TCP/TLS
    (local mode)        (network mode)
```

### Transport Modes

| Mode | Binding | Auth | Use Case |
|------|---------|------|----------|
| Local | Unix socket (`/run/sigillum.sock`) | File permissions (uid/gid) | Single machine, highest security |
| Network | TCP with TLS | mTLS client certs or HMAC bearer tokens | Multi-machine, remote clients |

### Session Flow

1. Client requests challenge: `GET /api/auth/challenge`
2. Client proves identity (wallet signature, FIDO2, or shared secret)
3. Server issues HMAC-signed session token
4. Client includes token in subsequent requests
5. Middleware validates token on every protected route

## Client SDK

`sigillum-client` implements `SecretStore` over HTTP. From the consumer's perspective, it's identical to `FileVault`.

```rust
// Local mode
let vault: Box<dyn SecretStore> = Box::new(FileVault::new(config));

// Remote mode — same trait, same methods
let vault: Box<dyn SecretStore> = Box::new(RemoteVault::connect(url)?);

// Consumer code doesn't change
let secret = vault.get_secret("db_password");
```

### Connection Strategy

```
RemoteVault::connect()
       │
       ├── Try Unix socket (/run/sigillum.sock)
       │       └── Success? Use it (fastest, most secure)
       │
       ├── Try localhost:9743
       │       └── Success? Use it (local daemon)
       │
       └── Try configured URL
               └── Success? Use it (remote daemon)
               └── Failure? Return error
```

## Extending Sigillum

### Custom Backend

Implement `SecretStore` to create a vault backed by anything:

```rust
use sigillum_core::{SecretStore, VaultError};
use secrecy::SecretString;

struct PostgresVault { /* ... */ }

impl SecretStore for PostgresVault {
    fn get_secret(&self, key: &str) -> Option<SecretString> {
        // SELECT value FROM secrets WHERE key = $1
    }
    // ... remaining methods
}
```

### Custom Unlock Mechanism

Implement `VaultLifecycle` to add new unlock methods:

```rust
use sigillum_core::VaultLifecycle;

struct BiometricUnlock { /* ... */ }

impl BiometricUnlock {
    fn unlock(&self, vault: &dyn VaultLifecycle) {
        let key = self.scan_fingerprint_derive_key();
        vault.load_master_key(key);
    }
}
```

## Design Principles

1. **Trait-first**: Consumers depend on traits, never concrete types. Swapping `FileVault` for `RemoteVault` requires zero code changes.
2. **Secrets are opaque**: `SecretString` has no `Display` or `Debug`. You must call `.expose_secret()` explicitly — no accidental leaks.
3. **Fail closed**: If the vault is locked, Tier 2 reads return `None` (not an error, not a default value). The caller decides what absence means.
4. **Audit by default**: The daemon logs every operation. You can't forget to add logging — it's structural.
5. **Hardware over passwords**: FIDO2 keys are phishing-resistant, theft-resistant (quorum), and user-friendly (tap, don't type).
