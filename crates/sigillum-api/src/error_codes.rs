//! Stable machine-readable error codes for the daemon API error envelope.
//!
//! Every non-2xx response carries an [`crate::response::ErrorResponse`] whose
//! `code` field is one of these constants. Codes are part of the stable wire
//! contract (see `docs/stability.md`): they are free-form `snake_case` strings
//! rather than a Rust enum so that newer daemons can introduce new codes
//! without breaking older clients — consumers must match on the string value
//! and treat unrecognized codes as opaque.
//!
//! HTTP status codes stay the coarse classification; `code` refines the
//! statuses that are overloaded today (403, 404, 429) so operators and UIs
//! can pick the right remediation without parsing human-readable messages.

/// 400 — the request body failed DTO validation. `fields` may carry a
/// per-field breakdown (`FieldError { field, message }`); when absent, only
/// the top-level `error` message describes the failure.
pub const VALIDATION_FAILED: &str = "validation_failed";

/// 400 — the request is malformed or inconsistent outside DTO validation
/// (for example a service-layer precondition on the payload).
pub const BAD_REQUEST: &str = "bad_request";

/// 400 — a typed-confirmation phrase did not match the expected phrase.
/// `action` carries the exact expected phrase so UIs can render it.
pub const TYPED_CONFIRMATION_MISMATCH: &str = "typed_confirmation_mismatch";

/// 401 — the session token is missing, invalid, or the supplied credential
/// (passphrase, snapshot decryption key) did not authenticate.
pub const UNAUTHORIZED: &str = "unauthorized";

/// 403 — the request is refused for a reason not covered by a more specific
/// code (for example plan step-state refusals or cross-compartment access
/// rules). New, more specific codes may replace this over time; consumers
/// must handle it as the generic 403 fallback.
pub const FORBIDDEN: &str = "forbidden";

/// 403 — the vault (or the compartment holding the requested resource) is
/// locked, or no compartment is active for the session. Remediation: unlock
/// the vault or switch to the right compartment, then retry.
pub const VAULT_LOCKED: &str = "vault_locked";

/// 403 — a treasury execution gate denied the operation: the
/// `execution_paused` kill switch, a per-family `allow_*_execution` gate, a
/// per-profile `execution_enabled` flag, or a claim/gas-topup gate.
pub const EXECUTION_GATE_DENIED: &str = "execution_gate_denied";

/// 403 — the session is valid but lacks the capability scope the endpoint
/// requires (including endpoints that require a full daemon session rather
/// than a capability-scoped one).
pub const CAPABILITY_SCOPE_DENIED: &str = "capability_scope_denied";

/// 403 — a treasury transaction policy rule blocked the action. `action`
/// carries the machine-readable policy reason (for example
/// `cross_party_linkage`).
pub const POLICY_VIOLATION: &str = "policy_violation";

/// 404 — the requested resource (profile, plan, deposit, key, …) does not
/// exist.
pub const NOT_FOUND: &str = "not_found";

/// 404 — the daemon vault has not been initialized yet. Remediation:
/// complete first-run setup (`/api/compartment/init` or FIDO2 setup).
pub const NOT_INITIALIZED: &str = "not_initialized";

/// 409 — the operation conflicts with current daemon state (for example
/// unlocking an already-unlocked vault or creating a duplicate profile).
pub const CONFLICT: &str = "conflict";

/// 423 — the daemon is actively draining unlocked state (lock in progress);
/// retry once the lock completes.
pub const LOCKED_IN_PROGRESS: &str = "locked_in_progress";

/// 429 — an upstream provider (EVM RPC) rate-limited the request.
pub const RATE_LIMITED: &str = "rate_limited";

/// 429 — too many failed unlock attempts; the daemon enforces a cooldown.
pub const UNLOCK_THROTTLED: &str = "unlock_throttled";

/// 500 — an unexpected internal failure (I/O, cryptography, serialization).
pub const INTERNAL: &str = "internal";

/// 503 — the daemon is up but not ready to serve (startup recovery still
/// running).
pub const UNAVAILABLE: &str = "unavailable";

/// Deserialization fallback for envelopes produced before `code` existed.
/// The daemon never emits this value; clients should treat it as "no code"
/// and fall back to the HTTP status.
pub const UNKNOWN: &str = "unknown";
