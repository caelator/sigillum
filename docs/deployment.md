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
On Unix, daemon startup creates or repairs this directory with `0700`
permissions before opening runtime state.

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

## Mode 4: Desktop App (macOS)

The desktop app is a Tauri v2 macOS shell that runs `sigillum-daemon`
in-process on a background thread, waits for local readiness, and opens the
daemon web console in the native window.

### Build the bundle

Install the Tauri v2 CLI, then build from the desktop crate:

```bash
cargo install tauri-cli --version '^2' --locked
cd crates/sigillum-desktop
cargo tauri build
```

The build writes, under the workspace root:

- `target/release/bundle/macos/Sigillum.app`
- `target/release/bundle/dmg/Sigillum_<version>_<arch>.dmg`

### Verify the download before opening

Compare the `.dmg` checksum with the release `SHA256SUMS` file:

```bash
shasum -a 256 Sigillum_<version>_<arch>.dmg
```

Or check the release manifest directly:

```bash
shasum -a 256 --check SHA256SUMS --ignore-missing
```

Do not open a `.dmg` whose checksum does not match the release `SHA256SUMS`.

### Install

Open the `.dmg`, drag `Sigillum.app` into `/Applications`, then eject the
mounted image.

### First launch on macOS 15+ (Gatekeeper)

Default builds are ad-hoc signed. macOS 15 removed the older
right-click-then-Open bypass for this case, so use the Privacy & Security
approval flow:

1. Double-click `Sigillum.app`. macOS reports that it "was not opened" because
   Apple could not verify it. Click `Done`, not `Move to Trash`.
2. Open System Settings -> Privacy & Security.
3. Scroll to the Security section, where `"Sigillum" was blocked to protect your Mac.` appears.
4. Click `Open Anyway` and authenticate.
5. Click `Open Anyway` or `Open` in the confirmation dialog.

To verify that a local build is ad-hoc signed:

```bash
codesign -dv /Applications/Sigillum.app
```

`codesign` shows `Signature=adhoc` for the default unsigned-credential build.

### Full signing and notarization (optional)

Set the standard Tauri v2 signing variables before building:

- `APPLE_CERTIFICATE`: base64-encoded `.p12`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`, for example `Developer ID Application: <name>`

For notarization, also set:

- `APPLE_ID`
- `APPLE_PASSWORD`, using an app-specific password
- `APPLE_TEAM_ID`

When these variables are present, `cargo tauri build` signs the bundle and
notarizes it when the `APPLE_ID` trio is present. When they are absent, the
build never fails for missing Apple credentials and remains ad-hoc signed.

### Runtime behavior

The desktop app uses the same data directory as the CLI and daemon:
`~/.sigillum` by default, or `SIGILLUM_BASE_DIR` when set. A Finder-launched app
does not see shell exports. To use a custom base directory, launch the binary
directly from a terminal:

```bash
/Applications/Sigillum.app/Contents/MacOS/sigillum-desktop
```

Each launch chooses a new ephemeral `127.0.0.1` port, waits up to 10 seconds for
TCP readiness, then opens the console in the native window. Launching a second
copy focuses the existing window. The system tray shows live lock state as
`Sigillum: Locked` or `Sigillum: Unlocked`, refreshes every 2 seconds, and
offers `Show Sigillum`, `Lock now`, and `Quit`.

Closing the window does not exit. It hides the window to the tray and
immediately locks the daemon, clearing loaded master keys from memory. Quit from
the app menu or tray locks the daemon first, bounded by a 3-second timeout, then
exits.

### Troubleshooting

If startup reports `daemon did not accept TCP connections within 10s` or
`daemon exited before readiness`, launch from a terminal to see the daemon error
on stderr:

```bash
/Applications/Sigillum.app/Contents/MacOS/sigillum-desktop
```

Common causes are unsafe `~/.sigillum` permissions, which the daemon requires
and repairs to `0700` on Unix, or a corrupted or foreign base directory.

Running a standalone `sigillum daemon --port 9743` at the same time is fine.
The desktop app picks its own ephemeral port, but the two daemon processes hold
independent unlock state over the same data directory.

If the window shows the static `Sigillum is starting…` page, the webview
loaded the bundled fallback. Press Cmd+R to reload.

Before calling a source checkout release-ready for this local boundary, run:

```bash
./scripts/check-release.sh
```

Before calling a host operationally ready for this local boundary, run:

```bash
sigillum doctor
```

The doctor command fails for blocking local-readiness problems such as a
non-local daemon URL, unreachable daemon, unreadable audit database, or unsafe
Unix data-directory permissions. Missing initialization or missing session
tokens are reported as warnings so first-run setup remains possible.
The release gate also runs `scripts/check-runtime-smoke.sh`, which starts a
temporary daemon, checks the served UI shell, initializes a passphrase
compartment, verifies vault write/read canaries, locks and unlocks it, verifies
the canaries again, and runs `sigillum doctor` against both first-run and
unlocked states.
After the runtime smoke, the gate runs `scripts/check-browser-smoke.sh`, which
starts another temporary daemon and drives a headless Chromium-family browser
through setup-wizard passphrase initialization, the unlocked operator
workspace, vault canary write/reveal, browser-session logout, and passphrase
re-authentication, failing on any browser console or runtime error. It needs a
local Chrome, Chromium, Brave, or Edge (override with `CHROME_BIN`); hosts
without one can skip it with `SIGILLUM_SKIP_BROWSER_SMOKE=1`.
For longer pre-production confidence, run `scripts/check-local-soak.sh` on the
target host. Set `SIGILLUM_SOAK_SECONDS` and `SIGILLUM_SOAK_INTERVAL_SECONDS` to
control duration and cadence; the harness repeatedly checks daemon status,
gateway health, vault write/read canaries, and `sigillum doctor`. Set
`SIGILLUM_SOAK_RECEIPT=target/readiness/local-soak.json` to write a durable JSON
receipt with the commit, dirty-checkout state, host, timing, iteration count,
doctor count, and checked surfaces. Set `SIGILLUM_SOAK_KEEP_ARTIFACTS=1` only
when you need daemon/gateway logs for investigation, because that keeps the
temporary harness directory on disk.

## Operational Notes

- The daemon is intended for local use on one host.
- The daemon authenticates with bearer session tokens over local HTTP.
- Unlock state lives in daemon memory.
- Locking clears the loaded master keys from memory.
- Snapshot restore replaces on-disk state and clears daemon session/runtime state.
- Recent state-changing operations are appended to a local audit log and visible through the UI/API.
- The CLI is useful for setup and launch, but the daemon is the more coherent way to keep compartments unlocked across multiple operations.
- Reproducible local releases use the committed `Cargo.lock`, Rust `1.88.0`,
  `./scripts/check-release.sh`, and `sigillum doctor` on the target host.

## What This Guide Does Not Cover

These deployment stories are out of scope for Sigillum's intended deployment model:

- remote daemon clients
- clustered or multi-host use
- remote audit-log services
- automatic backups
