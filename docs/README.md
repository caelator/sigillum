# Sigillum Documentation

This index separates operator guidance and stable public contracts from planning
records and historical engineering evidence.

## Evaluate and operate

- [Deployment](deployment.md) — local daemon, desktop, gateway preview, build,
  install, and readiness checks.
- [Backup and restore](backup.md) — snapshot contents, recovery, migrations, and
  failure behavior.
- [FIDO2](fido2.md) — hardware-key model, local HID flow, and limitations.
- [Stability policy](stability.md) — stable and unstable 1.0 surfaces.
- [Operator-surface parity](operator-surface-parity.md) — UI, CLI, and API
  coverage matrix.

## Understand the system

- [Architecture](architecture.md) — components, dependency direction, storage,
  unlock, daemon, privacy, and linkage models.
- [Wallet-management roadmap](wallet-management-roadmap.md) — product goal,
  discovery, inventory, consolidation, and future chain direction.
- [Competitive landscape](wallet-competitive-landscape.md) — dated product
  research and strategic context, not a support or release commitment.

## Security and release evidence

- [Security policy](../SECURITY.md) — supported versions, private reporting, and
  threat model.
- [Production-readiness audit](production-readiness-audit.md) — test-backed
  evidence and known limits for the local-only boundary.
- [Production-readiness summary](../PRODUCTION_READINESS.md) — current gate and
  product-completeness summary.
- [1.0 execution runbook](execution-runbook-1.0.md) — release mechanics and
  remaining operator gates.
- [Changelog](../CHANGELOG.md) — changes prepared for the future final release.

## Contributor references

- [Contributing](../CONTRIBUTING.md)
- [Support](../SUPPORT.md)
- [Code of Conduct](../CODE_OF_CONDUCT.md)
- [Refactor notes](refactor-notes.md) — current internal ownership seams.

## Planning records

The [1.0 release plan](release-1.0-plan.md),
[operator-surface and privacy plan](operator-surface-and-privacy-plan.md),
[execution handoff](execution-handoff.md), and
[catch-up plan](catchup-plan.md) are retained as engineering records. The
handoff is the active continuation aid only when its implementation checkpoint
is in the checkout's ancestry and no later implementation commit has superseded
it. Use the execution runbook for current RC truth, and verify live repository
and GitHub state before treating any checklist entry as release proof.

## Release-status rule

Sigillum currently has no supported stable release and no published GitHub
Release. A tag or draft GitHub Release alone is not proof of release. Valid
release evidence requires an
annotated tag on `main`, a passing Release workflow, verified artifacts and
checksums, and a published GitHub Release.
