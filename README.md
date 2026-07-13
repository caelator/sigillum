# Sigillum

**Local-first EVM wallet archaeology, treasury operations, and controlled recovery.**

[Documentation](docs/README.md) · [Security](SECURITY.md) ·
[Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

> [!IMPORTANT]
> Sigillum has not published a supported stable release. Build and evaluate the
> current source on a non-production machine with test wallets first. The visible
> `v1.0.0-rc.2` tag is not a valid release candidate: its release workflow failed
> the annotated-tag contract, and no GitHub Release was published from it.

Sigillum is a self-hosted, single-operator workstation for finding EVM wallets
and assets, understanding their provenance and risk, preparing consolidation
plans, and deliberately executing approved recovery work. Private keys and
vault material stay on the operator's computer; Sigillum is not a hosted wallet,
custodian, browser extension, or internet-facing service.

## What it does

- Discovers and inventories EVM addresses, native assets, ERC-20 balances,
  approvals, NFTs, DeFi positions, rewards, and claim candidates.
- Records provenance, counterparties, signability, risk findings, and required
  gas before value is moved.
- Builds reviewable consolidation plans with destination allowlists, value caps,
  simulation checks, and optional cross-payer linkage blocking.
- Queues and executes approved EIP-1559 transfers and supported plan steps using
  fail-closed, crash-aware state transitions. Execution is opt-in and defaults off.
- Creates purpose-labeled receive addresses and supports ERC-5564 stealth
  receiving flows for payer-private local operations.
- Protects high-sensitivity vault data with AES-256-GCM and supports local
  passphrase or FIDO2-backed compartment unlock.
- Provides the same local system through a native macOS shell, embedded
  browser console, command-line interface, and typed Rust client/API crates.

## Scope and safety boundary

Sigillum 1.0 targets EVM networks on one trusted machine. It does **not** claim
support for:

- hosted, remote, multi-user, or internet-facing operation;
- Bitcoin or other non-EVM chains;
- swap execution or fiat/NFT valuation;
- production payment processing through `sigillum-gateway`;
- protection after the host or Sigillum process is compromised;
- notarized distribution, crates.io packages, or a supported stable release.

The optional gateway is a disabled-by-default local preview. Its balance
observations are not payment-finality proofs. See the [security policy](SECURITY.md)
and [stability policy](docs/stability.md) before using real wallet material.

## Start from source

Requirements:

- Rust `1.88.0` (selected by `rust-toolchain.toml`)
- macOS or Linux development dependencies
- a local Chromium-family browser for the full browser smoke test
- USB HID access only when evaluating FIDO2 hardware flows

Clone and verify the workspace:

```bash
git clone https://github.com/caelator/sigillum.git
cd sigillum
cargo build --locked
cargo test --workspace --locked
```

Create the first local compartment:

```bash
cargo run --locked -p sigillum-cli -- setup
```

Start the local daemon and embedded console:

```bash
cargo run --locked -p sigillum-cli -- daemon --port 9743
```

Open <http://127.0.0.1:9743>. Before treating the machine as locally ready, run:

```bash
cargo run --locked -p sigillum-cli -- doctor
```

### macOS desktop app

The Tauri v2 desktop shell runs the daemon in-process on an ephemeral loopback
port and uses the same `~/.sigillum` data directory:

```bash
cargo install tauri-cli --version 2.11.4 --locked
cd crates/sigillum-desktop
cargo tauri build -- --locked
```

Local builds without Apple credentials are ad-hoc signed, not notarized. Read
the [deployment guide](docs/deployment.md) before installing the bundle.

### Rust library use

The workspace crates are not published to crates.io. Pin a Git revision when
using a crate as a source dependency:

```toml
[dependencies]
sigillum = {
  git = "https://github.com/caelator/sigillum.git",
  rev = "<commit>"
}
```

The file-vault API is intentionally narrower than the full daemon product:

```rust
use sigillum::{FileVault, SecretStore, VaultConfig};

let vault = FileVault::new(VaultConfig::default());
vault.set_api_key("provider", "token")?;
```

## Architecture at a glance

```text
macOS desktop ─┐
browser UI ────┼── local Axum daemon ── wallet inventory / plans / queue
CLI / client ──┘          │
                          ├── encrypted compartments + local SQLite audit
                          ├── EVM RPC providers
                          └── optional loopback-only gateway preview
```

The workspace separates shared transport types, the async client, vault core,
FIDO2 support, daemon, CLI, desktop shell, gateway, and integration facades.
Private signing material stays inside the local trust boundary. The daemon is
session-gated over loopback HTTP, but loopback is not a substitute for a trusted
host. See [Architecture](docs/architecture.md) for the detailed component and
state model.

## Documentation

Start with the [documentation map](docs/README.md). Key references include:

- [Deployment and local operation](docs/deployment.md)
- [Backup, restore, and migration](docs/backup.md)
- [FIDO2 model and constraints](docs/fido2.md)
- [Privacy and linkage model](docs/architecture.md#privacy--linkage-model)
- [Current readiness evidence](docs/production-readiness-audit.md)
- [Wallet-management roadmap](docs/wallet-management-roadmap.md)

## Development

The full gate checks formatting, architecture constraints, UI build freshness,
Rust builds/tests/lints, runtime and browser smoke flows, dependency advisories,
licenses, and repository cleanliness:

```bash
./scripts/check-release.sh
```

For smaller changes, see [CONTRIBUTING.md](CONTRIBUTING.md). Security findings
must be reported privately according to [SECURITY.md](SECURITY.md), not opened as
public issues.

## Release status

- Workspace version: `1.0.0`
- Supported stable release: none
- Published GitHub Release: none
- Current supported boundary: source evaluation only

The workspace version describes the intended 1.0 contract; it does not by itself
mean a release was published. Final release status is authoritative only when an
annotated `v1.0.0` tag and its GitHub Release both exist and the release workflow
passes.

## License

Licensed under either [Apache License 2.0](LICENSE-APACHE-2.0) or
[MIT](LICENSE-MIT), at your option.
