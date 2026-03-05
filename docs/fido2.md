# FIDO2 Hardware Key Integration

## Overview

Sigillum uses FIDO2 security keys to protect the vault master key via Shamir's Secret Sharing. This eliminates master passwords entirely — the vault unlocks when you physically tap a hardware key.

## Supported Hardware

Any FIDO2-compliant key with the `hmac-secret` extension:

- **YubiKey 5 series** (NFC, USB-A, USB-C)
- **Google Titan Security Key**
- **SoloKey v2**
- **Feitian BioPass**
- **Nitrokey FIDO2**
- **Trezor Model T** (FIDO2 mode)

The key must support CTAP2 with the `hmac-secret` extension. Most modern FIDO2 keys do.

## How It Works

### Key Registration

When you register a FIDO2 key, Sigillum:

1. Generates a new FIDO2 credential on the key (resident or non-resident)
2. Retrieves the `hmac-secret` output for a fixed salt
3. Uses that output as an encryption key for a Shamir shard
4. Stores the encrypted shard + credential ID in `titan_keys.json`

```
┌──────────┐     ┌─────────────┐     ┌─────────────────┐
│ FIDO2 Key│────►│ hmac-secret │────►│ Encrypt shard   │
│   tap    │     │   output    │     │ with hmac output │
└──────────┘     └─────────────┘     └────────┬────────┘
                                              │
                                              ▼
                                     titan_keys.json
                                     (credential ID +
                                      encrypted shard)
```

### Master Key Splitting

The master key is split using Shamir's Secret Sharing into N shares with a threshold of M:

```
Master Key [u8; 32]
       │
       ▼
   Shamir split
   (M-of-N threshold)
       │
       ├── Shard 1 ──► encrypted with Key A's hmac-secret
       ├── Shard 2 ──► encrypted with Key B's hmac-secret
       └── Shard 3 ──► encrypted with Key C's hmac-secret
```

Default: 2-of-3 (register 3 keys, any 2 can unlock).

### Vault Unlock

1. User taps Key A → Sigillum decrypts Shard 1
2. User taps Key B → Sigillum decrypts Shard 2
3. Shamir reconstruction: Shard 1 + Shard 2 → Master Key
4. `vault.load_master_key(master_key)` → vault is unlocked

```
Tap Key A ──► hmac-secret output ──► decrypt Shard 1
Tap Key B ──► hmac-secret output ──► decrypt Shard 2
                                          │
                                          ▼
                                    Shamir reconstruct
                                          │
                                          ▼
                                    Master Key [u8; 32]
                                          │
                                          ▼
                                    load_master_key()
```

## Setup Guide

### Register Your First Key

```bash
# Register a FIDO2 key with a label
sigillum fido2 register --label "Primary-YubiKey"

# When prompted, tap your key
# The vault master key is generated and split automatically
```

### Register Additional Keys

```bash
# Add more keys to increase redundancy
sigillum fido2 register --label "Backup-Titan"
sigillum fido2 register --label "Emergency-Solo"

# Set quorum (how many keys needed to unlock)
sigillum fido2 set-quorum 2
```

### Unlock the Vault

```bash
sigillum unlock

# Output:
# Quorum: 2 of 3 keys required
# Tap key 1 of 2... [tap]
# ✓ Primary-YubiKey
# Tap key 2 of 2... [tap]
# ✓ Backup-Titan
# Vault unlocked.
```

### Lock the Vault

```bash
sigillum lock
# Master key zeroized from memory.
```

### List Registered Keys

```bash
sigillum fido2 list

# Registered FIDO2 Keys:
#   - Primary-YubiKey  (ID: 71dd6181...) [2026-02-28]
#   - Backup-Titan     (ID: 5e3c562f...) [2026-02-28]
#   - Emergency-Solo   (ID: a39f2b1c...) [2026-03-01]
#
# Quorum: 2 of 3
```

### Remove a Key

```bash
sigillum fido2 remove --label "Emergency-Solo"
# Warning: Removing this key reduces your total from 3 to 2.
# With quorum 2, you will have no redundancy.
# Proceed? [y/N]
```

## Web UI Integration

In daemon mode, FIDO2 unlock works through the browser via WebAuthn:

1. Open `http://localhost:9743`
2. Click "Unlock Vault"
3. Browser prompts for security key (WebAuthn `navigator.credentials.get()`)
4. Tap key → daemon receives assertion → decrypts shard
5. Repeat for quorum
6. Vault unlocked for all connected clients

The daemon proxies the FIDO2 protocol: the browser handles USB communication via WebAuthn, and the daemon handles shard management.

## Quorum Strategies

| Strategy | Keys | Quorum | Use Case |
|----------|------|--------|----------|
| Single key | 1 | 1 | Personal workstation, low-value secrets |
| Redundant pair | 2 | 1 | Convenience with backup |
| Two-of-three | 3 | 2 | Standard security (recommended) |
| Three-of-five | 5 | 3 | High-value infrastructure |
| Geographic split | 3+ | 2+ | Keys in different physical locations |

## Recovery

### Lost a key (below quorum)

If you still have enough keys to meet quorum:

```bash
# Unlock with remaining keys
sigillum unlock

# Re-register new keys
sigillum fido2 register --label "Replacement-Key"
```

The master key is re-split with the new set of keys.

### Lost too many keys (cannot meet quorum)

If you have a passphrase backup:

```bash
sigillum restore --input backup.sigillum
# Enter passphrase: ********
# Vault restored. Register new FIDO2 keys.
```

If you have no backup and cannot meet quorum, **the vault is permanently locked**. This is by design — there is no backdoor.

## Passphrase Fallback

For environments where hardware keys aren't available (remote SSH, VMs):

```bash
# Set up passphrase as additional unlock method
sigillum passphrase set

# Unlock with passphrase instead of hardware key
sigillum unlock --passphrase
# Enter passphrase: ********
# Deriving key (Argon2id, 64MB, 3 iterations)...
# Vault unlocked.
```

The passphrase is processed through Argon2id (64MB memory, 3 iterations, 1 thread) to derive a 256-bit key. This key decrypts a separate Shamir shard stored for passphrase-based unlock.

## Security Considerations

- **Physical presence**: FIDO2 keys require physical tap. Remote attackers cannot unlock.
- **Phishing resistance**: FIDO2 credentials are origin-bound. A phishing site cannot request your vault credential.
- **Key compromise**: A single stolen key is useless if quorum > 1. The attacker needs M keys.
- **hmac-secret determinism**: The hmac-secret output is deterministic for a given credential + salt. This means the same key always produces the same shard decryption key — no state to synchronize.
- **No key extraction**: FIDO2 private keys cannot be exported from the hardware. Cloning a key requires physical access to the silicon.
