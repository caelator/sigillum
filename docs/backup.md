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

## What Snapshots Retain After Pruning

Since plan task 3.2 the at-rest linkage ledger is forgettable: scanned-address
prune (`POST /api/inventory/addresses/delete`), retired-allocation purge
(`POST /api/treasury/receive-addresses/purge`), and the profile-delete
`prune_inventory` cascade all delete rows from `wallet_inventory.json` (see
`docs/architecture.md` → "At-rest forgetting"). How that interacts with
backups:

- **A snapshot archives what existed at export time.** A snapshot taken
  BEFORE a prune retains the pruned history forever — that is what snapshots
  are for. Restoring it brings the history back with the rest of the tree.
  `setup/reset` archives of the data directory behave the same way.
- **The live tree stays pruned.** Every store save — and every successful
  store load — rewrites the `.bak` companion next to each JSON document, so
  pruned rows do not linger in the live directory's backup copies after a
  prune completes.
- **The audit log keeps the prune events by design.** The audit trail is
  append-only: `wallet_inventory.addresses.prune`,
  `treasury.receive.purge`, and `wallet_inventory.profile_prune` events
  remain, but they carry selector scope and per-store counts only — never
  the pruned address values.

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

## Version-Upgrade Guarantees (0.1 → 1.0)

Upgrading a 0.1-era data directory to the 1.0 daemon requires **no manual
migration step**. You install the newer binary and start it against the same
base directory; every persisted store migrates automatically.

### What migrates automatically

Each schema-versioned JSON store is wrapped in a `{schema, schema_version,
data}` envelope. The 1.0 daemon reads older versions transparently and, on the
next write to that store, re-saves it at the current schema version. Across the
1.0 line the following stores carry forward:

- `profiles.json` — provider, stealth, xpub, and seed wallet profiles
  (legacy unwrapped documents → current version).
- `deposits.json` — tracked stealth deposits.
- `queue.json` — the transaction queue, including any pending job.
- `wallet_inventory.json` — chain profiles, watch address book, discovered
  addresses and holdings, the risk catalog and findings, consolidation plans,
  **treasury policy, receiving allocations, and counterparties** (all held in
  this store).
- `token_registry.json` — locally imported ERC-20 token lists.

Audit history migrates from the legacy JSONL log (`audit.log`) into the SQLite
audit database (`audit.db`) on startup; the old log is preserved as
`audit.log.migrated` rather than deleted.

Migrations are forward-only and additive: fields introduced by newer schema
versions take documented defaults when an older document omits them, so no
operator data is dropped in the process. Passphrase-encrypted snapshots use a
version-stable format, so a snapshot exported by a 0.1-era daemon restores
cleanly under 1.0, and the restored stores then migrate on first read exactly
as an in-place upgrade would.

### Fail-closed on damage

Migration never trades safety for convenience. If a store file cannot be
parsed, the daemon recovers the matching `.bak` backup and quarantines the
unreadable file as `<name>.corrupt-<timestamp>` — it is moved aside, never
overwritten or discarded. A store that cannot be read from either the live file
or its backup fails closed rather than silently resetting to empty state.

### Verification

These guarantees are proven end to end by
`crates/sigillum-daemon/tests/upgrade_path.rs` (task F7): a committed
fixture data directory built at the oldest supported per-store schema versions
boots on the current daemon and is asserted to migrate every store to its
current version (queue → v5, wallet inventory → v20, and so on) with no
quarantine events, the vault canaries intact, the pending queue job preserved,
and the 0.1-era encrypted snapshot restoring under 1.0.
