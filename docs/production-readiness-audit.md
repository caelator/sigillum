# Production Readiness Audit

**Date:** June 4, 2026 (updated July 3, 2026)
**Scope:** local-first, single-host Sigillum source checkout and local-sidecar
gateway boundary
**Verdict:** source release gate passed for the local-first wallet-management
workstation, including EVM roadmap phases 1-9 and policy-gated, fail-closed
consolidation-plan execution that is opt-in and defaults off. Wallet management
is complete for EVM except swap execution, which is deferred per D-13. Non-EVM
chains (roadmap phase 10) and fiat/NFT valuation (D-16) remain deferred; broader
production completeness remains bounded by the open gaps below.

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
- desktop compile and bundle smoke through `scripts/check-desktop.sh`, which
  always builds `sigillum-desktop` and, on macOS, runs a debug Tauri bundle
  build, asserts `.app` and `.dmg` outputs, and verifies the `.app` carries a
  code signature
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

The desktop bundle proof is now repeatable as `scripts/check-desktop.sh`, wired
into the release gate after the browser smoke. It always compiles
`sigillum-desktop` with the locked dependency graph. On macOS, it also runs
`cargo tauri build --debug`, requires the debug `.app` and `.dmg` artifacts,
and verifies the `.app` is code-signed. The macOS bundle portion can be skipped
with `SIGILLUM_SKIP_DESKTOP_BUNDLE=1` only when the host cannot build Tauri
bundles; non-macOS hosts print an explicit bundle-skip line after the compile
check.

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

No RustSec advisories are currently ignored. The former temporary
exceptions for RUSTSEC-2026-0194 and RUSTSEC-2026-0195 (quick-xml 0.39.x
DoS advisories via `plist` ← `tauri`) were removed on 2026-07-08 after
`cargo update` moved the workspace to quick-xml 0.41.0, which is outside
the advisories' vulnerable range; `cargo audit` and `cargo deny check`
pass with empty ignore lists.

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
| Desktop app bundle readiness | `scripts/check-desktop.sh` always compiles `sigillum-desktop`; on macOS it runs a debug Tauri bundle build, asserts `.app` and `.dmg` artifacts, and verifies the `.app` code signature | Proven for current checkout on macOS; non-macOS release-gate legs compile the desktop crate and explicitly skip bundle packaging |
| Long-duration reliability | Recovery and crash tests pass, `scripts/check-local-soak.sh` passed a bounded 300-second local daemon/gateway run with 28 iterations, and a `mac-server` target-host run passed a 3600-second target with 117 iterations | Proven for the current local-first target host; broader host coverage and chaos testing remain future assurance work |
| External security assurance | Code gates, audit, deny, local adversarial/fuzz gate, SSRF/local-boundary tests, and UI boundary tests pass | Local boundary pass proven for current checkout; independent external penetration test not performed |
| Full wallet-management product roadmap | EVM roadmap phases 1-9 shipped and tested: discovery, inventory, risk, planning, policy-gated fail-closed execution (default off), DeFi exit adapters, and treasury automation | EVM scope is complete except swap execution, which is deferred per D-13; only non-EVM chains (phase 10), swap execution (D-13), and fiat/NFT valuation (D-16) remain deferred |

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

The active product objective remains larger than the current release gate. To
claim Sigillum is fully operational and production ready without qualification,
the project still needs:

1. a target-host `sigillum doctor` and soak receipt for each additional host
   being called ready beyond `mac-server`
2. an independent external penetration test if the claim expands beyond
   the source-verified local-first release gate; no external penetration test
   has been performed, and the release does not claim one (D-4)
3. public-testnet execution receipts for the four core execution families:
   native sweep, ERC-20 sweep, revoke, and gas top-up (F6)

Until those are complete, the accurate claim is narrower: the current source
checkout passes the local-first release gate, and the docs identify the
remaining assurance and product-completeness work.

## Execution-path security review (F5)

A focused adversarial review of the W7 queue-execution surface
(`service/queue/{gates,plan_steps,plan_steps/signing,processing}.rs`,
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

**Finding.** Both `process_queue` (the drain) and `update_treasury_policy` (and
pause/resume) acquire the same `operation_guard` async mutex
(`AsyncMutex<()>`, `state.rs`) for their entire duration, and every gate, pause,
and cap decision is a FRESH on-disk read (`current_treasury_policy` ->
`load_wallet_inventory`). **Exploitability of the named windows.** Because the
mutex makes policy writes and drains mutually exclusive, there is no in-daemon
window between hash-verify and sign, or between sign and broadcast, in which an
authenticated policy flip can land: a policy-update request issued during a
drain BLOCKS until the drain releases the guard, and vice-versa. The kill switch
is additionally re-checked at the top of every job iteration, so a pause halts
the queue before the next job starts. **Mitigation.** Serialization by
`operation_guard` + per-job fresh reads; enqueue and drain both re-derive the
gate verdict from current state rather than trusting recorded verdicts.
**Residual risk.** The only way to mutate policy mid-drain is an out-of-band edit
of `wallet_inventory.json` on disk, which is Threat 1's local-compromise
boundary, not a concurrency race. **Tests.** New
`policy_update_and_drain_serialize_no_torn_state_or_double_broadcast`
(`tests/plan_execution.rs`) runs a drain and a pause concurrently and asserts no
torn state and never a double broadcast; existing
`policy_flip_between_enqueue_and_drain_blocks_with_gate_reason`
(`tests/plan_enqueue.rs`) and `pause_halts_drain_mid_queue`
(`tests/execution_gates.rs`) cover the observable enqueue->drain and
mid-queue-pause windows.

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
