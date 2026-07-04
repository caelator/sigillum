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

## Crash-Recovery Guarantees

Sigillum journals destructive local operations before filesystem mutation and
reconciles interrupted work during daemon startup, before non-health API routes
are opened.

### Compartment initialization

- Pre-mutation interruption leaves no compartment files. Startup clears the
  journal and the vault remains uninitialized.
- Mid-mutation interruption can leave a partial compartment directory. Startup
  leaves the journal pending instead of guessing that the compartment is usable.
- Post-mutation interruption leaves a complete compartment directory and the
  `.initialized` marker. Startup clears the journal.

### Compartment removal and replacement

- Pre-mutation interruption leaves the live compartment tree unchanged. Startup
  clears the journal.
- Mid-mutation interruption can leave `.replacing` and `.rollback` sibling
  directories. Startup restores the rollback tree to the live compartment path
  and removes the staging directory.
- Post-mutation interruption leaves the replacement tree in the live path.
  Startup removes stale staging/rollback directories and clears the journal.

### Snapshot restore

- Pre-mutation interruption leaves the original data directory unchanged.
  Startup clears the journal.
- Mid-mutation interruption rolls back to the original tree if the restore swap
  did not finish. Startup removes the staging tree and clears the journal.
- Post-mutation interruption leaves the restored tree in place and the previous
  tree in the rollback sibling. Startup purges the rollback sibling and keeps
  the restored tree.

`sigillum setup reset` is not part of the snapshot journal. It archives the
current encrypted data directory and recreates an empty private directory; an
interruption leaves either the original tree or an archive plus fresh empty
tree, both of which are valid startup states.
