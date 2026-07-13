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
//! - **queue** — enqueue façade, payload construction, processing loop, sweep
//!   execution, and retry/recovery state helpers
//! - **evm** — on-chain balance observation, transaction construction, and
//!   provider RPC transport
//! - **profiles** — EVM provider and wallet profile CRUD, profile-backed send
//!   resolution, and provider/wallet lookup helpers
//! - **maintenance** — compound refresh + queue-drain cycles
//! - **backup / recovery** — encrypted snapshot export/restore
//! - **transit** — inter-compartment secret push
//! - **observability** — status and diagnostics endpoints
//! - **selfcheck** — operator self-check verifying configured providers,
//!   wallets, policy, and FIDO2 keys are well-formed and functioning
//! - **error** — [`ServiceError`] type with HTTP status code mapping
//! - **helpers** — shared hex decoding, u256 arithmetic, timestamps

mod backup;
pub(crate) mod chains;
mod compartments;
mod deposits;
mod error;
mod evm;
mod fido2;
mod generate;
pub(crate) mod helpers;
mod inventory;
mod lifecycle;
mod maintenance;
mod observability;
mod profiles;
mod queue;
mod recovery;
mod secrets;
mod selfcheck;
pub(crate) mod transaction_policy;
mod transit;
pub(crate) mod unlock;
mod wallets;

use std::sync::Arc;

use sigillum_core::VaultError;

use crate::AppState;
use crate::audit_log::AuditEventSpec;
use crate::operations::{OperationGuard, PendingOperationSpec};
use crate::state::LockTokenAuthorization;

pub(crate) use error::{ServiceError, ServiceResult};

pub(crate) mod capability_scopes {
    pub const WALLET_PROFILES_READ: &str = "wallet_profiles:read";
    pub const EVM_PROVIDERS_READ: &str = "evm_providers:read";
    pub const DEPOSITS_CREATE: &str = "deposits:create";
    pub const DEPOSITS_READ: &str = "deposits:read";
    pub const DEPOSITS_DELETE: &str = "deposits:delete";
    pub const DEPOSITS_REFRESH: &str = "deposits:refresh";
    pub const QUEUE_ENQUEUE_SWEEP: &str = "queue:enqueue-sweep";

    pub fn is_known(scope: &str) -> bool {
        matches!(
            scope,
            WALLET_PROFILES_READ
                | EVM_PROVIDERS_READ
                | DEPOSITS_CREATE
                | DEPOSITS_READ
                | DEPOSITS_DELETE
                | DEPOSITS_REFRESH
                | QUEUE_ENQUEUE_SWEEP
        )
    }
}

const DEFAULT_CAPABILITY_SESSION_TTL_SECS: u64 = 60 * 60;

/// Require a valid full daemon session at boundaries that do not expose a
/// `SigillumService` instance (for example biometric enrollment).
pub(crate) fn require_full_session_token<'a>(
    state: &AppState,
    token: Option<&'a str>,
) -> ServiceResult<&'a str> {
    if state.is_locking() {
        return Err(ServiceError::locked("Daemon is locking."));
    }
    let token = match token {
        Some(token) if state.verify_token(token) => token,
        _ => {
            return Err(ServiceError::unauthorized(
                "Invalid or missing session token.",
            ));
        }
    };
    if state.session_is_full(token) {
        Ok(token)
    } else {
        Err(ServiceError::forbidden(
            "A full daemon session is required for this operation.",
        ))
    }
}

/// Façade over all vault operations.
///
/// Holds an `Arc<AppState>` and delegates to domain-specific methods defined
/// across sub-modules. Every public method follows the same contract:
/// validate the session token → acquire the operation guard if mutating →
/// perform the operation → record an audit event → return the typed response.
pub(crate) struct SigillumService {
    state: Arc<AppState>,
}

/// Request-local session state captured immediately before a mutation waits
/// for the global operation mutex.
///
/// A queued request may outlive session revocation or an atomic token rotation
/// caused by a compartment switch. Token validity and the active compartment
/// must therefore still match once the request reaches the serialized mutation
/// section.
struct SessionOperationContext {
    token: String,
    active_compartment_id: Option<usize>,
}

/// Lock generation captured when an unlock request is admitted.
///
/// Unlike authenticated mutations, unlocks do not have an existing session to
/// revalidate. The generation proves that no lock began while an unlock was
/// deriving credentials, waiting for the operation mutex, or talking to a
/// hardware key.
#[derive(Clone, Copy)]
struct UnlockOperationContext {
    lock_generation: u64,
}

impl SigillumService {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Verify that the request carries any valid session token.
    ///
    /// Capability-aware entry points must use this primitive and then enforce
    /// their specific scope. All other service methods go through
    /// [`Self::require_session`], which deliberately requires a full session.
    fn require_authenticated_session<'a>(&self, token: Option<&'a str>) -> ServiceResult<&'a str> {
        if self.state.is_locking() {
            return Err(ServiceError::locked("Daemon is locking."));
        }
        match token {
            Some(token) if self.state.verify_token(token) => Ok(token),
            _ => Err(ServiceError::unauthorized(
                "Invalid or missing session token.",
            )),
        }
    }

    /// Require a full daemon session.
    ///
    /// This is the default authorization boundary for every service method
    /// that is not explicitly capability-scoped.
    fn require_session<'a>(&self, token: Option<&'a str>) -> ServiceResult<&'a str> {
        require_full_session_token(&self.state, token)
    }

    /// Require the authentication accepted only by process-global Lock.
    ///
    /// A compartment switch rotates the bearer token before returning its
    /// response. If that response is delayed, the immediately preceding full
    /// token remains a bounded, lock-only capability so the caller can still
    /// fail closed. No other route calls this verifier.
    fn require_lock_session<'a>(&self, token: Option<&'a str>) -> ServiceResult<&'a str> {
        if self.state.is_locking() {
            return Err(ServiceError::locked("Daemon is locking."));
        }
        let Some(token) = token else {
            return Err(ServiceError::unauthorized(
                "Invalid or missing session token.",
            ));
        };
        match self.state.authorize_lock_token(token) {
            LockTokenAuthorization::FullOrRetired => Ok(token),
            LockTokenAuthorization::Capability => Err(ServiceError::forbidden(
                "A full daemon session is required for this operation.",
            )),
            LockTokenAuthorization::Invalid => Err(ServiceError::unauthorized(
                "Invalid or missing session token.",
            )),
        }
    }

    fn require_full_session<'a>(&self, token: Option<&'a str>) -> ServiceResult<&'a str> {
        self.require_session(token)
    }

    fn require_scope<'a>(
        &self,
        token: Option<&'a str>,
        scope: &'static str,
    ) -> ServiceResult<&'a str> {
        let token = self.require_authenticated_session(token)?;
        if self.state.session_has_scope(token, scope) {
            Ok(token)
        } else {
            Err(ServiceError::forbidden(format!(
                "Missing daemon capability scope: {scope}"
            )))
        }
    }

    /// Capture the authenticated session context that a mutation is about to
    /// use. Endpoint-specific full-session/capability checks must run before
    /// this; this helper preserves that decision while making the subsequent
    /// operation-mutex wait race-safe.
    fn capture_session_operation_context(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<SessionOperationContext> {
        let token = self.require_authenticated_session(token)?.to_owned();
        let active_compartment_id = self.state.active_compartment_id_for(&token);
        Ok(SessionOperationContext {
            token,
            active_compartment_id,
        })
    }

    /// Acquire the serialized mutation boundary, then revalidate both the
    /// session and its active compartment while holding it. A lock request may
    /// latch `Locking`, a peer may revoke the token, or a same-token request may
    /// switch compartments while this request is queued; all three cases fail
    /// closed before mutation code can run.
    async fn acquire_session_operation<'a>(
        &'a self,
        context: &SessionOperationContext,
    ) -> ServiceResult<tokio::sync::MutexGuard<'a, ()>> {
        let guard = self.state.operation_guard().await;
        self.require_authenticated_session(Some(&context.token))?;
        if self.state.active_compartment_id_for(&context.token) != context.active_compartment_id {
            return Err(ServiceError::conflict(
                "Session compartment changed while the mutation was waiting.",
            ));
        }
        Ok(guard)
    }

    /// Admit a new unlock only while no lock is latched and bind it to the
    /// current monotonic lock generation.
    fn capture_unlock_operation_context(&self) -> ServiceResult<UnlockOperationContext> {
        self.state
            .unlock_generation_if_ready()
            .map(|lock_generation| UnlockOperationContext { lock_generation })
            .ok_or_else(|| ServiceError::locked("Daemon is locking."))
    }

    /// Wait for the serialized mutation boundary, then reject an unlock if a
    /// lock began at any point since admission. The final key/session install
    /// additionally uses [`Self::commit_unlock`] to close the small race
    /// between this recheck and the actual commit.
    async fn acquire_unlock_operation<'a>(
        &'a self,
        context: &UnlockOperationContext,
    ) -> ServiceResult<tokio::sync::MutexGuard<'a, ()>> {
        let guard = self.state.operation_guard().await;
        if self.state.unlock_generation_if_ready() != Some(context.lock_generation) {
            return Err(ServiceError::locked(
                "A lock began while the unlock request was in progress.",
            ));
        }
        Ok(guard)
    }

    /// Atomically install unlock state only if the admission generation still
    /// wins against `begin_locking`.
    fn commit_unlock<R>(
        &self,
        context: &UnlockOperationContext,
        commit: impl FnOnce() -> ServiceResult<R>,
    ) -> ServiceResult<R> {
        self.state
            .commit_unlock_if_current(context.lock_generation, commit)
            .ok_or_else(|| {
                ServiceError::locked("A lock began while the unlock request was in progress.")
            })?
    }

    /// Final funds-moving network admission point. This delegates to the
    /// lock-state mutex shared with `begin_locking`, making the decision a
    /// defined ordering boundary rather than a best-effort boolean poll.
    fn admit_broadcast(&self) -> ServiceResult<()> {
        if self.state.admit_broadcast_if_ready() {
            Ok(())
        } else {
            Err(ServiceError::locked(
                "Daemon is locking; transaction broadcast was not admitted.",
            ))
        }
    }

    /// Optionally verify a full session without rejecting unauthenticated callers.
    ///
    /// Capability sessions are intentionally hidden from optional-observability
    /// surfaces such as `/api/status`; those sessions may only use endpoints
    /// guarded by an explicit capability scope.
    fn optional_session<'a>(&self, token: Option<&'a str>) -> Option<&'a str> {
        if self.state.is_locking() {
            return None;
        }
        token.filter(|candidate| {
            self.state.verify_token(candidate) && self.state.session_is_full(candidate)
        })
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
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
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
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
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
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
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

    #[tokio::test]
    async fn queued_session_mutation_rejects_same_token_compartment_change() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [3u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [5u8; 32], meta(1, 2, "secure"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());
        let context = service
            .capture_session_operation_context(Some(&session))
            .unwrap();

        let held_operation = state.operation_guard().await;
        let mut queued = Box::pin(service.acquire_session_operation(&context));
        tokio::select! {
            biased;
            _ = &mut queued => panic!("mutation must wait for the held operation mutex"),
            () = tokio::task::yield_now() => {}
        }

        state.switch_active_for(&session, 1).unwrap();
        drop(held_operation);

        let error = match queued.await {
            Ok(_) => panic!("stale compartment mutation must not be admitted"),
            Err(error) => error,
        };
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
        assert_eq!(state.active_compartment_id_for(&session), Some(1));
    }

    #[tokio::test]
    async fn compartment_switch_rotates_token_and_binds_replacement_to_target() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [3u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [5u8; 32], meta(1, 2, "secure"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let response = service
            .switch_compartment(
                Some(&session),
                sigillum_api::request::CompartmentSwitchRequest { id: 1 },
            )
            .await
            .unwrap();

        assert_ne!(response.session_token, session);
        assert!(!state.verify_token(&session));
        assert!(state.verify_token(&response.session_token));
        assert_eq!(
            state.active_compartment_id_for(&response.session_token),
            Some(1)
        );
        assert_eq!(response.compartment_id, 1);
        assert_eq!(response.compartment_label, "secure");
    }

    #[tokio::test]
    async fn delayed_switch_response_preserves_only_emergency_lock_for_old_token() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [3u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [5u8; 32], meta(1, 2, "secure"));
        let predecessor = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        // Model a committed switch whose response (and therefore replacement
        // token) has not reached the caller yet.
        let response = service
            .switch_compartment(
                Some(&predecessor),
                sigillum_api::request::CompartmentSwitchRequest { id: 1 },
            )
            .await
            .unwrap();
        let successor = response.session_token;

        assert_eq!(state.active_compartment_id_for(&predecessor), None);

        assert_eq!(
            service
                .require_session(Some(&predecessor))
                .unwrap_err()
                .status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            service
                .list_secrets(Some(&predecessor))
                .unwrap_err()
                .status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            service
                .set_secret(
                    Some(&predecessor),
                    sigillum_api::KeyValueRequest {
                        key: "must-not-write".into(),
                        value: Some("secret".into()),
                    },
                )
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            service
                .switch_compartment(
                    Some(&predecessor),
                    sigillum_api::request::CompartmentSwitchRequest { id: 0 },
                )
                .await
                .unwrap_err()
                .status(),
            axum::http::StatusCode::UNAUTHORIZED
        );

        // Successful use of the replacement must not withdraw emergency Lock
        // from a briefly stale sibling tab.
        assert!(state.verify_token(&successor));
        assert!(state.verify_full_or_retired_lock_token(&predecessor));

        let locked = service.lock_all(Some(&predecessor)).await.unwrap();
        assert_eq!(locked.status, "locked");
        assert!(!state.is_unlocked());
        assert!(!state.verify_token(&predecessor));
        assert!(!state.verify_token(&successor));
        assert!(!state.verify_full_or_retired_lock_token(&predecessor));
    }

    #[tokio::test]
    async fn capability_mint_rejects_token_rotated_while_waiting() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [3u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [5u8; 32], meta(1, 2, "secure"));
        let predecessor = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut mint = Box::pin(service.mint_capability_session(
            Some(&predecessor),
            sigillum_api::request::CapabilitySessionRequest {
                scopes: vec![capability_scopes::DEPOSITS_READ.into()],
                ttl_secs: None,
            },
        ));
        tokio::select! {
            biased;
            result = &mut mint => panic!("mint must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
        drop(held_operation);

        let error = mint
            .await
            .expect_err("rotated authentication must not mint a capability");
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(state.session_count(), 1);
        assert!(state.verify_token(&successor));
    }

    #[tokio::test]
    async fn active_capability_session_cannot_lock_daemon() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [8u8; 32], meta(0, 1, "default"));
        let (capability, _) = state.create_capability_session(
            Some(0),
            vec![capability_scopes::DEPOSITS_READ.into()],
            std::time::Duration::from_secs(60),
        );
        let service = SigillumService::new(state.clone());

        let error = service
            .lock_all(Some(&capability))
            .await
            .expect_err("capability session must not authorize process-global Lock");

        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
        assert!(state.is_unlocked());
        assert!(state.verify_token(&capability));
    }

    #[tokio::test]
    async fn lock_latch_preempts_capability_mint_waiting_for_operation_mutex() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [8u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut mint = Box::pin(service.mint_capability_session(
            Some(&session),
            sigillum_api::request::CapabilitySessionRequest {
                scopes: vec![capability_scopes::DEPOSITS_READ.into()],
                ttl_secs: None,
            },
        ));
        tokio::select! {
            biased;
            result = &mut mint => panic!("mint must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let mut lock = Box::pin(service.lock_all(Some(&session)));
        tokio::select! {
            biased;
            result = &mut lock => panic!("lock must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert!(state.is_locking(), "lock intent must be latched");

        drop(held_operation);
        let mint_error = mint
            .await
            .expect_err("latched lock must reject a queued capability mint");
        assert_eq!(mint_error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(lock.await.unwrap().status, "locked");
        assert_eq!(state.session_count(), 0);
        assert!(!state.is_unlocked());
    }

    #[tokio::test]
    async fn fido2_set_pin_rejects_token_rotated_while_waiting() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        std::fs::write(state.base_dir.join(".initialized"), b"1").unwrap();
        state.unlock_compartment(0, [3u8; 32], meta(0, 1, "daily"));
        state.unlock_compartment(1, [5u8; 32], meta(1, 2, "secure"));
        let predecessor = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut set_pin = Box::pin(service.fido2_set_pin(
            Some(&predecessor),
            sigillum_api::request::Fido2SetPinRequest {
                new_pin: "1234".into(),
            },
        ));
        tokio::select! {
            biased;
            result = &mut set_pin => panic!("PIN mutation must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let successor = state.rotate_session_active_for(&predecessor, 1).unwrap();
        drop(held_operation);

        let error = set_pin
            .await
            .expect_err("rotated authentication must not mutate a hardware key");
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(state.verify_token(&successor));
    }

    #[tokio::test]
    async fn lock_latch_preempts_fido2_set_pin_waiting_for_operation_mutex() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        std::fs::write(state.base_dir.join(".initialized"), b"1").unwrap();
        state.unlock_compartment(0, [8u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut set_pin = Box::pin(service.fido2_set_pin(
            Some(&session),
            sigillum_api::request::Fido2SetPinRequest {
                new_pin: "1234".into(),
            },
        ));
        tokio::select! {
            biased;
            result = &mut set_pin => panic!("PIN mutation must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let mut lock = Box::pin(service.lock_all(Some(&session)));
        tokio::select! {
            biased;
            result = &mut lock => panic!("lock must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert!(state.is_locking(), "lock intent must be latched");

        drop(held_operation);
        let pin_error = set_pin
            .await
            .expect_err("latched lock must reject a queued hardware mutation");
        assert_eq!(pin_error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(lock.await.unwrap().status, "locked");
        assert!(!state.is_unlocked());
    }

    #[tokio::test]
    async fn unauthenticated_fido2_set_pin_fails_if_initialization_wins_while_waiting() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut set_pin = Box::pin(service.fido2_set_pin(
            None,
            sigillum_api::request::Fido2SetPinRequest {
                new_pin: "1234".into(),
            },
        ));
        tokio::select! {
            biased;
            result = &mut set_pin => panic!("PIN setup must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        std::fs::write(state.base_dir.join(".initialized"), b"1").unwrap();
        drop(held_operation);

        let error = set_pin
            .await
            .expect_err("an initialized daemon must not accept unauthenticated PIN setup");
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn lock_latch_preempts_already_queued_authenticated_mutation() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [8u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state.clone());
        let context = service
            .capture_session_operation_context(Some(&session))
            .unwrap();

        let held_operation = state.operation_guard().await;
        let mut queued_mutation = Box::pin(service.acquire_session_operation(&context));
        tokio::select! {
            biased;
            _ = &mut queued_mutation => panic!("mutation must wait for the held operation mutex"),
            () = tokio::task::yield_now() => {}
        }

        let mut lock = Box::pin(service.lock_all(Some(&session)));
        tokio::select! {
            biased;
            _ = &mut lock => panic!("lock must wait for the held operation mutex"),
            () = tokio::task::yield_now() => {}
        }

        assert!(
            state.is_locking(),
            "lock intent must be latched before drain"
        );
        let error = service.require_session(Some(&session)).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::LOCKED);

        drop(held_operation);
        let mutation_error = match queued_mutation.await {
            Ok(_) => panic!("latched lock must reject a previously queued mutation"),
            Err(error) => error,
        };
        assert_eq!(mutation_error.status(), axum::http::StatusCode::LOCKED);

        let response = lock.await.unwrap();
        assert_eq!(response.status, "locked");
        assert!(!state.is_unlocked());
        assert!(!state.verify_token(&session));
    }

    #[tokio::test]
    async fn lock_latch_invalidates_in_flight_passphrase_unlock_generation() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        let service = SigillumService::new(state.clone());
        let passphrase = "race-proof-passphrase";
        let initialized = service
            .init_compartment(
                None,
                sigillum_api::request::CompartmentInitRequest {
                    id: 0,
                    passphrase: passphrase.into(),
                    label: Some("default".into()),
                    threshold: Some(1),
                },
            )
            .await
            .unwrap();

        let held_operation = state.operation_guard().await;
        let mut unlock = Box::pin(service.unlock_with_passphrase(
            None,
            sigillum_api::request::PassphraseRequest {
                passphrase: passphrase.into(),
            },
        ));
        tokio::select! {
            biased;
            result = &mut unlock => panic!("unlock must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        let mut lock = Box::pin(service.lock_all(Some(&initialized.session_token)));
        tokio::select! {
            biased;
            result = &mut lock => panic!("lock must wait for the held operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert!(state.is_locking(), "lock intent must be latched");

        drop(held_operation);
        let unlock_error = unlock
            .await
            .expect_err("pre-latch unlock must not reopen the daemon");
        assert_eq!(unlock_error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(lock.await.unwrap().status, "locked");
        assert!(!state.is_unlocked());
        assert_eq!(state.session_count(), 0);

        // A genuinely new request after the completed lock captures the new
        // generation and remains a valid way to unlock.
        let unlocked = service
            .unlock_with_passphrase(
                None,
                sigillum_api::request::PassphraseRequest {
                    passphrase: passphrase.into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(unlocked.status, "unlocked");
        assert!(state.verify_token(&unlocked.session_token));
    }

    #[test]
    fn diagnostics_reports_runtime_metadata() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
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
        assert_eq!(diagnostics.operator_action_required_queue_job_count, 0);
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
        assert_eq!(
            diagnostics.runtime_policy.receiving_refresh_address_cap,
            200
        );
        assert_eq!(diagnostics.eth_stealth_deposit_count, 0);
        assert_eq!(diagnostics.funded_eth_stealth_deposit_count, 0);
    }
}
