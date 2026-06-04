# Contributing to Sigillum

## Getting Started

```bash
git clone https://github.com/caelator/sigillum.git
cd sigillum
cargo build
cargo test
```

## Development

### Workspace Structure

Sigillum is a Cargo workspace with local vault, API, daemon, client, CLI, SDK,
FIDO2, generator, server, gateway, and meta crates. Changes to
`sigillum-core` affect everything downstream.

```
crates/
├── sigillum-core      ← Start here. Traits and file vault.
├── sigillum-daemon    ← Axum server, routes, web UI
├── sigillum-api       ← Shared daemon/client request and response contracts
├── sigillum-client    ← Daemon API client
├── sigillum-fido2     ← Hardware key integration
├── sigillum-cli       ← Terminal interface
├── sigillum-sdk       ← Embeddable SDK
├── sigillum-server    ← Server library
├── sigillum-gateway   ← Local-sidecar payment preview gateway
└── sigillum           ← Meta-crate (re-exports core)
```

### Running Tests

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p sigillum-core

# With logging
RUST_LOG=debug cargo test --workspace -- --nocapture
```

### Code Quality

```bash
# Full release gate
./scripts/check-release.sh

# Lint
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Check for security advisories
cargo audit
```

## Pull Requests

1. Fork the repo and create a branch from `main`.
2. If you've added code, add tests.
3. Ensure `./scripts/check-release.sh` passes.
4. Open a PR with a clear description of the change.

### Commit Messages

Use imperative mood. Be specific about what changed and why.

```
Good:  Add Argon2id passphrase unlock to FileVault
Bad:   Updated vault stuff
```

### What We're Looking For

- Bug fixes with regression tests
- Performance improvements with benchmarks
- New vault backends (database-backed, cloud KMS, HSM)
- Platform support (systemd service files, launchd plists)
- Documentation improvements

### What We're Not Looking For

- Breaking changes to `SecretStore` or `VaultLifecycle` traits without RFC
- Additional dependencies without justification
- Features that weaken the security model

## Security

Found a vulnerability? **Do not open a public issue.** See [SECURITY.md](SECURITY.md) for reporting instructions.

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project: MIT OR Apache-2.0.
