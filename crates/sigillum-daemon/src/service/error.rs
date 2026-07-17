//! Service error types with HTTP status codes and stable machine-readable codes.
//!
//! Defines domain-specific errors mapped to appropriate HTTP status codes
//! (400, 401, 403, 404, 409, 423, 429, 500, 503) plus a stable `snake_case`
//! error code from [`sigillum_api::error_codes`] for REST API responses.
//!
//! ## Error-to-HTTP-Status Mapping Philosophy
//!
//! This module implements a centralized error mapping strategy that converts domain
//! errors (I/O, vault operations) into appropriate HTTP status codes. By implementing
//! `From` traits for external error types, service methods can use the `?` operator
//! directly on I/O and vault operations, avoiding verbose `.map_err()` chains.
//!
//! The mapping follows standard HTTP semantics:
//! - **400 Bad Request**: Client-supplied invalid input or malformed requests
//! - **401 Unauthorized**: Missing or invalid authentication credentials
//! - **403 Forbidden**: Valid credentials but insufficient permissions. The
//!   `code` disambiguates: `vault_locked` (unlock to retry),
//!   `execution_gate_denied` (treasury execution gates),
//!   `capability_scope_denied` (missing session scope), `policy_violation`
//!   (treasury policy block), or the generic `forbidden` fallback.
//! - **404 Not Found**: `not_found` for missing resources, `not_initialized`
//!   for a daemon that has not completed first-run setup.
//! - **409 Conflict**: Operation conflicts with current system state
//! - **423 Locked**: The daemon is actively draining unlocked state
//! - **429 Too Many Requests**: `unlock_throttled` (unlock cooldown) versus
//!   `rate_limited` (upstream provider limit)
//! - **500 Internal Server Error**: Unexpected I/O, cryptographic, or serialization failures

use std::fmt;

use axum::http::StatusCode;
use sigillum_api::error_codes;
use sigillum_core::VaultError;

pub(crate) type ServiceResult<T> = Result<T, ServiceError>;

#[derive(Debug, Clone)]
pub(crate) struct ServiceError {
    status: StatusCode,
    code: &'static str,
    message: String,
    action: Option<String>,
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.as_u16(), self.message)
    }
}

impl std::error::Error for ServiceError {}

impl From<std::io::Error> for ServiceError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(error.to_string())
    }
}

impl From<VaultError> for ServiceError {
    fn from(error: VaultError) -> Self {
        match error {
            VaultError::Locked => Self::vault_locked("Vault is locked."),
            VaultError::NotFound(_) => Self::not_found(error.to_string()),
            VaultError::NotInitialized => Self::not_initialized("Sigillum is not initialized."),
            _ => Self::internal(error.to_string()),
        }
    }
}

impl ServiceError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            action: None,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error_codes::BAD_REQUEST, message)
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, error_codes::UNAUTHORIZED, message)
    }

    /// Generic 403 refusal. Prefer the more specific constructors
    /// (`vault_locked`, `execution_gate_denied`, `capability_scope_denied`,
    /// `policy_violation`) whenever one applies.
    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, error_codes::FORBIDDEN, message)
    }

    /// 403 — the vault or the relevant compartment is locked (or no
    /// compartment is active); unlocking is the remediation.
    pub(crate) fn vault_locked(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, error_codes::VAULT_LOCKED, message)
    }

    /// 403 — a treasury execution gate (kill switch, per-family allow gate,
    /// per-profile execution flag, claim/gas-topup gate) denied the operation.
    pub(crate) fn execution_gate_denied(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            error_codes::EXECUTION_GATE_DENIED,
            message,
        )
    }

    /// 403 — the session is valid but lacks the required capability scope
    /// (or the endpoint requires a full daemon session).
    pub(crate) fn capability_scope_denied(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            error_codes::CAPABILITY_SCOPE_DENIED,
            message,
        )
    }

    pub(crate) fn policy_violation(action: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: error_codes::POLICY_VIOLATION,
            message: "policy_violation".into(),
            action: Some(action.into()),
        }
    }

    /// 400 with a machine-readable `action` payload (e.g. the exact expected
    /// typed-confirmation phrase) alongside the human-readable message.
    pub(crate) fn bad_request_with_action(
        message: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: error_codes::TYPED_CONFIRMATION_MISMATCH,
            message: message.into(),
            action: Some(action.into()),
        }
    }

    pub(crate) fn locked(message: impl Into<String>) -> Self {
        Self::new(StatusCode::LOCKED, error_codes::LOCKED_IN_PROGRESS, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, error_codes::NOT_FOUND, message)
    }

    /// 404 — the daemon vault has not been initialized yet (first-run setup
    /// incomplete), as opposed to a missing resource.
    pub(crate) fn not_initialized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, error_codes::NOT_INITIALIZED, message)
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, error_codes::CONFLICT, message)
    }

    /// 429 — an upstream provider (EVM RPC) rate limit.
    pub(crate) fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            error_codes::RATE_LIMITED,
            message,
        )
    }

    /// 429 — too many failed unlock attempts; the daemon enforces a cooldown.
    pub(crate) fn unlock_throttled(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            error_codes::UNLOCK_THROTTLED,
            message,
        )
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            error_codes::INTERNAL,
            message,
        )
    }

    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn action(&self) -> Option<&str> {
        self.action.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_map_to_documented_codes_and_statuses() {
        let cases: [(ServiceError, StatusCode, &str); 14] = [
            (
                ServiceError::bad_request("x"),
                StatusCode::BAD_REQUEST,
                error_codes::BAD_REQUEST,
            ),
            (
                ServiceError::unauthorized("x"),
                StatusCode::UNAUTHORIZED,
                error_codes::UNAUTHORIZED,
            ),
            (
                ServiceError::forbidden("x"),
                StatusCode::FORBIDDEN,
                error_codes::FORBIDDEN,
            ),
            (
                ServiceError::vault_locked("x"),
                StatusCode::FORBIDDEN,
                error_codes::VAULT_LOCKED,
            ),
            (
                ServiceError::execution_gate_denied("x"),
                StatusCode::FORBIDDEN,
                error_codes::EXECUTION_GATE_DENIED,
            ),
            (
                ServiceError::capability_scope_denied("x"),
                StatusCode::FORBIDDEN,
                error_codes::CAPABILITY_SCOPE_DENIED,
            ),
            (
                ServiceError::policy_violation("reason"),
                StatusCode::FORBIDDEN,
                error_codes::POLICY_VIOLATION,
            ),
            (
                ServiceError::bad_request_with_action("x", "phrase"),
                StatusCode::BAD_REQUEST,
                error_codes::TYPED_CONFIRMATION_MISMATCH,
            ),
            (
                ServiceError::locked("x"),
                StatusCode::LOCKED,
                error_codes::LOCKED_IN_PROGRESS,
            ),
            (
                ServiceError::not_found("x"),
                StatusCode::NOT_FOUND,
                error_codes::NOT_FOUND,
            ),
            (
                ServiceError::not_initialized("x"),
                StatusCode::NOT_FOUND,
                error_codes::NOT_INITIALIZED,
            ),
            (
                ServiceError::conflict("x"),
                StatusCode::CONFLICT,
                error_codes::CONFLICT,
            ),
            (
                ServiceError::too_many_requests("x"),
                StatusCode::TOO_MANY_REQUESTS,
                error_codes::RATE_LIMITED,
            ),
            (
                ServiceError::unlock_throttled("x"),
                StatusCode::TOO_MANY_REQUESTS,
                error_codes::UNLOCK_THROTTLED,
            ),
        ];
        for (error, status, code) in cases {
            assert_eq!(error.status(), status, "status for code {code}");
            assert_eq!(error.code(), code);
        }
        assert_eq!(
            ServiceError::internal("x").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(ServiceError::internal("x").code(), error_codes::INTERNAL);
    }

    #[test]
    fn policy_violation_and_typed_confirmation_keep_action_payloads() {
        let policy = ServiceError::policy_violation("cross_party_linkage");
        assert_eq!(policy.message(), "policy_violation");
        assert_eq!(policy.action(), Some("cross_party_linkage"));

        let confirmation = ServiceError::bad_request_with_action("mismatch", "expected phrase");
        assert_eq!(confirmation.message(), "mismatch");
        assert_eq!(confirmation.action(), Some("expected phrase"));
    }

    #[test]
    fn vault_errors_disambiguate_locked_and_not_initialized() {
        let locked = ServiceError::from(VaultError::Locked);
        assert_eq!(locked.status(), StatusCode::FORBIDDEN);
        assert_eq!(locked.code(), error_codes::VAULT_LOCKED);

        let uninitialized = ServiceError::from(VaultError::NotInitialized);
        assert_eq!(uninitialized.status(), StatusCode::NOT_FOUND);
        assert_eq!(uninitialized.code(), error_codes::NOT_INITIALIZED);

        let missing = ServiceError::from(VaultError::NotFound("secret".into()));
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.code(), error_codes::NOT_FOUND);
    }
}
