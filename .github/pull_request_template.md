# Pull request

## Summary

Describe the operator problem and the resulting behavior.

## Safety and boundaries

- [ ] Uses only synthetic or fully redacted test data.
- [ ] Preserves the local-only trust boundary, or includes an accepted design
  for changing it.
- [ ] Reviews security, privacy, signing, persistence, and recovery implications.
- [ ] Adds failure-path tests for security-sensitive behavior.

## Verification

List the exact checks and manual flows run.

- [ ] `cargo fmt --all --check`
- [ ] Relevant tests pass with `--locked`
- [ ] `./scripts/check-release.sh`, or the reason it was not run

## Documentation and compatibility

- [ ] Public docs match the new behavior.
- [ ] State/schema migration impact is documented and tested, or not applicable.
- [ ] Known limitations and rollback behavior are stated.
