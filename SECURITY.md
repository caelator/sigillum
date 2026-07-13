# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Unreleased source snapshots | No |
| v1.0.0 and later | Not yet published |

No released Sigillum version is currently supported. Security support begins
only after `v1.0.0` is published as a final release; release-candidate builds,
repository snapshots, and local builds are not supported releases.

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Use [GitHub private vulnerability reporting](https://github.com/caelator/sigillum/security/advisories/new)
when it is available. If that path is unavailable, email
**security@caelator.com**.

Include:

- the affected commit or release identifier;
- a concise description and safe reproduction using synthetic data;
- the expected impact and preconditions;
- suggested remediation, if known.

Do not send production seed phrases, private keys, API credentials, wallet data,
or full Sigillum data directories. Encrypt sensitive supporting material only
after agreeing on a safe transfer method with the maintainer.

The project will acknowledge and triage reports as maintainer availability
allows. No fixed response or remediation SLA is offered while Sigillum has no
supported stable release.

## Security Design

Sigillum's current security story is explicitly **local-first and single-host**.
The daemon and optional gateway sidecar are intended to remain inside one
machine's trust boundary; this document does not claim a hardened
internet-facing or multi-tenant deployment model because that is outside the
project’s intended scope. Gateway payment creation is disabled by default and
remains an experimental preview: balance observations are not finality proof,
so the gateway exposes only the latest balance observation. With the preview
disabled, no payment poller or payment-webhook retry loop is started.

### Threat Model

Sigillum protects against:

| Threat | Mitigation |
|--------|------------|
| Disk compromise (stolen laptop, server breach) | Tier 2 secrets are encrypted with AES-256-GCM. Unwrapped compartment keys are kept in memory only and are not stored on disk. |
| Memory dump / core dump | Master key held in `Zeroizing<[u8; 32]>` — overwritten on drop. `SecretString` prevents heap scanning for values. |
| Phishing for unlock material | FIDO2 unlock remains phishing-resistant at the credential layer. Passphrase unlock exists as a local fallback path and should be treated like any other secret. |
| Single key compromise | Shamir's Secret Sharing requires M-of-N keys. One stolen key is useless alone. |
| Unauthorized API access (daemon mode) | Bearer session tokens over local HTTP, with the gateway remaining a local-sidecar preview surface in this phase. |
| Accidental secret logging | `SecretString` has no `Display` or `Debug` impl. Secrets cannot be printed without explicit `expose_secret()`. |
| Timing attacks on key comparison | Constant-time comparison via `subtle` crate (transitive dependency of RustCrypto). |
| Replay attacks on backup files | Each backup includes a unique timestamp and random nonce. |
| Local state corruption | Versioned JSON state is atomically written, mirrored to `.bak`, restored from backup when safe, and fails closed when both live and backup are corrupt. |
| Panic while mutating synchronized state | Daemon security-state, `FileVault`, and gateway database locks abort if poisoned; the client clears its cached token; HID operations return a restart-required error. |

### What Sigillum does NOT protect against

- **Compromised process**: If an attacker has code execution within the Sigillum daemon process, they can read the master key from memory.
- **Hardware key theft with quorum**: If an attacker physically obtains M-of-N hardware keys, they can unlock the vault.
- **Root/admin on the host**: A root user can attach to the process and read memory.
- **Supply chain attacks**: Sigillum depends on RustCrypto crates. A compromised upstream dependency could undermine all guarantees.

### Cryptographic Choices

| Choice | Rationale |
|--------|-----------|
| AES-256-GCM over XChaCha20-Poly1305 | Hardware acceleration (AES-NI) on most x86/ARM. Both are AEAD; AES-256-GCM is NIST-approved. |
| Argon2id over bcrypt/scrypt | Memory-hard, resistant to both GPU and side-channel attacks. Recommended by OWASP. |
| 12-byte random nonce | Standard for AES-256-GCM. Fresh nonce per encryption via OsRng. Nonce reuse probability is negligible at vault-scale write volumes. |
| Shamir SSS over multi-sig | Shamir is information-theoretically secure. No threshold signature scheme needed — we're splitting a symmetric key, not signing. |
| FIDO2 hmac-secret over challenge-response | hmac-secret provides a deterministic secret derived from the credential, enabling offline shard decryption after initial tap. |

### Dependency Audit

All cryptographic dependencies are from the [RustCrypto](https://github.com/RustCrypto) project:

- `aes-gcm` — AES-256-GCM authenticated encryption
- `argon2` — Argon2id key derivation
- `sha2` — SHA-256/SHA-512
- `sha1` — SHA-1 only for standards-compatible TOTP HMACs
- `hmac` — HMAC-SHA256
- `zeroize` — Secure memory zeroing
- `secrecy` — Secret-wrapping types

No OpenSSL. No C bindings for crypto. Pure Rust.

### File Permissions

| File | Permissions | Contents |
|------|-------------|----------|
| `api_keys.json` | `0o600` | Tier 1 plaintext keys |
| `vault.enc` | `0o600` | Tier 2 AES-256-GCM ciphertext |
| `fido2_keys.json` | `0o600` | FIDO2 credential IDs + encrypted Shamir shards |
| `profiles.json` | `0o600` | Daemon operator profiles and wallet/provider bindings |
| `deposits.json` | `0o600` | Daemon deposit registry |
| `queue.json` | `0o600` | Daemon queue state |
| `audit.db` | `0o600` | Local SQLite audit database |

All files are created with restrictive permissions.

`queue.json` and its mirrored `.bak` may contain the exact signed transaction
bytes while a job is `prepared` or `submitted_unknown`. Those bytes cannot be
used to derive a private key, but any process that can read them can broadcast
the already-approved transaction. Queue API responses always redact the bytes,
and terminal or affirmatively broadcast states clear them from the live queue
document. Backup refresh is best-effort: a failed backup write can leave older
signed bytes in `queue.json.bak` until a later successful load/save refreshes
it. Treat read access to the Sigillum data directory and its backups as
transaction-execution authority: use an owner-only account, full-disk
encryption, and protected backup retention. Host compromise is outside
Sigillum's threat model.

Durable atomic replace operations may use same-directory temporary files before
rename. Those files are created with restrictive permissions, `fsync`'d before
rename, and the parent directory is synchronized when the platform supports it.

Run `sigillum doctor` on a target host to check local directory permissions,
daemon reachability, session-token state, and audit database readability.
