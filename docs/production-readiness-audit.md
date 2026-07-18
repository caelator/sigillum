# Production Readiness Audit

**Date:** June 4, 2026 (updated July 18, 2026)
**Scope:** local-first, single-host Sigillum source checkout and local-sidecar
gateway boundary
**Verdict:** RC5 is a valid but incomplete draft candidate for protected-main
commit `7e047438f6305ef1cedecdf4790e1b0e1d7e1e6e`. Remote annotated tag object
`c726ba9` peels to that commit; release workflow `29248938476` passed all six
jobs, and the five payload assets independently match the draft's
`SHA256SUMS`. Standard/chaos F4 and doctor receipts bind to the same SHA. RC5
still lacks F6 public-testnet receipts, desktop clean-install evidence, UI
sign-off, and a complete sanitized evidence bundle, so it cannot promote a
final release.

The operator-surface branch changes code after RC5 and includes protected main
through merge `3b647f8`; its committed keyboard/accessibility checkpoint is
`29426df`. At historical interaction/browser checkpoint `c435611`, UI tests pass
225/225, typecheck and build pass, axe passes 15/15, screenshots pass 12/12,
and real-daemon browser smoke passes. Current implementation checkpoint
`8ea6f8e` restores the architecture, formatting, and strict-clippy gates.
Audited documentation/release-gate checkpoint
`fc1e93b1aa2cf9524b2b99fd04342863ba6b2b1d` passed the complete clean-tree
release gate. RC5 evidence is historical baseline for that feature line. After
protected-main integration the next eligible candidate is RC6, and every
release receipt must bind to RC6's peeled SHA. No final `v1.0.0` tag or published
GitHub Release exists. RC2–RC4 remain immutable failed-contract evidence.
Gateway payments remain disabled-by-default experimental observations, not
supported 1.0 confirmation semantics.

RC4 release run `29230844456` nevertheless completed all six jobs, including
strict source and mounted-dmg verification, and produced its six expected draft
assets. That is retained as positive evidence for the signing remediation and
negative evidence that a green workflow cannot override a defective assurance
contract. RC5 run `29248938476` is the first succeeding candidate under the
remediated tag, packaging, runtime, and F6 schema-v2 contracts, but operator
evidence remains incomplete as described above.

## Evidence Snapshot

Audited checkpoint `fc1e93b` is source-ready for the documented single-host
boundary: `./scripts/check-release.sh` ran from `2026-07-18T17:15:16Z` through
`2026-07-18T17:30:35Z` (919 seconds), did not mutate tracked files, and recorded
`gate_rc=0`, `tee_rc=0`, `filter_rc=0`, and `overall_rc=0`. The operator-local
external receipt ID is `20260718T171516Z-fc1e93b1aa2c`, and its log SHA-256 is
`efb8e2240949d32f0f53ff8ee028d1016724df849e038b361b9fed78f23dab94`.
The gate makes the release standard executable instead of scattering it across
README, CI, and readiness notes. This local receipt proves `fc1e93b` only; any
later documentation-only successor requires an external exact-HEAD clean-gate
receipt before protected-main integration, without another documentation edit
solely to record that receipt.

The gate covers:

- Cargo metadata resolution
- architecture guardrails and file-size/module-boundary budgets
- daemon UI dependency install, TypeScript typecheck, DOM smoke tests including
  setup-wizard passphrase initialization coverage, npm high-severity advisory
  audit, and Vite build
- generated daemon UI asset freshness for `app.js` and `styles.css`
- pinned axe-core `4.12.1` accessibility scans across 15 strict-mock setup,
  locked, destination, and routed-subview scenarios; violations, incomplete
  checks, stale bundles, missing scenarios, browser exceptions, or unknown
  mock routes fail closed
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
- real-daemon headless-browser smoke (`scripts/check-browser-smoke.sh`). The
  historical flow covered setup, Vault canaries, logout, and reauthentication,
  and implementation checkpoint `c435611` migrates it to visible setup, all five
  destination controllers, current Vault/palette/modal/focus behavior, logout,
  reauthentication, and persistence. The migrated harness passes end to end
  against an isolated real daemon; its startup timeout now defaults to a
  configurable 120 seconds for cold Cargo builds
- focused events-client tests prove that same-tab session revoke or token
  rotation closes the previously authenticated SSE stream and reconnects only
  with the current token, preventing stale `EventSource` authorization from
  surviving the browser session lifecycle
- desktop compile and bundle smoke through `scripts/check-desktop.sh`, which
  always builds `sigillum-desktop` and, on macOS, runs a debug Tauri bundle
  build through the fail-closed signing wrapper, asserts exactly one `.app`
  and `.dmg`, exercises the release notice-resource overlay, and strictly
  verifies the source app plus its mounted-dmg copy:
  exact identifier/executable, bound plist, sealed resources, nonempty
  `CodeResources`, expected signature mode, and matching CDHash
- configurable local daemon/gateway soak harness for repeated daemon status,
  vault write/read canaries, gateway health, and `sigillum doctor`
- RustSec advisory scan through `cargo audit`
- supply-chain policy through `cargo deny check`
- final whitespace validation through `git diff --check`

The release-tag contract reads the authoritative remote annotated tag object
and peeled commit, binds both to the triggering SHA, fetches a missing tag
object only into a non-tag scratch ref, requires RC numbers to advance the
retained remote sequence by exactly one, and pins the initial tag-object ID into
the final draft-release job. Pushed RC tags are permanent receipt anchors, and
the RC draft/assets remain available through final-draft verification. A final
`v1.0.0` tag must peel to the identical commit as the RC receipts, or those
receipts are void and a new RC is required. Final assets receive their own
checksum verification before publication; only then may the older RC draft be
removed.

The historical `v1.0.0-rc.1` rehearsal tag was deleted under the superseded
cleanup procedure. That number remains burned and is not recreated. It is the
only permitted legacy gap; the retained 1.0.0 RC sequence begins at `rc.2` and
must remain contiguous thereafter.

`v1.0.0-rc.3` is retained as packaging-failure evidence. Its checksum-valid
draft assets contained an app for which `codesign -dv` misleadingly printed
`Signature=adhoc`, while `codesign --verify --deep --strict` failed because the
bundle had no resource seal. The new verifier rejects that exact linker-only
shape, mounts the dmg read-only without executing its binary, and requires the
mounted app to match the already verified source app. RC5 is the retained
successor to the separate RC4 evidence-contract failure. Because the current
feature line changes code after RC5, its next permitted candidate is
`v1.0.0-rc.6` after protected-main integration.

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

That historical browser-level proof was automated as
`scripts/check-browser-smoke.sh` and wired into the release gate after runtime
smoke. The five-controller migration subsequently detached the legacy setup and
Vault nodes used by the script. The rewrite committed at `c435611` now
targets the five controllers and new interaction contracts and passes end to
end against an isolated daemon. The harness still captures
a screenshot and DOM
snapshot on failure and may be skipped on hosts without a browser via
`SIGILLUM_SKIP_BROWSER_SMOKE=1`; a skip is not positive browser evidence.

The desktop bundle proof is now repeatable as `scripts/check-desktop.sh`, wired
into the release gate after the browser smoke. It always compiles
`sigillum-desktop` with the locked dependency graph. On macOS, it also runs
the project Tauri wrapper with explicit ad-hoc signing when credentials are
absent, exercises the release notice-resource overlay, requires exactly one
debug `.app` and `.dmg`, verifies the source and
mounted apps with `codesign --verify --deep --strict`, validates signature
metadata, hardened runtime, and CDHash parity, and runs negative bundle/dmg
regressions including a CDHash mismatch. Developer ID mode also requires the
dmg signature team to match the app. Partial,
mixed, or whitespace-only Apple credential configurations fail closed. The
Developer ID path also requires one complete notarization family and validates
stapled tickets on the source app, mounted app, and dmg. Mode-independent
hostile dmg-layout regressions run in the always-on ad-hoc suite. Because the
pinned Tauri bundler notarizes the app before it creates and signs the dmg, the
project wrapper performs the dmg submission/stapling step explicitly and an
offline fake-tool regression checks both credential families and failure
states. The GitHub release workflow uses the Apple-ID family; API-key
notarization remains a local/manual wrapper path. The
macOS bundle portion can be skipped with `SIGILLUM_SKIP_DESKTOP_BUNDLE=1` only
outside CI when the host cannot build Tauri bundles; non-macOS hosts print an
explicit bundle-skip line after the compile check.

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

Chaos mode is enabled with `SIGILLUM_SOAK_CHAOS=1`; it `kill -9`s the
harness's own daemon every `SIGILLUM_SOAK_CHAOS_EVERY` iterations, defaulting
to 10. A kill cycle is counted only after the next full iteration's doctor and
canary checks pass against the restarted daemon, and the receipt records those
cycles in its additive `chaos` object. On the first cycle, the harness also runs
the bounded in-flight plan-step assertion
`cargo test -p sigillum-daemon --test execution_semantics chaos_kill_in_flight`,
reusing the W7.4 crash-resume mock-RPC machinery, and records that assertion's
result in the receipt.

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
- `cargo test -p sigillum-gateway --test gateway_tests --locked` for auth hashing,
  HMAC signing, constant-time comparison, amount/address validation, and SSRF
  URL rejection cases
- `cargo test -p sigillum-gateway --test gateway_integration --locked` for local-sidecar
  auth, idempotency, scope, rate-limit, rollback, and daemon-side-effect
  boundaries
- `npm --prefix crates/sigillum-daemon/ui test` for typed DOM boundary tests,
  token-bound `401` clearing, immediate same-tab locked-shell policy, stale FIDO
  detection suppression, dynamic-modal isolation, setup/unlock/logout flows,
  dispatcher argument coercion, invalid value formatting, wallet/treasury
  forms, and self-check rendering

This is a local adversarial/fuzz pass for the current single-host product
boundary. It is not a replacement for an independent penetration test, and it
does not claim internet-facing or hosted-service assurance.

`cargo deny check` currently emits duplicate-version warnings and exits
successfully with advisories, bans, licenses, and sources all accepted.

No RustSec advisories are currently ignored. The former temporary
exceptions for RUSTSEC-2026-0194 and RUSTSEC-2026-0195 (quick-xml 0.39.x
DoS advisories via `plist` ← `tauri`) were removed on 2026-07-08 after
`cargo update` moved the workspace to quick-xml 0.41.0, which is outside
the advisories' vulnerable range; `cargo audit` and `cargo deny check`
pass with empty ignore lists.

## Requirement Audit

| Requirement | Current evidence | Status |
| --- | --- | --- |
| Build and dependency graph resolve | `cargo metadata --locked --no-deps --format-version 1` inside `./scripts/check-release.sh` | Passed without tracked-tree mutation at `fc1e93b`; an exact-HEAD external receipt is required for any successor |
| Architecture stays within professional boundaries | `./scripts/check-architecture.sh` inside the release gate | Passed inside the complete clean-tree gate at `fc1e93b` |
| Daemon UI compiles and tested source matches generated assets | `npm ci`, `npm audit --audit-level=high`, `npm run typecheck`, `npm test`, `npm run build`, generated-asset freshness, and pinned 15-scenario axe-core checks | Historical focused evidence is green at `c435611`; the complete UI source gate passed at `fc1e93b` |
| Rust workspace builds, tests, and lints | Locked workspace check/test/clippy plus independent no-HID FIDO2 check/test/clippy inside the release gate | Passed inside the complete clean-tree gate at `fc1e93b`, including independent no-HID coverage |
| Security and supply-chain baseline | `cargo audit --file Cargo.lock` and `cargo deny --locked check` inside the release gate | Passed for the `fc1e93b` lockfile, with accepted duplicate dependency warnings |
| Release identity and monotonicity | Remote direct/peeled tag validation, event-SHA and cross-job tag-object binding, scratch-ref recovery, retained monotonically numbered RC tags, and draft-only release creation | Proven only when the tag workflow passes at the gated `main` SHA; branch/tag protection is a fail-closed pre-merge settings check |
| Local daemon and gateway loopback integration behavior | Workspace integration tests pass outside the sandbox | Passed inside the complete local gate at `fc1e93b`; protected-main CI and RC6 evidence remain |
| Target-host operational readiness | RC5 standard soak, chaos soak, and doctor receipts bind to `7e04743` | Proven for RC5 only; repeat all required host evidence at RC6 because the feature line changes code |
| Runtime daemon lifecycle behavior | `scripts/check-runtime-smoke.sh` starts the daemon, verifies status, initializes a passphrase compartment, writes and reads vault canaries, locks, unlocks, lists compartments, and runs doctor | Passed inside the complete local gate at `fc1e93b`; supported-host doctor and RC6 receipts remain |
| Queue submission durability, dependency finality, and pause | Queue schema v5 persists `prepared` raw bytes/hash and a pre-RPC `submitted_unknown` marker; recovery checks receipts or resubmits exact bytes without re-signing; dependent plan steps remain unsigned until every prerequisite reaches receipt-confirmed finality; the real HTTP pause regression latches before the active drain mutex and blocks later broadcasts | Passed inside the complete local gate at `fc1e93b`; protected-main CI and RC6 receipts remain |
| Runtime browser/UI visual behavior | DOM tests, the strict 15-scenario mock accessibility gate, 12-shot walkthrough, and migrated real-daemon browser smoke pass across visible setup, all five controllers, Vault persistence, modal/palette safety, focus, logout, and reauthentication | Passed inside the complete local gate at `fc1e93b`; manual screenshot/operator sign-off remains |
| Desktop app bundle readiness | Source/mounted-dmg verification runs inside `./scripts/check-release.sh`; RC5 workflow `29248938476` produced checksum-valid draft assets | The local source bundle gate passed at `fc1e93b`, but clean-machine installation and fresh RC6 workflow/assets remain unproved |
| Long-duration reliability | Recovery/crash tests plus RC5 standard 3600-second and chaos 600-second receipts passed at `7e04743` | Historical for the feature line; standard plus chaos receipts on each supported host must bind to RC6 |
| External security assurance | Code gates, audit, deny, local adversarial/fuzz gate, SSRF/local-boundary tests, and UI boundary tests are part of the release contract | The local source contract passed at `fc1e93b`; no independent external penetration test has been performed |
| Full wallet-management product roadmap | EVM roadmap phases 1-9 shipped and tested: discovery, inventory, risk, planning, policy-gated fail-closed execution (default off), DeFi exit adapters, and treasury automation | EVM scope is complete except swap execution, which is deferred per D-13; only non-EVM chains (phase 10), swap execution (D-13), and fiat/NFT valuation (D-16) remain deferred |

## Release Boundary

The release gate proves the source tree is coherent for Sigillum's current
documented product boundary:

- one local operator machine
- local daemon with bearer session-token auth
- local vault files and daemon-held unlock state
- optional local gateway sidecar inside the same trust boundary; experimental
  payment creation is disabled by default and balance observations are not
  payment confirmations
- no hosted backend and no remote multi-tenant service claim

The gate does not prove internet-facing readiness, hosted service safety,
multi-host coordination, or arbitrary wallet discovery completeness.

The explicit release scope is **Sigillum Local-First Operator Console v1**. It
includes the local daemon, vault, CLI, session model, current wallet/inventory
slices, and local-sidecar gateway preview. Gateway payments remain outside
supported 1.0 confirmation semantics: opt-in creation emits observations only,
reports `latest_balance_observation_at`, and exposes no privileged third-party
invoice-signing callback.
Ethereum receive-branch xpub profiles
now support imported external watch-only public branches through
`external_receive_xpub`, and Ethereum account-level xpub profiles support
`external_account_xpub` normalized into receive branches; those profiles are
non-executable and can feed the existing inventory gap scanner without local
signing material. Imported receive xpubs can optionally carry a custom
`external_receive_path`; Sigillum validates the BIP-32 syntax and xpub depth,
uses the path in inventory/export/self-check evidence, and warns that the path
is operator-asserted metadata. Imported account-level xpubs can likewise carry
`external_account_path`; Sigillum validates the terminal hardened account path,
derives the receive branch locally, uses the supplied path in inventory/export/
self-check evidence, and keeps the wallet watch-only. The EVM wallet-management
product described by roadmap phases 1-9 has shipped, including NFT/DeFi/airdrop
inventory and consolidation-plan execution. That execution is a policy-gated,
fail-closed opt-in and defaults off. Only non-EVM chains (roadmap phase 10), swap
execution (D-13), fiat/NFT valuation (D-16), and hosted or internet-facing
wallet operations remain deferred.

## Remaining Work Before A Broader Completion Claim

The current feature line still needs the following release-qualified evidence:

1. for any documentation-only successor to `fc1e93b`, an external clean-gate
   receipt at that exact HEAD before integration through protected `main`; then
   required Ubuntu/macOS CI, a successful RC6 draft workflow, and independently
   checksum-verified assets at one exact SHA
2. standard and chaos soak plus `sigillum doctor` receipts on every supported
   host at that RC6 SHA (F4)
3. five public-testnet transactions for the four core execution families:
   native sweep, ERC-20 sweep, revoke, and both legs of `fund_gas` → dependent
   sweep on Ethereum Sepolia (`11155111`) plus one supported L2 testnet: Base
   Sepolia (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`)
   (F6)
4. a checksum-verified clean desktop install reaching unlock without a
   developer toolchain, manual UI walkthrough/sign-off, and a complete
   sanitized external evidence bundle bound to the same RC6 SHA

An independent external penetration test is additionally required only if the
assurance claim expands beyond the source-verified local-first boundary. No
external penetration test has been performed, and this release does not claim
one (D-4).

Until those are complete, the accurate claim is narrower: implementation
checkpoint `8ea6f8e` and audited documentation/release-gate checkpoint
`fc1e93b` pass the local source contract, while protected-main/RC6 CI, host,
testnet, clean-install, operator-sign-off, and same-candidate evidence receipts
remain. These docs identify the remaining assurance and product-completeness
work.

## Execution-path security review (F5)

A focused adversarial review of the W7 queue-execution surface
(`service/queue/{gates,pause,processing,broadcast,plan_steps,plan_steps/signing}.rs`,
`service/inventory/plan_execution_enqueue.rs`, `service/inventory/treasury.rs`,
`service/inventory/planner.rs`) performed before this surface is enabled in any
receipt. Every execution capability defaults OFF and is opt-in per
`TreasuryPolicy` (§0.1.8). The trust boundary under review is a single local
operator machine with an unlocked compartment; an attacker with arbitrary write
access to the base directory already possesses capability strictly greater than
the queue surface grants (they hold the vault and unlock state), so the model
here is partial/uncoordinated tampering and stolen-session-token abuse, not
full local root. Each of the five plan-named threat cases has a written
disposition and, where testable, a named regression test. All dispositions were
verified against the built code, not the design intent.

### Threat 1 — Malicious plan JSON injected into state files

**Finding.** `wallet_inventory.json` is loaded through the schema-versioned
`json_store` path. Structurally corrupt or wrong-schema JSON is rejected on read
and, when a good `.bak` exists, quarantined to a timestamped `.corrupt-*` file
with the last-good state atomically restored; well-formed-but-hostile JSON
(tampered amounts, injected steps, stripped evidence) deserializes cleanly and
loads. **Exploitability.** Requires base-dir write access (already-compromised
host); the injection adds no capability beyond the vault the attacker would
already hold, and load never *executes* injected content. **Mitigation.** Every
fund-controlling field is re-validated server-side at enqueue against CURRENT
state — approval, blockers, cross-party linkage, simulation pass + W6.2
freshness, W7.1 gates, destination allowlist, step/plan native caps, claim gate,
gas-topup opt-in, dependency ordering — and again at drain, where the evidence
hash is recomputed from live state before any key material is touched. A step
whose destination is tampered to a non-allowlisted address is refused
(`block_destination`); a step whose simulation evidence is stripped is refused
fail-closed (missing `simulated_at_unix` is treated as stale). **Residual
risk.** An attacker who consistently rewrites BOTH `wallet_inventory.json` AND
`queue.json` (recomputing a valid evidence hash) while the compartment is
unlocked and gates are ON can enqueue-and-drain a hostile step — but that is
strictly weaker than directly using the already-unlocked vault, and is the
documented local-compromise boundary. **Tests.**
`hostile_inventory_tampered_destination_refused_at_enqueue`,
`hostile_inventory_garbage_evidence_refused_at_enqueue`,
`corrupt_inventory_is_quarantined_and_good_state_restored`
(`tests/plan_execution.rs`).

### Threat 2 — Calldata tampering between preflight and execution

**Finding.** The W7.2/W7.3 tamper detector is a SHA-256 commitment
(`plan_step_evidence_hash_hex_parts`) over a fixed-order set of `key=value\n`
lines binding plan/step identity, source, derivation path, wallet/provider
profile, asset, amount, destination, and the prepared call
(`call_target_address`, `call_data_hex`, `call_value_wei_hex`, label) plus the
lexicographically sorted simulation evidence. At drain the hash is recomputed
from the live step (authoritative for step fields) and the job's own stored call
fields (authoritative for the calldata that will actually be broadcast); any
mismatch, or a missing plan/step, parks the job `operator_action_required` and
never signs. **Exploitability.** Redirecting funds requires changing a
committed field (e.g. `call_target_address`) without changing the digest — a
SHA-256 preimage/collision. **Mitigation review.** Canonicalization was probed
for field omission, ordering, delimiter injection, and type coercion: every
field is emitted with its own fixed `key=` prefix and `\n` terminator, so no
value can be omitted or merged into an adjacent field — a crafted value
containing an embedded `\nkey=` always ADDS a line rather than replacing one,
changing the digest. Evidence entries carry a constant `evidence=` key and are
sorted, so reordering is inert and injection cannot masquerade as another field.
**Residual risk.** The only identified structural collisions are semantically
inert: `None` and `Some("")` for the optional `asset_address`,
`destination_address`, and `call_value_wei_hex` fields hash identically, but all
decode to the same absent/zero semantics, so no value or routing change hides
behind them. Coordinated dual-file rewrite falls under Threat 1's residual
boundary. **Tests.** New `evidence_hash_resists_field_delimiter_injection`
(unit, `plan_execution_enqueue.rs`) extends the existing
`evidence_hash_detects_tampered_call_and_evidence`,
`verify_plan_step_execution_evidence_*`, and the integration
`evidence_hash_tamper_blocks_execution_as_operator_action_required`.

### Threat 3 — Policy TOCTOU

**Finding.** `process_queue` and ordinary treasury-policy updates still hold the
same `operation_guard` async mutex (`AsyncMutex<()>`, `state.rs`) for their
durations, so non-pause gate/cap updates cannot interleave with the drain.
Pause has a deliberately separate fast path: `POST /api/queue/pause` sets an
in-memory `AtomicBool` before it waits for that mutex. The drain reads the latch
between jobs and again as the final instruction before external broadcast;
resume clears it only after the durable policy says resumed. Startup restores
the latch from persisted `TreasuryPolicy.execution_paused`.
**Exploitability of the named windows.** An authenticated pause can therefore
preempt a drain even while the HTTP request is still waiting to persist the
policy. If it lands after signing, the exact signed bytes and hash remain in the
durable `prepared` state and no RPC occurs. The job may resume later with those
same bytes; it is never re-signed. Ordinary policy writes remain serialized by
the mutex. **Mitigation.** The operation mutex protects durable policy and
queue updates, while the lock-free latch closes the pause-specific preemption
gap at both job and broadcast boundaries. Enqueue and drain still re-derive
gate verdicts from current state rather than trusting recorded verdicts.
**Residual risk.** An out-of-band edit of `wallet_inventory.json` is Threat 1's
local-compromise boundary, not an in-daemon concurrency race.
**Tests.** `pause_halts_drain_mid_queue` (`tests/execution_gates.rs`) holds a
real first RPC broadcast, calls the HTTP pause route concurrently, observes the
latch before releasing RPC, proves no later job broadcasts, and proves the
persisted pause survives restart. The existing
`policy_flip_between_enqueue_and_drain_blocks_with_gate_reason`
(`tests/plan_enqueue.rs`) covers the ordinary policy-flip boundary.

### Threat 4 — Session-token theft → enqueue attempt (D-17 boundary)

**Finding.** With `allow_plan_execution` and the relevant per-family gate ON, a
stolen local session token can approve, enqueue, and drain a plan step and thus
move funds. This matches Decision D-17 exactly. **What stands between the thief
and fund movement.** (a) The execution gates default OFF, and while any gate is
off every enqueue route refuses at the policy check with an empty queue left
behind — a stolen token alone, in the shipped default posture, cannot move
funds. (b) Per-family gates scope which action classes are permitted. (c) Typed
confirmation at bulk enqueue (`EXECUTE N PLAN STEPS TOTAL W WEI`). (d) Audit
fingerprints: enqueue, sign, and broadcast each record an 8-byte truncated
SHA-256 `session_fingerprint_hex` of the token for attribution.
**Exploitability / honest residual risk.** The typed-confirmation phrase is
DISCLOSED in the mismatch response (`action` field), so it is fat-finger
protection, not an attacker barrier — a programmatic thief reads it and complies.
The audit fingerprints DETECT and attribute the movement; they do not PREVENT
it. Therefore, per D-17, with gates ON a stolen session token on the local
machine can move funds: the mitigations bound and detect the blast radius (gates
default off, per-family scoping, kill switch, immutable audit trail with session
fingerprints), they do not prevent it. FIDO2 tap-to-execute is the named
post-1.0 hardening candidate. This statement is the residual risk G2's
`docs/stability.md` and G5's readiness docs must carry. **Tests.** New
`enqueue_sign_broadcast_events_carry_matching_session_fingerprint`
(`tests/plan_execution.rs`) proves every fund-moving action is fingerprinted
under one session identity; existing
`gates_off_enqueue_routes_refuse_at_policy_check_and_queue_untouched` and
`enqueue_plan_refuses_wrong_confirmation_and_reports_expected_phrase`
(`tests/plan_enqueue.rs`) cover the default-off barrier and the phrase
disclosure.

### Threat 5 — Linkage bypass attempts

**Finding.** Cross-party linkage clustering
(`analyze_plan_linkage`) keys destinations and party attributions through
`normalize_linkage_address` (trim + ASCII-lowercase); the treasury allowlist
match uses `eq_ignore_ascii_case` over `normalize_address`-canonicalized
(lowercased, length/hex-validated) addresses. **Exploitability.** Checksum-case
(EIP-55) variants and surrounding whitespace all fold to one identity, so
case-variant or whitespace-padded destination encodings do NOT evade party
clustering or the allowlist; `fund_gas` sponsor-to-multiple-parties laundering
is caught by the same per-plan analyzer. **Mitigation.** Normalization is
applied consistently at both build and compare time; when
`block_cross_party_linkage` is on, a step whose destination would publicly link
two counterparties is refused at enqueue (`cross_party_linkage`). **Residual
risk (new limitation, see below).** Linkage analysis is per-plan: two
counterparties swept to a shared destination via steps in DIFFERENT consolidation
plans are not clustered, and cross-plan `fund_gas` laundering is likewise
unseen. This is a detection-completeness gap, not a normalization bypass, and is
bounded because the linkage warnings are advisory and the hard block still fires
within any single plan. Operator guidance: run one consolidation plan per review
cycle, or set distinct per-party sweep destinations. **Tests.** New
`normalize_linkage_address_folds_case_and_whitespace` and
`analyze_plan_linkage_clusters_case_variant_destinations` (unit, `planner.rs`)
prove case/whitespace variants still cluster; existing
`enqueue_step_refuses_cross_party_linkage` and the `fund_gas` linkage tests
(`tests/plan_enqueue.rs`) cover the hard block.

### F1 follow-up — `decode_quantity_hex` prefix-less tolerance on policy caps

**Finding.** `sigillum-core::decode_quantity_hex` (`ethereum_stealth.rs`)
accepts prefix-less digit strings and interprets them as hex (`"1000"` ->
`0x1000` = 4096 wei). At the operator-facing treasury-policy update boundary this
is a fund-controlling foot-gun: an operator typing a decimal-looking cap silently
gets a different, larger cap. **Decision.** FIX with strict `0x`-prefix
validation at the policy DTO boundary ONLY — `validated_cap_hex` and
`validated_required_quantity_hex` in `service/inventory/treasury.rs`, which back
`max_step_native_wei_hex`, `max_plan_native_wei_hex`, `max_gas_topup_wei_hex`,
`max_fee_per_gas_cap_hex`, `hot_floor_wei_hex`, and `hot_target_wei_hex`. A
prefix-less value now fails closed with HTTP 400
(`<field> must be a 0x-prefixed hex uint256 quantity`). `decode_quantity_hex`
was left globally tolerant deliberately: an impact review confirmed those two
helpers are its only policy-boundary callers, whereas the global function also
parses provider RPC quantity responses, on-chain balances, and evidence math,
where prefix-less tolerance is expected and changing it would be an out-of-scope,
higher-risk change. Built-in defaults already carry a `0x` prefix and are
unaffected. **Tests.** `treasury_policy_cap_fields_reject_prefixless_hex` and
`treasury_policy_required_fields_reject_prefixless_hex`
(`tests/execution_gates.rs`) assert both directions (prefix-less -> 400,
`0x`-prefixed -> 200 and persisted verbatim).

### New findings surfaced by this review

- **N1 (documented residual, not a code defect).** Cross-plan linkage detection
  gap — `analyze_plan_linkage` is per-plan; shared-destination or shared-gas-
  sponsor linkage spanning separate plans is not clustered. Bounded and given
  operator guidance under Threat 5; candidate for post-1.0 cross-plan analysis.
- **N2 (design property made explicit).** The bulk-enqueue typed-confirmation
  phrase is returned in the mismatch response, so it is fat-finger protection,
  not an attacker barrier — consistent with the D-17 residual risk under
  Threat 4.
- **N3 (inert).** `None` vs `Some("")` optional fields collide in the evidence
  canonicalization but are semantically identical (absent/zero); no value or
  routing change can hide behind the collision. Documented under Threat 2.

No fail-closed behavior was weakened. The single production change (F1) is
strictly additive validation at one operator-facing boundary. Verification:
`cargo test -p sigillum-daemon` (full crate), `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`./scripts/check-adversarial.sh`, and `./scripts/check-architecture.sh` all pass.
