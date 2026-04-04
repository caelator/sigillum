//! Core service layer for Sigillum vault operations.
//!
//! [`SigillumService`] is the single entry point for all business logic.
//! Route handlers in `crate::routes` delegate directly to its methods, which
//! coordinate vault access, audit logging, and operation journaling.
//!
//! ## Module layout
//!
//! Each sub-module owns one bounded domain:
//!
//! - **lifecycle** — passphrase unlock, FIDO2 unlock, lock, session revocation
//! - **secrets / wallets** — tier-1 (API keys) and tier-2 (encrypted) secret CRUD
//! - **compartments** — compartment init, add, remove, switch, listing
//! - **fido2** — hardware key registration, removal, setup wizard
//! - **deposits** — stealth deposit creation, refresh, sweep enqueueing
//! - **queue** — job processing with exponential backoff
//! - **evm** — on-chain balance observation and transaction construction
//! - **profiles** — EVM provider and stealth wallet profile management
//! - **maintenance** — compound refresh + queue-drain cycles
//! - **backup / recovery** — encrypted snapshot export/restore
//! - **transit** — inter-compartment secret push
//! - **observability** — status and diagnostics endpoints
//! - **error** — [`ServiceError`] type with HTTP status code mapping
//! - **helpers** — shared hex decoding, u256 arithmetic, timestamps

mod backup;
mod compartments;
mod deposits;
mod error;
mod evm;
mod fido2;
pub(crate) mod helpers;
mod lifecycle;
mod maintenance;
mod observability;
mod profiles;
mod queue;
mod recovery;
mod secrets;
mod transit;
pub(crate) mod unlock;
mod wallets;

use std::sync::Arc;

use sigillum_core::VaultError;

use crate::AppState;
use crate::audit_log::AuditEventSpec;
use crate::operations::{OperationGuard, PendingOperationSpec};

pub(crate) use error::{ServiceError, ServiceResult};

/// Façade over all vault operations.
///
/// Holds an `Arc<AppState>` and delegates to domain-specific methods defined
/// across sub-modules. Every public method follows the same contract:
/// validate the session token → acquire the operation guard if mutating →
/// perform the operation → record an audit event → return the typed response.
pub(crate) struct SigillumService {
    state: Arc<AppState>,
}

impl SigillumService {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Verify that the request carries a valid session token, returning a reference to it.
    fn require_session<'a>(&self, token: Option<&'a str>) -> ServiceResult<&'a str> {
        match token {
            Some(token) if self.state.verify_token(token) => Ok(token),
            _ => Err(ServiceError::unauthorized(
                "Invalid or missing session token.",
            )),
        }
    }

    /// Optionally verify a session token without rejecting unauthenticated callers.
    fn optional_session<'a>(&self, token: Option<&'a str>) -> Option<&'a str> {
        token.filter(|candidate| self.state.verify_token(candidate))
    }

    /// Append a typed audit event, mapping I/O failures to `ServiceError`.
    fn record_audit(
        &self,
        compartment_id: Option<usize>,
        spec: AuditEventSpec,
    ) -> ServiceResult<()> {
        self.state
            .record_audit_event(compartment_id, spec)
            .map_err(|error| ServiceError::internal(format!("Failed to write audit log: {error}")))
    }

    /// Begin a journaled operation, returning an RAII guard that cleans up on drop.
    fn begin_operation(
        &self,
        spec: PendingOperationSpec,
        subject: Option<String>,
    ) -> ServiceResult<OperationGuard> {
        self.state.begin_operation(spec, subject).map_err(|error| {
            ServiceError::internal(format!("Failed to journal operation: {error}"))
        })
    }

    /// Run a closure against the vault for the caller's active compartment.
    fn with_active_vault<R, F>(&self, token: &str, f: F) -> ServiceResult<R>
    where
        F: FnOnce(&sigillum_core::FileVault, usize) -> ServiceResult<R>,
    {
        let id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
        self.state
            .with_active_vault_for(token, |vault| f(vault, id))
            .unwrap_or_else(|| Err(ServiceError::forbidden("No active compartment.")))
    }

    /// Run a closure against a specific compartment's vault by ID.
    fn with_vault<R, F>(&self, id: usize, f: F) -> ServiceResult<R>
    where
        F: FnOnce(&sigillum_core::FileVault) -> ServiceResult<R>,
    {
        self.state
            .with_vault(id, f)
            .unwrap_or_else(|| Err(ServiceError::internal("Vault not found.")))
    }

    /// Map a [`VaultError`] to the appropriate HTTP-level [`ServiceError`] for snapshot ops.
    fn snapshot_error(context: &str, error: VaultError) -> ServiceError {
        match error {
            VaultError::NotInitialized => ServiceError::not_found("Sigillum is not initialized."),
            VaultError::Decryption(_) => ServiceError::unauthorized(format!(
                "{context}: wrong passphrase or corrupted snapshot."
            )),
            VaultError::Serialization(_) => {
                ServiceError::bad_request(format!("{context}: invalid snapshot format."))
            }
            other => ServiceError::internal(format!("{context}: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::*;

    fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold,
            passphrase_mode: None,
        }
    }

    #[test]
    fn invalid_session_hides_unlocked_status() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let service = SigillumService::new(state);

        let status = service.status(Some("not-a-real-session")).unwrap();
        assert!(status.locked);
        assert!(status.active_compartment.is_none());
        assert!(status.unlocked_compartments.is_empty());
    }

    #[tokio::test]
    async fn set_api_key_requires_session() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state);

        let error = service
            .set_api_key(
                None,
                sigillum_api::KeyValueRequest {
                    key: "github".into(),
                    value: Some("token".into()),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoke_session_only_removes_target_token() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        state.unlock_compartment(0, [9u8; 32], meta(0, 1, "default"));
        let session_a = state.create_session(Some(0));
        let session_b = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let result = service.revoke_session(Some(&session_a)).await.unwrap();
        assert_eq!(result.status, "revoked");
        assert!(!state.verify_token(&session_a));
        assert!(state.verify_token(&session_b));
        assert!(state.is_unlocked());
    }

    #[test]
    fn diagnostics_reports_runtime_metadata() {
        let dir = TempDir::new().unwrap();
        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        state.unlock_compartment(0, [4u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        let diagnostics = service.diagnostics(Some(&session)).unwrap();
        assert_eq!(diagnostics.status, "ok");
        assert_eq!(diagnostics.unlock_scope, "process-global");
        assert_eq!(diagnostics.session_scope, "per-session-active-compartment");
        assert_eq!(diagnostics.unlocked_compartment_count, 1);
        assert_eq!(diagnostics.active_session_count, 1);
        assert_eq!(diagnostics.default_active_compartment_id, Some(0));
        assert_eq!(diagnostics.max_unlocked_threshold, Some(1));
        assert_eq!(diagnostics.pending_operation_count, 0);
        assert_eq!(diagnostics.queue_job_count, 0);
        assert_eq!(diagnostics.blocked_queue_job_count, 0);
        assert_eq!(diagnostics.retrying_queue_job_count, 0);
        assert_eq!(diagnostics.failed_queue_job_count, 0);
        assert_eq!(diagnostics.deferred_queue_job_count, 0);
        assert_eq!(diagnostics.startup_interrupted_operation_count, 0);
        assert_eq!(diagnostics.startup_recovered_operation_count, 0);
        assert_eq!(diagnostics.startup_unresolved_operation_count, 0);
        assert_eq!(diagnostics.startup_recovered_queue_job_count, 0);
        assert_eq!(diagnostics.startup_reconciled_deposit_count, 0);
        assert_eq!(diagnostics.runtime_policy.queue_default_process_limit, 50);
        assert_eq!(diagnostics.runtime_policy.queue_max_process_limit, 500);
        assert_eq!(
            diagnostics.runtime_policy.deposit_default_refresh_limit,
            100
        );
        assert_eq!(diagnostics.runtime_policy.deposit_max_refresh_limit, 500);
        assert_eq!(diagnostics.runtime_policy.audit_default_limit, 25);
        assert_eq!(diagnostics.runtime_policy.audit_max_limit, 200);
        assert_eq!(diagnostics.runtime_policy.queue_retry_base_delay_secs, 5);
        assert_eq!(diagnostics.runtime_policy.queue_retry_max_delay_secs, 300);
        assert_eq!(
            diagnostics
                .runtime_policy
                .provider_balance_observation_concurrency,
            8
        );
        assert_eq!(diagnostics.eth_stealth_deposit_count, 0);
        assert_eq!(diagnostics.funded_eth_stealth_deposit_count, 0);
    }
}
