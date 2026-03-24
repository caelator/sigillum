# Backup and Restore

Sigillum now supports local passphrase-encrypted snapshots of the data directory.

## What Gets Captured

Snapshots archive the Sigillum storage tree under:

```text
~/.sigillum/
```

That includes:

- compartment files such as `api_keys.json`, `vault.enc`, and `meta.enc`
- passphrase wrapping material when present
- FIDO2 shard/config files such as `fido2_keys.json`
- initialization markers and compartment directory layout

Snapshots are encrypted with a passphrase before they leave disk.

## What the Snapshot Is Not

The older phrase "backup passphrase" in setup flows refers to passphrase-based master-key wrapping for unlock fallback.

That fallback passphrase is separate from the snapshot feature:

- fallback passphrase: unwraps a compartment master key
- snapshot passphrase: encrypts an exported archive file

## CLI

Export:

```bash
sigillum backup export --output sigillum-snapshot.json
```

Restore:

```bash
sigillum backup restore --input sigillum-snapshot.json
```

Restore replaces the current local data directory with the snapshot contents.

## Daemon API

When the daemon is running, the local HTTP API exposes:

- `POST /api/backup/export`
- `POST /api/backup/restore`

These endpoints are local-service operations. Export requires an authenticated session. Restore requires authentication when the daemon is already initialized, and it clears the current session state after a successful restore.

## Operational Notes

- snapshots are local-first and passphrase-encrypted
- restore is a whole-tree replacement, not a merge
- restoring logs out the daemon and requires a fresh unlock
- there is still no scheduled backup system or remote backup service
