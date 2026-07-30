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

The [1.0 release plan](release-1.0-plan.md) and
[1.0 execution runbook](execution-runbook-1.0.md) are the only current release
authorities. Other plans and handoffs are historical engineering records; they
must not be used as current release proof. Verify the live repository and
GitHub state before treating any checklist entry as satisfied.

## Release-status rule

Sigillum currently has no supported stable release, no RC6, and no published
GitHub Release. RC2–RC4 are immutable failure receipts; RC5 is an unpublished
historical draft for `7e04743` only. A tag or draft alone is not release proof.
Valid final release evidence requires the qualified same-SHA RC receipts, an
annotated final tag on protected `main`, exact-byte promotion of the five RC
payloads, a validated evidence bundle, explicit H2 approval, and a published
GitHub Release.
