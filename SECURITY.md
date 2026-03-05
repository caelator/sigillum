# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Email security reports to: **security@caelator.com**

Include:
- Description of the vulnerability
- Steps to reproduce
- Impact assessment
- Suggested fix (if any)

You will receive an acknowledgment within 48 hours. We aim to release a patch within 7 days of confirmation.

## Security Design

### Threat Model

Sigillum protects against:

| Threat | Mitigation |
|--------|------------|
| Disk compromise (stolen laptop, server breach) | Tier 2 secrets encrypted with AES-256-GCM. Master key not on disk. |
| Memory dump / core dump | Master key held in `Zeroizing<[u8; 32]>` — overwritten on drop. `SecretString` prevents heap scanning for values. |
| Phishing for master password | No master password. FIDO2 hardware keys are phishing-resistant by design (origin-bound). |
| Single key compromise | Shamir's Secret Sharing requires M-of-N keys. One stolen key is useless alone. |
| Unauthorized API access (daemon mode) | HMAC session tokens, mTLS client certs, or Unix socket permissions. |
| Accidental secret logging | `SecretString` has no `Display` or `Debug` impl. Secrets cannot be printed without explicit `expose_secret()`. |
| Timing attacks on key comparison | Constant-time comparison via `subtle` crate (transitive dependency of RustCrypto). |
| Replay attacks on backup files | Each backup includes a unique timestamp and random nonce. |

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
- `hmac` — HMAC-SHA256
- `zeroize` — Secure memory zeroing
- `secrecy` — Secret-wrapping types

No OpenSSL. No C bindings for crypto. Pure Rust.

### File Permissions

| File | Permissions | Contents |
|------|-------------|----------|
| `api_keys.json` | `0o600` | Tier 1 plaintext keys |
| `vault.enc` | `0o600` | Tier 2 AES-256-GCM ciphertext |
| `titan_keys.json` | `0o600` | FIDO2 credential IDs + encrypted Shamir shards |
| `vault_index.json` | `0o600` | Key name index (no secret values) |

All files are created with restrictive permissions. The vault never writes secrets to temporary files.
