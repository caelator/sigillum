# Stability Policy

This document declares which Sigillum surfaces are stable as of 1.0.0, which
surfaces are explicitly unstable, and how versions evolve. It applies from the
`v1.0.0` tag onward.

## Versioning

- The workspace follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
  from 1.0.0: breaking changes to a stable surface require a major version,
  backward-compatible additions require a minor version, and fixes require a
  patch version.
- Crates are NOT published to crates.io; releases are annotated git tags (`v*`)
  plus GitHub Release artifacts (macOS desktop bundle, macOS/Linux CLI binaries),
  and library consumers use git or path dependencies. The version promises attach
  to the tagged releases.
- macOS is the supported desktop platform at 1.0; the daemon and CLI are
  supported on macOS and Linux; the Linux desktop build is compile-only; Windows
  is unsupported.

### Pre-tag adjustments

No valid `v1.0.0` tag or GitHub Release has published yet, so the stability
promises above are not yet in force. Until the first valid tag, release
candidates may adjust stable-candidate surfaces with the change recorded in
`CHANGELOG.md`. Current adjustments since `1.0.0-rc.5`:

- `sigillum api profiles eth-seed create` redacts the mnemonic from stdout by
  default (new `--reveal-mnemonic` and `--mnemonic-out PATH` flags control
  delivery), and `profiles eth-seed upsert` imports an existing mnemonic.
- `EthStealthGenerateResponse` and `EthStealthDepositMutationResponse` gained
  a backward-compatible `warnings` array (additive, defaults to empty).
- `ErrorResponse` gained a required `code` (stable machine-readable string
  from the catalog below; envelopes serialized before this change deserialize
  with the fallback `unknown`) and an optional `fields` array carrying
  per-field validation errors (`{field, message}`). Daemon errors now
  disambiguate the overloaded 403/404/429 statuses through the catalog, and
  CLI daemon errors print as `error[<code>]: <message>` (with one indented
  line per field error) when the daemon supplied a code.
- New routes `GET /api/operations`, `GET /api/operations/{id}`, and
  `POST /api/operations/{id}/cancel` expose the daemon's background-operation
  registry (in-memory, process-lifetime). `WalletInventoryScanRequest` gained
  an optional `run_async` flag (absent/false preserves the synchronous
  behavior exactly); `WalletInventoryScanResponse` and
  `DiscoveryJobMutationResponse` gained an additive optional `operation`
  field. `sigillum api inventory scan-evm` gained a matching `--run-async`
  flag.
- Discovery-job cancel/resume semantics are now real and tightened: cancel
  cooperatively stops the running scan (or marks an orphaned `running` job
  canceled) and conflicts (409 `conflict`) on terminal jobs; resume starts a
  NEW background operation and discovery job continuing from the interrupted
  job's persisted checkpoints and conflicts on completed or still-running
  jobs. Previously both verbs merely rewrote the stored status string.
- A discovery scan that fails mid-run now persists the job as `failed` with
  `last_error` (previously the record stayed `running` forever), which also
  makes it resumable.

## Stable at 1.0

Breaking any of these is a major-version event:

1. **`sigillum-api` wire shapes** - the request/response JSON contracts shared
   by the daemon, client, CLI, and UI. Enumerated values serialize to their exact
   current strings, and unknown inbound values deserialize into a
   forward-compatible catch-all variant instead of failing, so newer daemons never
   break older clients on values alone.
2. **Daemon route paths and semantics** - the HTTP route paths, methods, auth
   expectations (bearer session tokens over loopback), and fail-closed validation
   semantics of the local daemon API.
3. **CLI syntax** - command names, arguments, environment variables, and JSON
   output shapes of `sigillum` and the `sigillum api` operator commands.
4. **On-disk formats** - every persisted daemon store is schema-versioned and
   evolves by migration only: a newer daemon reads every older schema version and
   migrates it forward; formats are never rewritten incompatibly in place;
   downgrading a data directory to an older daemon is not supported.
5. **`sigillum-core` public traits** - the `SecretStore` and `VaultLifecycle`
   trait contracts.
6. **`TreasuryPolicy` fail-closed defaults** - every execution and automation
   capability (`allow_plan_execution` and its per-family gates,
   `allow_claim_execution`, `allow_gas_topups`, `allow_treasury_automation`, and
   the `block_cross_party_linkage` opt-in) defaults to OFF and requires an
   explicit operator opt-in. New capabilities ship default-off behind their own
   opt-ins. Weakening a fail-closed default is treated as a breaking change and
   will not happen within 1.x.

### Error code catalog

Every non-2xx daemon response carries the `ErrorResponse` envelope from
`sigillum-api`: `code` (one of the strings below), `error` (human-readable
message), optional `action` (machine-readable remediation payload), and
optional `fields` (per-field validation breakdown, `{field, message}` with
wire field paths such as `allowed_destinations[0].address`).

Codes are stable strings, deliberately not an enum: a newer daemon may add
codes within 1.x, and clients must treat unrecognized codes as opaque and
fall back to the HTTP status. Removing or repurposing an existing code is a
major-version event.

| Code | HTTP | Meaning |
| --- | --- | --- |
| `validation_failed` | 400 | Request body failed DTO validation; `fields` carries the per-field breakdown when the DTO reports one. |
| `bad_request` | 400 | Malformed or inconsistent request outside DTO validation. |
| `typed_confirmation_mismatch` | 400 | Typed-confirmation phrase mismatch; `action` carries the exact expected phrase. |
| `unauthorized` | 401 | Missing/invalid session token, or credential (passphrase, snapshot key) did not authenticate. |
| `forbidden` | 403 | Generic refusal not covered by a more specific code (e.g. plan step-state refusals). The 403 fallback; may gain more specific siblings over time. |
| `vault_locked` | 403 | Vault or relevant compartment is locked, or no compartment is active; unlock (or switch compartment) and retry. |
| `execution_gate_denied` | 403 | A treasury execution gate denied the operation: `execution_paused` kill switch, a per-family `allow_*_execution` gate, a per-profile `execution_enabled` flag, or a claim/gas-topup gate. |
| `capability_scope_denied` | 403 | Session is valid but lacks the required capability scope (or the endpoint requires a full daemon session). |
| `policy_violation` | 403 | A treasury transaction-policy rule blocked the action; `action` carries the policy reason. |
| `not_found` | 404 | The requested resource does not exist. |
| `not_initialized` | 404 | The daemon vault has not been initialized; complete first-run setup. |
| `conflict` | 409 | Operation conflicts with current daemon state (e.g. already unlocked, duplicate profile). |
| `locked_in_progress` | 423 | The daemon is draining unlocked state; retry once the lock completes. |
| `rate_limited` | 429 | An upstream provider (EVM RPC) rate-limited the request. |
| `unlock_throttled` | 429 | Too many failed unlock attempts; the daemon enforces a cooldown. |
| `internal` | 500 | Unexpected internal failure. |
| `unavailable` | 503 | The daemon is up but not ready to serve (startup recovery running). |

## Unstable at 1.0

These may change in any release without a major-version bump:

- Internal daemon Rust modules - `sigillum-daemon`'s module layout and non-route
  internals are implementation details.
- The operator console DOM - element IDs, selectors, markup structure, and the
  generated `app.js`/`styles.css` assets.
- The gateway sidecar - `sigillum-gateway` keeps its local-sidecar preview
  positioning; its API keys, payment-intent, and webhook surfaces are preview
  quality. Payment creation is disabled by default behind
  `GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS=1`; balance observations are not finality
  proof and are exposed only as the latest balance observation. Privileged
  third-party invoice-signing callbacks are not implemented.
- The `sigillum-sdk` and `sigillum-server` facade crates.
- Anything in the documented 1.0 non-goals: non-EVM chains (Bitcoin/UTXO,
  Solana, Tron, Cosmos), swap execution and DEX routing, price/valuation feeds,
  external runtime registries/feeds (token lists, spender reputation, and spam
  lists stay operator-imported local files), Lido withdrawal-queue exits (wstETH
  unwrap only), remote/hosted/multi-host/internet-facing modes, crates.io
  publishing, and Windows support. These do not exist at 1.0 and no compatibility
  promise attaches to them.

## Assurance boundary

The 1.0 claims rest on the source-verified local-first release gate
(`./scripts/check-release.sh`: tests, adversarial property-based coverage,
runtime smoke, supply-chain audits) plus recorded soak and testnet evidence. No
external penetration test has been performed, and the release does not claim one.

## Recorded residual risk: local execution surface

Sigillum 1.0 adds policy-gated execution of consolidation plans. With
`allow_plan_execution` (and the relevant per-family execution gates) enabled, a
stolen session token on the local machine can move funds. The shipped mitigations
— typed confirmation at enqueue, per-family fail-closed policy gates, gate-flip
audit events carrying session fingerprints, the `execution_paused` kill switch
latched between jobs and immediately before broadcast, durable exact-byte
submission recovery that never re-signs a prepared job, and policy re-reads at
both enqueue time and queue-drain time — detect and bound
that misuse; they do not prevent it. FIDO2 tap-to-execute is the named post-1.0
hardening candidate for closing this gap. Operators who do not accept this risk
should leave the execution gates off (their default), which preserves the
export-only plan handoff.
