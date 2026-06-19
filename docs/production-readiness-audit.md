# Production Readiness Audit

**Date:** June 4, 2026 (updated June 19, 2026)
**Scope:** local-first, single-host Sigillum source checkout and local-sidecar
gateway boundary
**Verdict:** source release gate passed for Sigillum Local-First Operator
Console v1; broader production completeness remains bounded by the open gaps
below

## Evidence Snapshot

The current release candidate is considered source-ready for the documented
single-host boundary only when `./scripts/check-release.sh` passes. The gate
was added to make the release standard executable instead of scattered across
README, CI, and readiness notes.

The gate covers:

- Cargo metadata resolution
- architecture guardrails and file-size/module-boundary budgets
- daemon UI dependency install, TypeScript typecheck, DOM smoke tests including
  setup-wizard passphrase initialization coverage, npm high-severity advisory
  audit, and Vite build
- generated daemon UI asset freshness for `app.js` and `styles.css`
- Rust formatting, workspace check, workspace tests, and clippy with warnings
  denied
- local adversarial/fuzz pass through `scripts/check-adversarial.sh`, covering
  core property tests, daemon HTTP boundary rejection cases, CLI adversarial
  smoke, gateway security/integration checks, and daemon UI DOM boundary tests
- real local daemon runtime smoke for first-run status, served UI shell,
  passphrase compartment initialization, lock/unlock, compartment listing, and
  `sigillum doctor`
- vault write/read canaries for connection keys and encrypted secrets before
  and after re-unlock inside the runtime smoke gate
- repeatable headless-browser smoke (`scripts/check-browser-smoke.sh`) driving
  a real local daemon through setup-wizard passphrase initialization, unlocked
  operator workspace, canary write/reveal, browser-session logout, and
  passphrase re-authentication, failing on any browser console or runtime error
- configurable local daemon/gateway soak harness for repeated daemon status,
  vault write/read canaries, gateway health, and `sigillum doctor`
- RustSec advisory scan through `cargo audit`
- supply-chain policy through `cargo deny check`
- final whitespace validation through `git diff --check`

The first sandboxed run failed when loopback integration tests could not bind
local sockets under the execution sandbox. The same release gate passed outside
the sandbox, which is the relevant environment for the local daemon and gateway
integration tests.

Runtime proof also exposed and fixed a startup readiness bug: a fresh daemon base
directory could be created with default process permissions before the host
doctor checked it. The daemon now creates or repairs its base directory to
`0700` on Unix before runtime state is opened, and a regression test covers both
first creation and permission repair.

After that fix, an isolated daemon started with
`SIGILLUM_BASE_DIR=/private/tmp/sigillum-runtime-proof-fixed-20260604` created a
`0700` base directory, served `/api/status`, passed `sigillum doctor` with no
blocking local-readiness failures, and rendered the first-run browser UI at
`http://127.0.0.1:18743` with the expected `Sigillum Vault`, `NO VAULT`, setup
state, action controls, and live refresh metadata.

The repeatable version of that proof is now `scripts/check-runtime-smoke.sh`.
It starts a real daemon on a temporary local base directory, verifies first-run
status, checks the served UI shell, initializes a passphrase compartment,
verifies initialized/unlocked state and compartment listing, stores and reads
connection-key and encrypted-secret canaries, locks the daemon, unlocks it
again, reads the canaries again, and runs `sigillum doctor` against first-run
and unlocked states.

A browser-level proof was also run against an isolated daemon on
`http://127.0.0.1:19943`: the in-app browser completed passphrase setup through
the setup wizard, reached the unlocked operator workspace, stored and revealed
one connection-key canary and one encrypted-secret canary, logged out the browser
session into the locked UI, re-unlocked through the passphrase form, and showed
the same canary counts after re-authentication.

That browser-level proof is now repeatable as `scripts/check-browser-smoke.sh`,
wired into the release gate after the runtime smoke. It starts an isolated
daemon, drives a headless Chromium-family browser through the same setup,
vault-canary, logout, and re-authentication workflow over the Chrome DevTools
Protocol, captures a screenshot and DOM snapshot on failure, and can be skipped
on hosts without a local browser via `SIGILLUM_SKIP_BROWSER_SMOKE=1`.

The configurable reliability harness is `scripts/check-local-soak.sh`. A bounded
local validation run passed with `SIGILLUM_SOAK_SECONDS=300`,
`SIGILLUM_SOAK_INTERVAL_SECONDS=10`, and 28 full iterations. A longer
target-host run also passed on `mac-server` with
`SIGILLUM_SOAK_SECONDS=3600`, `SIGILLUM_SOAK_INTERVAL_SECONDS=30`, and
`SIGILLUM_SOAK_RECEIPT=target/readiness/local-soak-3600-d1fd325.json`.
The receipt records a clean `main` checkout at
`d1fd32570f461c77f47c736a011295cd49d70cc4`, Darwin `25.5.0`, 117 iterations,
117 `sigillum doctor` runs, the daemon and gateway loopback URLs, and the
checked surfaces: daemon status, vault API-key write/read canary, gateway
health, and `sigillum doctor`.

The local adversarial pass is now executable as `scripts/check-adversarial.sh`
and wired into the release gate after the full workspace tests. It runs:

- `cargo test -p sigillum-core --test fuzz_boundaries` with 256 proptest cases
  by default, configurable through `SIGILLUM_ADVERSARIAL_PROPTEST_CASES`
- `cargo test -p sigillum-daemon --test adversarial_api`, covering malformed
  JSON, unexpected content types, empty bodies, missing or malformed bearer
  tokens, invalid compartment setup values, and bad EVM address/hex inputs at
  the HTTP route boundary
- `cargo test -p sigillum-cli --test cli_smoke`, including adversarial command
  parsing and `sigillum doctor` no-daemon behavior
- `cargo test -p sigillum-gateway --test gateway_tests` for auth hashing,
  HMAC signing, constant-time comparison, amount/address validation, and SSRF
  URL rejection cases
- `cargo test -p sigillum-gateway --test gateway_integration` for local-sidecar
  auth, idempotency, scope, rate-limit, rollback, and daemon-side-effect
  boundaries
- `npm --prefix crates/sigillum-daemon/ui test` for typed DOM boundary tests,
  stale-token clearing, setup/unlock/logout flows, dispatcher argument coercion,
  invalid value formatting, wallet/treasury forms, and self-check rendering

This is a local adversarial/fuzz pass for the current single-host product
boundary. It is not a replacement for an independent penetration test, and it
does not claim internet-facing or hosted-service assurance.

`cargo deny check` currently emits duplicate-version warnings and exits
successfully with advisories, bans, licenses, and sources all accepted.

## Requirement Audit

| Requirement | Current evidence | Status |
| --- | --- | --- |
| Build and dependency graph resolve | `cargo metadata --no-deps --format-version 1` inside `./scripts/check-release.sh` | Proven for current checkout |
| Architecture stays within professional boundaries | `./scripts/check-architecture.sh` inside the release gate | Proven for current checkout |
| Daemon UI compiles and tested source matches generated assets | `npm ci`, `npm audit --audit-level=high`, `npm run typecheck`, `npm test`, `npm run build`, plus generated asset freshness in the release gate | Proven for current checkout |
| Rust workspace builds, tests, and lints | `cargo fmt --all --check`, `cargo check --workspace`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` inside the release gate | Proven for current checkout |
| Security and supply-chain baseline | `cargo audit` and `cargo deny check` inside the release gate | Proven for current checkout, with accepted duplicate dependency warnings |
| Local daemon and gateway loopback integration behavior | Workspace integration tests pass outside the sandbox | Proven for current checkout in an unsandboxed local environment |
| Target-host operational readiness | `sigillum doctor` passed in `scripts/check-runtime-smoke.sh` for first-run and unlocked temporary daemon states, and the `mac-server` 3600-second soak receipt recorded 117 doctor runs on a clean checkout | Proven for `mac-server`; each additional target host still needs its own doctor/soak receipt |
| Runtime daemon lifecycle behavior | `scripts/check-runtime-smoke.sh` starts the daemon, verifies status, initializes a passphrase compartment, writes and reads vault canaries, locks, unlocks, lists compartments, and runs doctor | Proven for current checkout in an unsandboxed local environment |
| Runtime browser/UI visual behavior | DOM smoke tests pass, the runtime smoke checks the served UI shell, and `scripts/check-browser-smoke.sh` repeatably drives a headless browser through setup, unlocked operator workspace, vault canary write/reveal, browser-session logout, passphrase re-authentication, and post-auth canary count checks against an isolated local daemon inside the release gate | Proven for current checkout as repeatable automation in an unsandboxed local environment with a Chromium-family browser |
| Long-duration reliability | Recovery and crash tests pass, `scripts/check-local-soak.sh` passed a bounded 300-second local daemon/gateway run with 28 iterations, and a `mac-server` target-host run passed a 3600-second target with 117 iterations | Proven for the current local-first target host; broader host coverage and chaos testing remain future assurance work |
| External security assurance | Code gates, audit, deny, local adversarial/fuzz gate, SSRF/local-boundary tests, and UI boundary tests pass | Local boundary pass proven for current checkout; independent external penetration test not performed |
| Full wallet-management product roadmap | Existing docs and tests cover current local wallet, inventory, risk, and plan slices | Not complete; deeper discovery, DeFi/NFT metadata, broader non-EVM support, and richer consolidation execution remain roadmap work |

## Release Boundary

The release gate proves the source tree is coherent for Sigillum's current
documented product boundary:

- one local operator machine
- local daemon with bearer session-token auth
- local vault files and daemon-held unlock state
- optional local gateway sidecar inside the same trust boundary
- no hosted backend and no remote multi-tenant service claim

The gate does not prove internet-facing readiness, hosted service safety,
multi-host coordination, or arbitrary wallet discovery completeness.

The explicit release scope is **Sigillum Local-First Operator Console v1**. It
includes the local daemon, vault, CLI, session model, current wallet/inventory
slices, and local-sidecar gateway preview. Ethereum receive-branch xpub profiles
now support imported external watch-only public branches through
`external_receive_xpub`, and Ethereum account-level xpub profiles support
`external_account_xpub` normalized into receive branches; those profiles are
non-executable and can feed the existing inventory gap scanner without local
signing material. The comprehensive wallet-management roadmap is intentionally
deferred from this readiness claim: arbitrary custom-path xpub discovery, rich
NFT/DeFi/airdrop inventory, non-EVM chains, automated consolidation execution,
and hosted or internet-facing wallet operations remain future product work.

## Remaining Work Before A Broader Completion Claim

The active product objective remains larger than the current release gate. To
claim Sigillum is fully operational and production ready without qualification,
the project still needs:

1. a target-host `sigillum doctor` and soak receipt for each additional host
   being called ready beyond `mac-server`
2. an independent external penetration test if the claim expands beyond
   source-verified local-first readiness

Until those are complete, the accurate claim is narrower: the current source
checkout passes the local-first release gate, and the docs identify the
remaining assurance and product-completeness work.
