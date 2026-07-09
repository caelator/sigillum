//! Service error types with HTTP status codes.
//!
//! Defines domain-specific errors mapped to appropriate HTTP status codes
//! (400, 401, 403, 404, 409, 500) for REST API responses.
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
//! - **403 Forbidden**: Valid credentials but insufficient permissions (locked vault)
//! - **423 Locked**: The daemon is actively draining unlocked state
//! - **404 Not Found**: Requested resource or vault initialization state not available
//! - **409 Conflict**: Operation conflicts with current system state
//! - **500 Internal Server Error**: Unexpected I/O, cryptographic, or serialization failures

use std::fmt;

use axum::http::StatusCode;
use sigillum_core::VaultError;

pub(crate) type ServiceResult<T> = Result<T, ServiceError>;

#[derive(Debug, Clone)]
pub(crate) struct ServiceError {
    status: StatusCode,
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
            VaultError::Locked => Self::forbidden("Vault is locked."),
            VaultError::NotFound(_) => Self::not_found(error.to_string()),
            VaultError::NotInitialized => Self::not_found("Sigillum is not initialized."),
            _ => Self::internal(error.to_string()),
        }
    }
}

impl ServiceError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            action: None,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub(crate) fn policy_violation(action: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
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
            message: message.into(),
            action: Some(action.into()),
        }
    }

    pub(crate) fn locked(message: impl Into<String>) -> Self {
        Self::new(StatusCode::LOCKED, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub(crate) fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn action(&self) -> Option<&str> {
        self.action.as_deref()
    }
}
