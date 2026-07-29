# Refactor Notes

These notes record the ownership rules that should stay true as Sigillum moves
from broad files toward small domain modules. They describe the current code,
not an aspirational end state.

## Refactor Principles

- Preserve daemon routes, JSON wire shapes, CLI commands, and public client
  method names unless a separate migration plan says otherwise.
- Move behavior by domain, not by convenience. A moved type, method, or command
  should land next to the code that owns the corresponding operator workflow.
- Keep route handlers thin. Business rules belong in daemon service modules.
- Prefer internal module visibility. Use top-level re-exports only to preserve
  public crate contracts.
- Keep tests outside production files for API and client crates.
- Ratchet line budgets after each meaningful split so large files do not
  quietly grow back.

## Current Ownership

- `sigillum-daemon/src/routes/*` owns HTTP wiring and request extraction.
- `sigillum-daemon/src/service/*` owns daemon business behavior.
- `sigillum-daemon/src/service/queue/*` owns queue payload construction,
  processing, sweep execution, state recovery, retry policy, and related tests.
- `sigillum-daemon/src/service/evm/rpc.rs` owns JSON-RPC transport and provider
  error classification. The parent EVM service still owns wallet-facing signing
  and balance helpers until those seams are split further.
- `sigillum-daemon/src/service/profiles/resolution.rs` and
  `sigillum-daemon/src/service/profiles/sends.rs` own profile-backed lookup and
  send construction. `profiles.rs` remains the profile CRUD facade.
- `sigillum-daemon/src/service/inventory/treasury/*` owns treasury overview,
  receiving, policy, party, and allocation behavior behind a narrow facade.
- `sigillum-daemon/ui/src/app.ts` is the runtime entry. It should stay focused
  on boot flow and composition.
- `sigillum-daemon/ui/src/views/*`, `actions/*`, `render/*`, `state/*`, and
  `api/*` own typed frontend domains. `app.js` and `styles.css` are checked-in
  build outputs embedded by Rust.
- `sigillum-daemon/ui/src/styles/*` owns authored CSS. The root stylesheet is a
  build artifact or narrow import surface, not a dumping ground.
- `sigillum-api` owns transport DTOs. Domain modules may be internal as long as
  existing public type names remain re-exported from `request`, `response`, and
  the crate root.
- `sigillum-client` owns async daemon API methods. Domain methods should move
  into crate-local modules while preserving `SigillumClient` method names.
- `sigillum-cli/src/daemon_api.rs` owns top-level `sigillum api` dispatch.
  Domain subcommands should live under `sigillum-cli/src/daemon_api/*` while
  preserving command names and flags.

## Queue Domain Checkpoint

Wave 5 established the first API/client/CLI vertical slice:

- Queue request DTOs live in `sigillum-api/src/request/queue.rs`.
- Queue response DTOs live in `sigillum-api/src/response/queue.rs`.
- Both API modules are private implementation modules with public type names
  re-exported from `request.rs`, `response.rs`, and the crate root.
- Queue client methods live in `sigillum-client/src/queue.rs`.
- `sigillum api queue ...` handling lives in
  `sigillum-cli/src/daemon_api/queue.rs`.
- `scripts/check-architecture.sh` enforces the module locations, public
  re-export pattern, no inline tests in the split API/client files, and line
  budgets for the moved files.

Future API/client/CLI domain splits should follow this shape: move one domain
end to end, keep public names stable, add or update tests, document ownership,
ratchet guardrails, then re-run GitNexus indexing after commit.

## W7.3 Queue Execution Split

Mirrors the sweeps split for the two families the drain loop lifts behind
the W7.1 execution gates:

- `PlanStepExecution` job execution owns `service/queue/plan_steps.rs`
  (pre-signing guards: dependency ordering, evidence-hash re-verification,
  signer resolution, fee cap) and `service/queue/plan_steps/signing.rs` (signing
  only, with no post-sign network I/O). `service/queue/processing.rs` persists
  `prepared` and `submitted_unknown` barriers, while
  `service/queue/broadcast.rs` owns exact-byte submission, recovery, and typed
  broadcast/broadcast-failed audit events for every queue family.
- Legacy `EthSeed*` job execution owns `service/queue/seed_sends.rs`,
  mirroring `sweeps.rs`'s balance-check-then-send shape but signing with a
  key derived on demand from the seed wallet's vault-stored mnemonic.
- Both families gate through the same `ExecutionFamily`/`execution_gate_*`
  machinery `gates.rs` already exposed for `PlanStepExecution`; `queue.rs`'s
  drain loop no longer hard-blocks either family once gates pass.
- Once a job reaches `prepared`, no queue module may re-sign it. Recovery checks
  the stored hash and may resubmit only the persisted raw bytes.
- `scripts/check-architecture.sh` enforces the new module locations and line
  budgets.
