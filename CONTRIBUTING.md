# Contributing to Sigillum

Thanks for helping improve Sigillum. This project is a security-sensitive,
local-first EVM wallet workstation, so changes are reviewed for correctness,
operator clarity, and failure behavior—not only whether they compile.

## Before you start

- Use a public issue for bugs, feature discussions, and documentation gaps.
- Do **not** disclose vulnerabilities or sensitive wallet data in an issue. Use
  the private process in [SECURITY.md](SECURITY.md).
- Do not use production keys, seed phrases, addresses with privacy requirements,
  provider credentials, or customer data in examples, fixtures, logs, or PRs.
- Keep proposals inside the single-host, local-only product boundary unless an
  accepted design explicitly changes it.

## Set up the workspace

```bash
git clone https://github.com/caelator/sigillum.git
cd sigillum
cargo build --locked
cargo test --workspace --locked
```

The pinned Rust toolchain is declared in `rust-toolchain.toml`. The daemon UI
also uses Node/npm inside `crates/sigillum-daemon/ui`.

## Workspace map

| Crate | Responsibility |
| --- | --- |
| `sigillum-api` | Shared local-daemon transport types |
| `sigillum-client` | Async client for the local daemon |
| `sigillum-core` | Vault traits, encryption helpers, and file backend |
| `sigillum-fido2` | Local HID/FIDO2 and shard recovery |
| `sigillum-daemon` | Local API, state, wallet operations, and embedded UI |
| `sigillum-cli` | Setup, diagnostics, daemon launch, and operator commands |
| `sigillum-desktop` | Native Tauri shell around the local daemon console |
| `sigillum-gateway` | Experimental loopback-only payment observation preview |
| `sigillum-generator` | Password, passphrase, and TOTP generation |
| `sigillum-sdk` / `sigillum-server` | Integration facades |
| `sigillum` | File-vault meta-crate |

Read [docs/architecture.md](docs/architecture.md) before changing boundaries or
state ownership.

## Make a change

1. Fork the repository and branch from `main`.
2. Keep the change focused and include regression tests for behavior changes.
3. Update public documentation when commands, configuration, storage, security,
   or supported behavior changes.
4. Avoid new `unwrap`, `expect`, silent fallback, or destructive recovery paths
   in security-sensitive code.
5. Use synthetic test data and redact logs before attaching them.

Useful targeted checks:

```bash
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The authoritative pre-merge gate is:

```bash
./scripts/check-release.sh
```

It also exercises the daemon UI, local runtime, browser flow, dependency audit,
license policy, and repository cleanliness. Some platform-dependent checks need
the prerequisites documented in [docs/deployment.md](docs/deployment.md).

## Pull requests

A useful PR explains:

- the operator problem and chosen boundary;
- security, privacy, persistence, and rollback implications;
- tests and manual verification performed;
- documentation or migration impact;
- known limitations that remain.

Use imperative, specific commit messages (for example, `Reject stale execution
approvals`). Maintainers may ask for a smaller patch, additional failure-path
tests, or an architecture record for changes that alter trust boundaries.

## Good contribution areas

- reproducible bug fixes with regression tests;
- accessibility and operator-workflow improvements;
- test coverage for corruption, crash recovery, policy, and signing boundaries;
- documentation accuracy and safer onboarding;
- performance improvements backed by measurements;
- platform support that preserves the local-only security model.

Hosted service modes, weakened policy checks, secret-bearing diagnostics, and
breaking public contract changes require explicit design agreement before code.

## License

By contributing, you agree that your contribution is licensed under MIT OR
Apache-2.0, matching the repository.
