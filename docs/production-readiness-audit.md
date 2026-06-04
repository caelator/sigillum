# Production Readiness Audit

**Date:** June 4, 2026
**Scope:** local-first, single-host Sigillum source checkout and local-sidecar
gateway boundary
**Verdict:** source release gate passed; broader production completeness remains
bounded by the open gaps below

## Evidence Snapshot

The current release candidate is considered source-ready for the documented
single-host boundary only when `./scripts/check-release.sh` passes. The gate
was added to make the release standard executable instead of scattered across
README, CI, and readiness notes.

The gate covers:

- Cargo metadata resolution
- architecture guardrails and file-size/module-boundary budgets
- daemon UI dependency install, TypeScript typecheck, DOM smoke tests including
  setup-wizard passphrase initialization coverage, and Vite build
- generated daemon UI asset freshness for `app.js` and `styles.css`
- Rust formatting, workspace check, workspace tests, and clippy with warnings
  denied
- real local daemon runtime smoke for first-run status, served UI shell,
  passphrase compartment initialization, lock/unlock, compartment listing, and
  `sigillum doctor`
- vault write/read canaries for connection keys and encrypted secrets before
  and after re-unlock inside the runtime smoke gate
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

The configurable reliability harness is `scripts/check-local-soak.sh`. A bounded
local validation run passed with `SIGILLUM_SOAK_SECONDS=20` and five full
iterations. Production-style evidence should run the same harness for a longer
target-host window, for example with `SIGILLUM_SOAK_SECONDS=3600`.

`cargo deny check` currently emits duplicate-version warnings and exits
successfully with advisories, bans, licenses, and sources all accepted.

## Requirement Audit

| Requirement | Current evidence | Status |
| --- | --- | --- |
| Build and dependency graph resolve | `cargo metadata --no-deps --format-version 1` inside `./scripts/check-release.sh` | Proven for current checkout |
| Architecture stays within professional boundaries | `./scripts/check-architecture.sh` inside the release gate | Proven for current checkout |
| Daemon UI compiles and tested source matches generated assets | `npm ci`, `npm run typecheck`, `npm test`, `npm run build`, plus generated asset freshness in the release gate | Proven for current checkout |
| Rust workspace builds, tests, and lints | `cargo fmt --all --check`, `cargo check --workspace`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` inside the release gate | Proven for current checkout |
| Security and supply-chain baseline | `cargo audit` and `cargo deny check` inside the release gate | Proven for current checkout, with accepted duplicate dependency warnings |
| Local daemon and gateway loopback integration behavior | Workspace integration tests pass outside the sandbox | Proven for current checkout in an unsandboxed local environment |
| Target-host operational readiness | `sigillum doctor` passed in `scripts/check-runtime-smoke.sh` for first-run and unlocked temporary daemon states | Proven for repeatable isolated local proof; each target host still needs its own doctor result |
| Runtime daemon lifecycle behavior | `scripts/check-runtime-smoke.sh` starts the daemon, verifies status, initializes a passphrase compartment, writes and reads vault canaries, locks, unlocks, lists compartments, and runs doctor | Proven for current checkout in an unsandboxed local environment |
| Runtime browser/UI visual behavior | DOM smoke tests pass, the runtime smoke checks the served UI shell, and a browser completed setup, unlocked operator workspace, vault canary write/read, browser-session logout, passphrase re-authentication, and post-auth canary count checks against an isolated local daemon | Proven for current checkout as a manual in-app browser proof; repeatable browser automation remains useful |
| Long-duration reliability | Recovery and crash tests pass, and `scripts/check-local-soak.sh` passed a bounded local daemon/gateway run | Harness proven for current checkout; longer target-host soak evidence still needed |
| External security assurance | Code gates, audit, deny, and SSRF/local-boundary tests pass | Not fully proven; no external penetration test or broad fuzzing campaign yet |
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

## Remaining Work Before A Broader Completion Claim

The active product objective remains larger than the current release gate. To
claim Sigillum is fully operational and production ready without qualification,
the project still needs:

1. repeatable browser automation for the initialized and unlocked operator
   workflows, not only DOM-module tests, served-shell smoke, and the current
   manual in-app browser proof
2. target-host `sigillum doctor` results for any real host being called ready
3. a long-duration target-host daemon and gateway soak run, beyond the bounded
   local harness validation
4. a documented fuzzing or adversarial test pass across the daemon API,
   gateway, and UI boundary
5. a decision on whether open wallet-management roadmap items are required for
   the specific release scope or explicitly deferred

Until those are complete, the accurate claim is narrower: the current source
checkout passes the local-first release gate, and the docs identify the
remaining assurance and product-completeness work.
