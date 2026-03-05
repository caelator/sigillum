# Backup & Restore

## Overview

Sigillum supports encrypted vault snapshots that capture both Tier 1 and Tier 2 secrets in a single portable file. Backups use dual-mode protection: passphrase (for portable recovery) and FIDO2 (for seamless restore with same keys).

## Backup Format

```
┌───────────────────────────────────────────────────┐
│ MAGIC (8 bytes): "SIGILLUM"                        │
├───────────────────────────────────────────────────┤
│ VERSION (1 byte): 0x01                             │
├───────────────────────────────────────────────────┤
│ MODE (1 byte): 0x01=passphrase, 0x02=fido2, 0x03=dual │
├───────────────────────────────────────────────────┤
│ TIMESTAMP (8 bytes): Unix epoch, big-endian        │
├───────────────────────────────────────────────────┤
│ PASSPHRASE ENVELOPE (if mode & 0x01):              │
│   Salt (32 bytes)                                  │
│   Nonce (12 bytes)                                 │
│   Encrypted CEK (48 bytes: 32 key + 16 tag)        │
├───────────────────────────────────────────────────┤
│ FIDO2 ENVELOPE (if mode & 0x02):                   │
│   Nonce (12 bytes)                                 │
│   Encrypted CEK (48 bytes: 32 key + 16 tag)        │
├───────────────────────────────────────────────────┤
│ PAYLOAD:                                           │
│   Nonce (12 bytes)                                 │
│   Ciphertext (variable)                            │
│   Auth Tag (16 bytes)                              │
└───────────────────────────────────────────────────┘
```

The Content Encryption Key (CEK) is a random 256-bit key generated per backup. The payload is the vault snapshot encrypted with the CEK. The CEK itself is wrapped by the passphrase-derived key and/or the vault master key (FIDO2 mode).

## Creating Backups

### Passphrase-protected

```bash
sigillum backup --output vault.sigillum
# Enter passphrase: ********
# Confirm passphrase: ********
# Backup written to vault.sigillum (3.2 KB)
```

Anyone with the passphrase can restore. Suitable for offsite storage, disaster recovery.

### FIDO2-protected

```bash
sigillum backup --fido2 --output vault.sigillum
# Vault must be unlocked (master key in memory)
# Backup written to vault.sigillum (2.8 KB)
```

Restore requires the same FIDO2 keys that can unlock the vault. No passphrase needed.

### Dual-mode (recommended)

```bash
sigillum backup --dual --output vault.sigillum
# Enter passphrase: ********
# Confirm passphrase: ********
# Vault must be unlocked for FIDO2 envelope
# Backup written to vault.sigillum (3.5 KB)
```

Either passphrase OR FIDO2 keys can restore. Maximum flexibility.

## Restoring from Backup

### Preview changes first

```bash
sigillum restore --input vault.sigillum --dry-run
# Enter passphrase: ********
#
# Restore preview:
#   + github_token (Tier 1, new)
#   ~ openai (Tier 1, changed)
#   = anthropic (Tier 1, unchanged)
#   + db_password (Tier 2, new)
#   - old_secret (Tier 2, not in backup, will be kept)
#
# 2 new, 1 changed, 1 unchanged, 1 local-only
# Run without --dry-run to apply.
```

### Apply restore

```bash
sigillum restore --input vault.sigillum
# Enter passphrase: ********
# Restored 4 secrets (2 new, 1 updated, 1 unchanged).
```

### Restore with FIDO2

```bash
sigillum restore --input vault.sigillum --fido2
# Tap key 1 of 2... [tap]
# Tap key 2 of 2... [tap]
# Restored 4 secrets.
```

## Daemon API

### Export

```
POST /api/vault/backup
Content-Type: application/json

{
    "mode": "dual",
    "passphrase": "********"
}

Response: application/octet-stream (encrypted backup file)
```

### Import

```
POST /api/vault/restore
Content-Type: multipart/form-data

file: <backup file>
passphrase: ********

Response:
{
    "restored": 4,
    "new": 2,
    "updated": 1,
    "unchanged": 1
}
```

Upload limit: 10 MB.

## Scheduled Backups

The daemon can be configured to create automatic backups:

```bash
sigillum daemon --auto-backup-dir /backups --auto-backup-interval 24h
```

Automatic backups use the vault's master key (FIDO2 mode). The vault must be unlocked for automatic backups to succeed.

Backup files are named: `sigillum-YYYYMMDD-HHMMSS.sigillum`

## Best Practices

1. **Always use dual-mode** for important backups. If you lose your FIDO2 keys, the passphrase is your last resort.
2. **Store backups offsite**. A backup next to the vault it protects is useless if the machine is lost.
3. **Test your restores**. A backup you've never tested is a hope, not a plan.
4. **Use strong passphrases**. Argon2id is memory-hard but not invincible. A weak passphrase is still a weak passphrase.
5. **Rotate backups**. Don't keep only the latest. Keep the last N backups in case the latest is corrupted.
