# Production Readiness Audit

**Date:** June 4, 2026 (updated July 31, 2026)
**Scope:** local-first, single-host Sigillum source checkout and local-sidecar
gateway boundary
**Verdict:** there is no production-ready or published stable release.
`v1.0.0-rc.2` through `v1.0.0-rc.4` are immutable failure receipts for the tag,
packaging, and F6/runtime contracts. `v1.0.0-rc.5` at `7e04743` completed its
workflow and has checksum-verified assets in an unpublished historical draft,
but it cannot certify the later integrated hardening changes.

`v1.0.0-rc.6` is now immutable failed-workflow evidence. Annotated tag object
`1687443c67e6a90b1db84c78d6f372463dc8c639` peels to protected-main commit
`194a90384bccef65bed42cf491d763a4c46948c0`. Release run `30600446396` passed
the release-contract job, both verify jobs, and both artifact jobs. Its final
job created one unique unpublished `prerelease=true` draft with the exact six
expected nonempty, checksum-valid assets, then failed because an immediate
list query 336 ms later did not observe that newly created release. RC6 cannot
qualify and must remain preserved; its tag and draft are not moved, deleted,
reused, or published.

`v1.0.0-rc.7` is the successful pre-migration evidence anchor. Annotated tag
object `c086f10ef411fc6341a713f3e98ba32e97351096` peels to the then-current
protected-main commit `3a4dbbf5294710056b2f0685b4c9bb9e985c730a`. Release
run `30612063470` passed the release-contract job, both verify jobs, both
artifact jobs, and the release job. It left exactly one unpublished
`prerelease=true` draft with the six expected nonempty uploaded assets. RC7
proves the bounded visibility fix and complete release automation under the
prior macOS 15 target contract; it cannot qualify a macOS 26 runner/support
contract introduced after its tag.

The next eligible evidence anchor is `v1.0.0-rc.8` only after this target
migration lands through protected `main` and the exact clean release gate,
independent review, and required CI pass for that resulting main SHA. Every
current-line automated and operator receipt must bind to that RC8 peeled SHA.
C7 is partial: the five-destination operator console is implemented, but its
RC8 walkthrough and operator sign-off remain open. The target-host evidence
boundary is macOS 26 on Apple Silicon (`aarch64`) only. The current macOS
26.5.2/arm64 host matches the platform class, but its historical receipts
remain non-qualifying because they are not RC8-bound. No final `v1.0.0` tag or
published GitHub Release exists. Gateway payments remain disabled-by-default
experimental observations, not supported 1.0 confirmation semantics.

RC4 release run `29230844456` nevertheless completed all six jobs, including
strict source and mounted-dmg verification, and produced its six expected draft
assets. That is retained as positive evidence for the signing remediation and
negative evidence that a green workflow cannot override a defective assurance
contract. RC5 run `29248938476` is the first succeeding candidate under the
remediated tag, packaging, runtime, and F6 schema-v2 contracts, but operator
evidence remains incomplete as described above.

RC5 release run `29248938476` completed the corrected six-job workflow and left
its six expected assets in an unpublished draft. That is positive evidence for
the RC5 code and release contracts at `7e04743`, not for later commits. RC5
receipts cannot be carried forward.

## Evidence Snapshot

Historical checkpoint `fc1e93b` passed its then-current clean-tree release gate
in 919 seconds, but that SHA is predecessor evidence only. Pre-migration
protected-main commit `3a4dbbf5294710056b2f0685b4c9bb9e985c730a` passed
`./scripts/check-release.sh` from a clean tree without tracked-file mutation,
completed independent review, landed through protected `main`, passed required
CI, and anchored the successful RC7 workflow. Those receipts establish the
integrated source and prior target contract. The macOS 26 runner/support
migration changes that contract after RC7; its exact resulting protected-main
SHA must repeat the gate, review, and required CI before RC8 may be created.
The gate makes the release standard executable instead of scattering it across
README, CI, and readiness notes.

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
the final draft-release job. RC releases must remain draft, unpublished, and
`prerelease=true`; the final draft and published release must be
`prerelease=false`. Pushed RC tags are permanent receipt anchors, and the RC
draft/assets remain available through final-draft verification. A final
`v1.0.0` tag must peel to the identical commit as the RC receipts, or those
receipts are void and a new RC is required.

The final-promotion implementation reruns the source-verification legs but
skips artifact rebuilds. It copies the exact five qualified RC payload bytes
under final names, verifies byte identity and tag-normalized digest equality,
regenerates `SHA256SUMS`, and adds the validated release-evidence bundle as the
seventh asset. Fresh final-tag payload builds cannot substitute for the
qualified RC bytes. The implementation and bounded visibility fix are
source-verified at `3a4dbbf`, and the RC7 workflow passed end to end under the
prior target contract. The macOS 26 runner/support contract remains
unqualified until its protected-main SHA passes the gates and the RC8 workflow
passes end to end.

RC release records must remain unpublished drafts with `prerelease=true`
through qualification and exact-byte promotion. The final draft and published
release must both have `prerelease=false`.
`scripts/check-release-state-contract.sh` and the H2 ceremony validate
`rc-draft`, each `final-draft` snapshot, and `final-published` explicitly
instead of treating `draft` alone as sufficient release metadata.

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
historical successor to the separate RC4 evidence-contract failure. RC6 is the
retained historical successor at `194a903`: its contract, verify, and artifact
jobs passed and its exact six checksum-valid assets remain in a unique
unpublished prerelease draft, but the final job failed on the 336 ms
release-visibility race. RC7 is the retained successful successor at
`3a4dbbf`: its bounded visibility fix, exact-HEAD gates, protected merge, CI,
and all six release-workflow jobs passed, leaving one unique unpublished
prerelease draft with the expected six nonempty uploaded assets. Because the
macOS 26 runner/support contract changes after RC7, the next permitted
candidate is `v1.0.0-rc.8` only after this migration's exact-HEAD gating,
review, protected merge, and CI.

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

The configurable reliability harness is `scripts/check-local-soak.sh`. A
historical bounded local validation run passed with `SIGILLUM_SOAK_SECONDS=300`,
`SIGILLUM_SOAK_INTERVAL_SECONDS=10`, and 28 full iterations. A longer
historical run also passed on `mac-server` with
`SIGILLUM_SOAK_SECONDS=3600`, `SIGILLUM_SOAK_INTERVAL_SECONDS=30`, and
`SIGILLUM_SOAK_RECEIPT=target/readiness/local-soak-3600-d1fd325.json`.
The receipt records a clean `main` checkout at
`d1fd32570f461c77f47c736a011295cd49d70cc4`, Darwin `25.5.0`, 117 iterations,
117 `sigillum doctor` runs, the daemon and gateway loopback URLs, and the
checked surfaces: daemon status, vault API-key write/read canary, gateway
health, and `sigillum doctor`.
Darwin `25.5.0` corresponds to macOS 26.5.2 and matches the new macOS 26
OS/architecture class. This receipt remains non-qualifying historical harness
evidence because it binds to `d1fd325`, predates the new target contract, and
does not bind to RC8; it cannot satisfy RC8 F4, doctor, clean-install, or C7
gates.

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
| Build and dependency graph resolve | `cargo metadata --locked --no-deps --format-version 1` inside `./scripts/check-release.sh` | Passed at pre-migration protected-main commit `3a4dbbf`; repeat after the macOS 26 migration for RC8 |
| Architecture stays within professional boundaries | `./scripts/check-architecture.sh` inside the release gate | Exact source gate and independent review passed at `3a4dbbf`; repeat after the macOS 26 migration for RC8 |
| Daemon UI compiles and tested source matches generated assets | `npm ci`, `npm audit --audit-level=high`, `npm run typecheck`, `npm test`, `npm run build`, generated-asset freshness, and pinned accessibility checks | Source gate passed at `3a4dbbf`; RC8 operator sign-off remains |
| Rust workspace builds, tests, and lints | Locked workspace check/test/clippy plus independent no-HID FIDO2 check/test/clippy inside the release gate | Passed at pre-migration protected-main commit `3a4dbbf`; repeat after the macOS 26 migration for RC8 |
| Security and supply-chain baseline | `cargo audit --file Cargo.lock` and `cargo deny --locked check` inside the release gate | Passed for the exact lockfile at `3a4dbbf`; repeat after the macOS 26 migration for RC8 |
| Release identity and monotonicity | Remote direct/peeled tag validation, event-SHA and cross-job tag-object binding, scratch-ref recovery, retained monotonically numbered RC tags, draft-only release creation, and explicit RC/final draft/published prerelease-state contracts | RC7 passed all six jobs and left one unique unpublished prerelease draft with six expected nonempty uploaded assets; the post-RC7 macOS 26 contract requires a fresh RC8 end-to-end receipt |
| Local daemon and gateway loopback integration behavior | Workspace integration tests pass outside the sandbox | Exact source gate and protected-main CI passed at `3a4dbbf`; candidate qualification under the new target requires the RC8 workflow |
| Target-host operational readiness | RC5 standard soak, chaos soak, and doctor receipts bind to `7e04743`; the schema-v2 receipt contract records platform, exact macOS product version, canonical architecture, and opaque machine identity | Proven for RC5 only; repeat the 3600-second standard soak, 600-second chaos soak, and doctor at RC8 on the same eligible macOS 26/aarch64 host |
| Runtime daemon lifecycle behavior | `scripts/check-runtime-smoke.sh` starts the daemon, verifies status, initializes a passphrase compartment, writes and reads vault canaries, locks, unlocks, lists compartments, and runs doctor | Exact source runtime gate passed at `3a4dbbf`; eligible-host RC8 doctor receipt remains |
| Queue submission durability, dependency finality, and pause | Queue schema v5 persists `prepared` raw bytes/hash and a pre-RPC `submitted_unknown` marker; recovery checks receipts or resubmits exact bytes without re-signing; dependent plan steps remain unsigned until every prerequisite reaches receipt-confirmed finality; the real HTTP pause regression latches before the active drain mutex and blocks later broadcasts | Exact source gate and protected-main CI passed at `3a4dbbf`; RC8 release and field evidence remain |
| Runtime browser/UI visual behavior | DOM tests, mock accessibility, screenshots, and migrated real-daemon browser smoke cover visible setup and all five destinations | Source gate passed at `3a4dbbf`; RC8 operator walkthrough/sign-off remains |
| Desktop app bundle readiness | Source/mounted-dmg verification runs inside `./scripts/check-release.sh`; RC7 workflow `30612063470` passed all six jobs and produced one unique draft with the expected six nonempty uploaded assets | RC7 proves the prior target contract; a successful macOS 26 RC8 workflow plus clean-machine install/unlock remain unproved |
| Long-duration reliability | Recovery/crash tests plus RC5 standard 3600-second and chaos 600-second receipts passed at `7e04743` | Historical for the feature line; schema-v2 standard and chaos receipts from the same eligible target host must bind to RC8 |
| External security assurance | Code gates, audit, deny, local adversarial/fuzz gate, SSRF/local-boundary tests, and UI boundary tests are part of the release contract | Exact source contract and independent review passed at `3a4dbbf`; no independent external penetration test has been performed |
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

The pre-migration protected-main source line at `3a4dbbf` completed the exact
clean release gate, independent review, protected merge, required CI, and
successful RC7 workflow. RC6 is preserved as failed-workflow evidence, and RC7
is preserved as successful automation evidence under the prior target contract.
The remaining release-qualified evidence is:

1. the macOS 26 runner/support migration landed through protected `main`,
   followed by the exact clean release gate, independent review, and required
   CI at the resulting main SHA
2. an immutable RC8 tag plus a successful end-to-end workflow leaving one
   draft, unpublished, `prerelease=true` GitHub Release whose exact six
   nonempty assets independently verify at that exact SHA
3. F7 upgrade verification at RC8 and schema-v2 standard 3600-second and chaos
   600-second F4 receipts plus `sigillum doctor` on the same eligible macOS
   26/aarch64 host
4. five funded public-testnet transactions for the four core execution families:
   native sweep, ERC-20 sweep, revoke, and both legs of `fund_gas` → dependent
   sweep on Ethereum Sepolia (`11155111`) plus one supported L2 testnet: Base
   Sepolia (`84532`), Arbitrum Sepolia (`421614`), or OP Sepolia (`11155420`)
   (F6), all bound to RC8
5. a checksum-verified RC8 dmg clean install reaching unlock without a
   developer toolchain and RC8 operator walkthrough/sign-off across all five
   destinations (C7)
6. RC8-bound schema-v2 evidence-bundle validation, exact-byte final-draft
   verification with the bundle as the seventh asset and
   `prerelease=false`, and explicit H2 approval recorded before invoking the
   final-tag ceremony

An independent external penetration test is additionally required only if the
assurance claim expands beyond the source-verified local-first boundary. No
external penetration test has been performed, and this release does not claim
one (D-4).

Until those are complete, the accurate claim is narrower: the integrated source
at `3a4dbbf` implements the intended local-first product and passed its exact
release gate, independent review, protected-main CI, and all six RC7 workflow
jobs under the prior target contract. RC6 remains non-qualifying
failed-workflow evidence, and RC7 remains successful pre-migration automation
evidence. The macOS 26 protected-main migration, successful RC8 workflow,
eligible-host, testnet, clean-install, operator-sign-off, exact-byte promotion,
and explicit H2 receipts remain.

## Execution-path security review (F5)

A focused adversarial review of the W7 queue-execution surface
(`service/queue/{gates,pause,processing,broadcast,plan_steps,plan_steps/signing}.rs`,
`service/inventory/plan_execution_enqueue.rs`,
`service/inventory/treasury/{mod.rs,allocations.rs,policy.rs,receiving.rs}`,
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
`validated_required_quantity_hex` in
`service/inventory/treasury/policy.rs`, which back
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
