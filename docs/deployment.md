# Deployment Guide

Sigillum currently has two practical deployment modes, both on a single machine,
plus one optional local-sidecar gateway. That single-machine boundary is the
intended product shape.

## Mode 1: Embedded Local Library

Use `sigillum` or `sigillum-core` directly when one process owns the vault files.

```rust
use sigillum::{FileVault, SecretStore, VaultConfig};

let vault = FileVault::new(VaultConfig::default());
vault.set_api_key("github", "ghp_...")?;
```

Use this mode when:

- one local process needs the secrets
- you do not need the browser UI
- independent unlock state per process is acceptable

## Mode 2: Local Daemon

Use the daemon when you want a long-lived local process to hold unlocked compartment keys in memory and expose the web UI.

Start it with:

```bash
cargo run -p sigillum-cli -- daemon --port 9743
```

Then open:

```text
http://localhost:9743
```

Use this mode when:

- you want the embedded web UI
- you want one local process to stay unlocked while you manage compartments
- you are working with passphrase or local FIDO2 setup flows
- you want local snapshot export and restore over the daemon API
- you want bearer session-token auth over local HTTP

## Data Location

By default, Sigillum stores data under:

```text
~/.sigillum/
```

The daemon and CLI both operate on that same local data directory unless configured otherwise in code.
Set `SIGILLUM_BASE_DIR` to point both tools at an alternate local directory for
tests or separate operator profiles.

## Mode 3: Local Gateway Sidecar

Use `sigillum-gateway` when you want a local payment-preview and webhook flow
surface beside the daemon.

Use this mode when:

- you need project-level payment intent creation against a local daemon
- you want the gateway to stay loopback-bound and single-host
- you are treating the gateway as a preview surface inside the same local trust boundary

The gateway talks to the daemon over local HTTP and is intended to stay part of
the same single-machine trust boundary.
Provide a pre-established daemon bearer token through
`SIGILLUM_DAEMON_SESSION_TOKEN` or `SIGILLUM_SESSION_TOKEN` when the gateway
needs authenticated daemon operations.

Before calling a host production-ready for this local boundary, run:

```bash
sigillum doctor
```

The doctor command fails for blocking local-readiness problems such as a
non-local daemon URL, unreachable daemon, unreadable audit database, or unsafe
Unix data-directory permissions. Missing initialization or missing session
tokens are reported as warnings so first-run setup remains possible.

## Operational Notes

- The daemon is intended for local use on one host.
- The daemon authenticates with bearer session tokens over local HTTP.
- Unlock state lives in daemon memory.
- Locking clears the loaded master keys from memory.
- Snapshot restore replaces on-disk state and clears daemon session/runtime state.
- Recent state-changing operations are appended to a local audit log and visible through the UI/API.
- The CLI is useful for setup and launch, but the daemon is the more coherent way to keep compartments unlocked across multiple operations.
- Reproducible local releases use the committed `Cargo.lock`, Rust `1.88.0`,
  `cargo audit`, and `cargo deny check`.

## What This Guide Does Not Cover

These deployment stories are out of scope for Sigillum's intended deployment model:

- remote daemon clients
- clustered or multi-host use
- remote audit-log services
- automatic backups
