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

Use `sigillum-gateway` only when you want an experimental local payment-preview
and webhook flow surface beside the daemon. Payment creation is disabled by
default and is outside supported 1.0 payment-confirmation semantics.

Use this mode when:

- you are explicitly testing project-level payment observations against a local
  daemon
- you want the gateway to stay loopback-bound and single-host
- you are treating the gateway as a preview surface inside the same local trust boundary

The gateway talks to the daemon over local HTTP and is intended to stay part of
the same single-machine trust boundary.
Set `GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS=1` to opt into payment creation.
When the flag is absent, the gateway does not start its payment poller and does
not refresh/list daemon deposits or retry payment webhooks in the background.
Address-balance reads can produce `payment.observed` events with
`latest_balance_observation_at`, but do not prove chain finality. No privileged
third-party invoice-signing callback is implemented.
Provide a pre-established daemon bearer token through
`SIGILLUM_DAEMON_SESSION_TOKEN` or `SIGILLUM_SESSION_TOKEN` when the gateway
needs authenticated daemon operations.

## Mode 4: Desktop App (macOS)

The desktop app is a Tauri v2 macOS shell that runs `sigillum-daemon`
in-process on a background thread, waits for local readiness, and opens the
daemon web console in the native window.

### Supported release matrix

There is no published stable release yet. For the 1.0 release-candidate
ceremony, support is intentionally narrower than “runs on macOS”:

| Surface | OS and CPU | Status |
|---|---|---|
| Desktop app and CLI | macOS 15.x, Apple Silicon (`aarch64`) | Intended 1.0 target; becomes supported only after fresh RC7 doctor, standard-soak, chaos-soak, and clean-machine install/unlock receipts |
| CLI archive | Ubuntu 24.04, `x86_64` | Built and source-gated in CI; not a supported target host until it has the same RC-bound operational receipts |
| Linux desktop | Linux | Compile-only |
| macOS on Intel, earlier macOS releases, Windows | Other | Unsupported for 1.0; no release artifact or target-host evidence |

The evidence contract is fail-closed: a host outside macOS 15.x/aarch64 cannot
produce qualifying F4, doctor, clean-install, or UI receipts. In particular,
the current macOS 26.5.2/arm64 development host is ineligible even if local
source tests pass.

The permanent macOS application identifier is `com.sigillum.desktop`. Changing
it after release would create a distinct application identity and requires an
explicit compatibility and migration decision.

### Build the bundle

Install the Tauri v2 CLI, then build through the project wrapper from the
workspace root:

```bash
cargo install tauri-cli --version 2.11.4 --locked
./scripts/build-macos-bundle.sh -- --locked
```

The build writes, under the workspace root:

- `target/release/bundle/macos/Sigillum.app`
- `target/release/bundle/dmg/Sigillum_<version>_<arch>.dmg`

The release gate runs `scripts/check-desktop.sh` for repeatable desktop
coverage. It always compiles `sigillum-desktop`; on macOS it also runs
the wrapper in debug mode, requires exactly one debug `.app` and `.dmg`, and
strictly verifies both the source app and the app mounted read-only from the
dmg. The debug build uses a deterministic temporary notice file through the
same Tauri resource overlay as release builds, then removes the source fixture.
Verification requires the exact identifier and executable, a bound
`Info.plist`, sealed resources, nonempty `_CodeSignature/CodeResources`, the
expected signature mode, hardened runtime, and matching CDHash values. It also
runs negative regressions for the RC3 linker-only failure, missing hardened runtime,
tampering, wrong identifiers, CDHash mismatch, symlinks, and malformed dmg
layouts. Developer ID mode additionally requires the dmg to carry a non-ad-hoc
Developer ID signature from the same team as the app and validates stapled
notarization tickets on both app copies and the dmg. Mode-independent hostile
dmg-layout regressions run in the always-on ad-hoc suite. Tauri notarizes and
staples the app before creating the dmg; the project build wrapper then submits,
staples, and validates the signed dmg with `scripts/notarize-macos-dmg.sh`.
Set
`SIGILLUM_SKIP_DESKTOP_BUNDLE=1` only
on non-CI macOS hosts that cannot build Tauri bundles; CI rejects the toggle.

### Third-party license notices

Release binaries ship with a generated `THIRD-PARTY-NOTICES.txt`, containing
MIT/Apache-style attribution for all bundled Rust dependencies. `cargo deny`
gates licenses but does not produce attribution, so the release workflow
(`.github/workflows/release.yml`) generates the file with `cargo-about` pinned
at `0.9.1`, using the committed `about.toml` accepted-license list that mirrors
`deny.toml` and the `about.hbs` template at the repo root.

The workflow writes the file to
`crates/sigillum-desktop/THIRD-PARTY-NOTICES.txt` and then builds with a Tauri
config overlay:

```bash
./scripts/build-macos-bundle.sh \
  --config '{"bundle":{"resources":{"THIRD-PARTY-NOTICES.txt":"THIRD-PARTY-NOTICES.txt"}}}' \
  -- --locked
```

That overlay merges the file into the bundle resources, so it lands at
`Sigillum.app/Contents/Resources/THIRD-PARTY-NOTICES.txt` inside the shipped
`.dmg` and `.app.zip`. The committed `tauri.conf.json` intentionally does not
list the resource, because Tauri fails a build when a listed resource file is
missing and the actual file only exists after generation. Raw local builds do
not include notices unless given the overlay. `check-desktop.sh` uses a
temporary fixture under ignored `target/` to exercise the identical overlay
and seal shape; release builds replace that fixture with the generated notice.

To reproduce locally:

```bash
cargo install cargo-about --version 0.9.1 --locked --features cli
cargo about generate --output-file crates/sigillum-desktop/THIRD-PARTY-NOTICES.txt about.hbs
./scripts/build-macos-bundle.sh \
  --config '{"bundle":{"resources":{"THIRD-PARTY-NOTICES.txt":"THIRD-PARTY-NOTICES.txt"}}}' \
  -- --locked
dmg_files=(target/release/bundle/dmg/Sigillum_*.dmg)
test "${#dmg_files[@]}" -eq 1
./scripts/check-macos-bundle-signature.sh \
  --mode adhoc \
  target/release/bundle/macos/Sigillum.app \
  "${dmg_files[0]}"
```

Use `--mode developer-id` for a fully credentialed Developer ID build. The
release workflow runs the same verifier after the notices overlay and before
it stages or uploads any artifact.

The workflow also attaches `THIRD-PARTY-NOTICES.txt` directly to the GitHub
release next to `SHA256SUMS`.

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

### Upgrade, rollback, and uninstall

Before an upgrade, lock and quit Sigillum, export an encrypted snapshot, and
verify that the snapshot can be read by the current version. Verify the new
`.dmg` against `SHA256SUMS`, replace only `/Applications/Sigillum.app`, then
launch the new version and run `sigillum doctor`. The application bundle and
`~/.sigillum` are separate; replacing the app does not remove operator data.

Store migrations are forward-only. Do not install an older binary over a data
directory that a newer version has opened. To roll back, restore a snapshot
captured by the older version into a separate base directory and run that older
binary against the separate directory. Never point two versions at the same
live data directory.

For an app-only uninstall, lock and quit Sigillum and move
`/Applications/Sigillum.app` to Trash. This deliberately retains
`~/.sigillum`, exported snapshots, and any external release evidence. To remove
operator data as a separate, explicit action, first preserve any required audit
or recovery evidence, then archive or remove `~/.sigillum` and every exported
snapshot. Filesystem snapshots and backups may retain copies; ordinary deletion
is not a cryptographic erasure guarantee.

### First launch on the supported macOS 15.x target (Gatekeeper)

Credential-free builds made through the project wrapper are ad-hoc signed.
macOS 15 removed the older
right-click-then-Open bypass for this case, so use the Privacy & Security
approval flow:

1. Double-click `Sigillum.app`. macOS reports that it "was not opened" because
   Apple could not verify it. Click `Done`, not `Move to Trash`.
2. Open System Settings -> Privacy & Security.
3. Scroll to the Security section, where `"Sigillum" was blocked to protect your Mac.` appears.
4. Click `Open Anyway` and authenticate.
5. Click `Open Anyway` or `Open` in the confirmation dialog.

To verify an installed credential-free build, require a complete bundle
signature and inspect the exact metadata:

```bash
codesign --verify --deep --strict --verbose=4 /Applications/Sigillum.app
codesign -dv --verbose=4 /Applications/Sigillum.app 2>&1 | \
  grep -E '^(Identifier=|Signature=|Info\.plist |TeamIdentifier=|Sealed Resources )'
```

The credential-free path must show `Identifier=com.sigillum.desktop`,
`Signature=adhoc`, `Info.plist entries=...`, `TeamIdentifier=not set`, and
`Sealed Resources version=2 ...`. A `Signature=adhoc` line by itself is not
proof: RC3's linker-only binary printed that line while its bundle failed
strict verification.

### Full signing and notarization (optional release mode)

Set the complete signing trio before invoking the project wrapper:

- `APPLE_CERTIFICATE`: base64-encoded `.p12`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`, for example `Developer ID Application: <name>`

Developer ID mode also requires exactly one complete notarization credential
family. For Apple ID credentials, set:

- `APPLE_ID`
- `APPLE_PASSWORD`, using an app-specific password
- `APPLE_TEAM_ID`

Alternatively, use the complete App Store Connect API-key trio:

- `APPLE_API_KEY` (the key ID)
- `APPLE_API_ISSUER`
- `APPLE_API_KEY_PATH` (a readable, nonempty `.p8` file)

The wrapper treats empty and whitespace-only values as absent. It accepts only
one of these states:

- no credentials, or explicit `APPLE_SIGNING_IDENTITY=-`: enforce a complete
  ad-hoc app-bundle signature and do not notarize;
- all three signing variables, with a non-`-` identity, plus exactly one
  complete notarization trio: Developer ID signing, notarization, and stapling
  of both app copies and the dmg.

The Developer ID path contacts Apple's notary service for both release and
`--debug` builds. The credential-routing regression uses fake tools and runs
offline on both CI platforms; it never submits an artifact or prints credential
arguments. API-key notarization is supported for local/manual wrapper runs; the
GitHub release workflow currently forwards the Apple-ID family only and does
not materialize a `.p8` secret file.

Partial signing fields, partial notarization fields, both notarization
families, Developer ID signing without notarization, notarization with ad-hoc
signing, or an unreadable API-key path fail before Tauri starts. The project
does not require `spctl` for ad-hoc builds;
their documented Gatekeeper path remains manual. The GitHub release workflow
maps the signing and Apple-ID secrets into this same validator and defaults to
the explicit ad-hoc mode when they are absent.

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

If packaged startup fails, Sigillum shows a native alert with a sanitized
diagnostic code and writes a `0600` report to
`~/Library/Logs/Sigillum/desktop-startup.log`. The alert and report deliberately
omit raw startup errors so secrets, session tokens, and sensitive configuration
are not copied into support material. Run:

```bash
sigillum doctor
```

For local development, launching the binary from a terminal also exposes the
underlying daemon error on stderr:

```bash
/Applications/Sigillum.app/Contents/MacOS/sigillum-desktop
```

Common causes are unsafe `~/.sigillum` permissions, which the daemon requires
and repairs to `0700` on Unix, or a corrupted or foreign base directory.

Do not run a standalone `sigillum daemon --port 9743` against the desktop
app's data directory. The desktop app picks its own ephemeral port, but Sigillum
permits only one daemon process per data directory; the second process exits
after failing to acquire the daemon lock. For isolated development, set
`SIGILLUM_BASE_DIR` to a different private directory before starting the
standalone daemon.

If the window shows the static `Sigillum is starting…` page, the webview
loaded the bundled fallback. Press Cmd+R to reload.

Before calling a source checkout release-ready for this local boundary, run:

```bash
./scripts/check-release.sh
```

The gate requires the committed lockfile for dependency-resolving Cargo
commands, verifies both default and no-HID FIDO2 configurations, and fails if
any check changes the tracked tree. Run it without concurrent agents or other
release gates modifying the checkout.

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
schema-v2 receipt with the commit, dirty-checkout state, timing, iteration
count, doctor count, checked surfaces, explicit platform, macOS
`ProductVersion`, canonical architecture, and an opaque SHA-256 machine
identity. Standard and chaos release receipts must both report macOS 15.x,
`aarch64`, and the same identity digest. Set `SIGILLUM_SOAK_KEEP_ARTIFACTS=1` only
when you need daemon/gateway logs for investigation, because that keeps the
temporary harness directory on disk.

## Release Dry Run (RC tag)

The release workflow (`.github/workflows/release.yml`) triggers on tags matching
`v*`, then rejects tags that are not annotated, do not match the workspace
version (or its `-rc.N` form), lack a dated changelog section, or are not on
`main` history. `v1.0.0-rc.5` peels to pre-hardening protected-main commit
`7e04743`; Release run `29248938476` passed all six jobs and produced the
expected six assets, but its GitHub Release remains an unpublished historical
draft. It cannot certify the integrated hardening line. `v1.0.0-rc.6` tag
object `1687443c` peels to protected-main commit `194a903`; run `30600446396`
passed the release contract, both source-verification legs, and both artifact
jobs. The final job created the correct unique unpublished prerelease draft
with all six expected checksum-valid assets, then failed because its list query
ran 336 ms later and did not yet observe the draft. RC6 is therefore immutable
failed-workflow evidence and must not be rerun, moved, deleted, or promoted.
Only after the bounded visibility fix passes the clean gate and review, lands
through protected `main`, and passes required CI is annotated
`v1.0.0-rc.7` eligible.
The authoritative, fail-closed ceremony is section 6 of
[`execution-runbook-1.0.md`](./execution-runbook-1.0.md); do not replace its
pinned-SHA and post-gate identity checks with a shorter tag command:

1. Follow the runbook from a clean checkout, pin `GATE_SHA`, and run the full
   gate.
2. Select the next monotonically increasing RC number from the retained remote
   `v1.0.0-rc.*` tags, reassert `HEAD == GATE_SHA == origin/main`, then create
   and push that annotated tag with an explicit non-force refspec.
3. Watch the Release workflow in the Actions tab; the contract job, both verify legs, both artifact jobs, and release job must all pass.
4. Verify the RC release is a draft, unpublished, and `prerelease=true`, then
   verify it contains the `.dmg`, zipped `.app`, both `sigillum-cli` `tar.gz`
   archives, `THIRD-PARTY-NOTICES.txt`, and `SHA256SUMS`.
   `scripts/check-release-state-contract.sh rc-draft` performs the
   fail-closed release-state check against the fetched release JSON.
5. Download the assets and run `shasum -a 256 --check SHA256SUMS --ignore-missing`.
6. Confirm the release remains an unpublished prerelease draft and its body
   carries the dated `CHANGELOG` section for the version. There is no fallback
   release body.
7. Record the tag-object ID, peeled commit, workflow run, and checksum result.
   Retain the RC draft/assets through final-draft verification and retain every
   pushed RC tag permanently. Only after final publication may the RC draft be
   deleted; never move, delete, or reuse its tag number.

Final `v1.0.0` promotion is a separate fail-closed ceremony in H2 of
[`release-1.0-plan.md`](./release-1.0-plan.md). It binds the exact sanitized
operator-evidence archive digest into the protected annotated tag, waits for
the exact final workflow, reruns source verification while skipping artifact
rebuilds, copies the exact five qualified RC payload bytes to final names,
verifies byte identity and tag-normalized digests, regenerates `SHA256SUMS`,
requires each final-draft snapshot to be `prerelease=false`, uploads the
evidence bundle as the seventh asset without replacement, re-downloads and
re-verifies all seven assets, then requires the published release to retain
`prerelease=false`. `scripts/check-release-state-contract.sh` enforces those
`final-draft` and `final-published` transitions before the conditional
publication authorized by the H2 approval recorded before the ceremony began.
The exact-byte promotion path is implemented, but RC6 is an immutable
failed-workflow receipt and H2 remains blocked until the visibility fix lands,
protected CI passes, RC7 qualifies, and all RC7 operator receipts and the
evidence bundle pass.

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
