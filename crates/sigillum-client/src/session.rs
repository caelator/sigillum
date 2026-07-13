//! Session lifecycle, authenticator, and compartment-transition methods.

use std::sync::MutexGuard;
use std::sync::atomic::Ordering;
use std::time::Duration;

use reqwest::Method;
use sigillum_api::request::{
    BiometricEnrollRequest, CompartmentSwitchRequest, Fido2RegisterRequest, Fido2RemoveRequest,
};

use super::{
    BiometricChallengeResponse, BiometricEnrollResponse, ClientError, CompartmentInfo,
    CompartmentListResponse, Fido2DetectResponse, Fido2ListResponse, Fido2RegisterResponse,
    Fido2RemoveResponse, Fido2StatusResponse, SessionRevokeResponse, SigillumClient,
    StatusResponse, SwitchCompartmentResponse, request_session_token,
};

mod establish;
mod lock;

// The daemon retains only the immediate predecessor token for at most 180
// seconds. Keep the response and fallback deadlines comfortably inside that
// grace period even when callers supply a reqwest client with no timeout.
const TRANSITION_GATE_TIMEOUT: Duration = Duration::from_secs(120);
const TRANSITION_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
const FALLBACK_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_STATE_CLEAR: u8 = 0;
const LOCK_STATE_INTENT: u8 = 1;
const LOCK_STATE_UNCONFIRMED: u8 = 2;

impl SigillumClient {
    // ── Session state ──────────────────────────────────────────

    /// Return a clone of the current session token, if any.
    pub fn session_token(&self) -> Option<String> {
        let current = self.session_token_state();
        if self.session_lock_state.load(Ordering::SeqCst) == LOCK_STATE_CLEAR {
            current.clone()
        } else {
            None
        }
    }

    pub fn set_session_token(&self, token: impl Into<String>) {
        let mut current = self.session_token_state();
        match self.session_lock_state.load(Ordering::SeqCst) {
            LOCK_STATE_CLEAR => *current = Some(token.into()),
            LOCK_STATE_INTENT => {}
            _ => *current = None,
        }
    }

    pub fn clear_session_token(&self) {
        let mut current = self.session_token_state();
        if self.session_lock_state.load(Ordering::SeqCst) != LOCK_STATE_INTENT {
            *current = None;
        }
    }

    pub(super) fn replace_session_token_if_current(
        &self,
        expected: Option<&str>,
        replacement: Option<String>,
    ) -> bool {
        let mut current = self.session_token_state();
        if self.session_lock_state.load(Ordering::SeqCst) != LOCK_STATE_CLEAR
            || current.as_deref() != expected
        {
            return false;
        }
        *current = replacement;
        true
    }

    fn revoke_session_token_for_lock(&self, expected: Option<&str>) -> bool {
        let mut current = self.session_token_state();
        if self.session_lock_state.load(Ordering::SeqCst) != LOCK_STATE_INTENT
            || current.as_deref() != expected
        {
            return false;
        }
        *current = None;
        true
    }

    fn adopt_validated_session_token(
        &self,
        expected: Option<&str>,
        replacement: &str,
    ) -> Result<(), ClientError> {
        if !is_canonical_session_token(replacement) {
            return Err(ClientError::InvalidSessionTransition(
                "daemon returned a non-canonical session token".into(),
            ));
        }
        self.replace_session_token_if_current(expected, Some(replacement.to_owned()))
            .then_some(())
            .ok_or(ClientError::SessionContextChanged)
    }

    fn adopt_session_token_for_pending_lock(&self, replacement: &str) -> Result<(), ClientError> {
        if !is_canonical_session_token(replacement) {
            return Err(ClientError::InvalidSessionTransition(
                "daemon returned a non-canonical session token".into(),
            ));
        }
        let mut current = self.session_token_state();
        if current.is_some() || self.session_lock_state.load(Ordering::SeqCst) != LOCK_STATE_INTENT
        {
            return Err(ClientError::SessionContextChanged);
        }
        *current = Some(replacement.to_owned());
        Ok(())
    }

    pub(super) fn clear_session_token_if_current(
        &self,
        expected: Option<&str>,
    ) -> Result<(), ClientError> {
        self.replace_session_token_if_current(expected, None)
            .then_some(())
            .ok_or(ClientError::SessionContextChanged)
    }

    pub(super) fn ensure_session_requests_allowed(&self) -> Result<(), ClientError> {
        match self.session_lock_state.load(Ordering::SeqCst) {
            LOCK_STATE_CLEAR => Ok(()),
            LOCK_STATE_INTENT => Err(ClientError::SessionContextChanged),
            _ => Err(ClientError::SessionStateUnconfirmed),
        }
    }

    fn begin_lock_intent(&self) -> Result<(), ClientError> {
        let _current = self.session_token_state();
        match self.session_lock_state.compare_exchange(
            LOCK_STATE_CLEAR,
            LOCK_STATE_INTENT,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {}
            Err(LOCK_STATE_UNCONFIRMED) => return Err(ClientError::SessionStateUnconfirmed),
            Err(_) => return Err(ClientError::SessionContextChanged),
        }
        self.session_boundary_generation
            .fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn clear_lock_boundary(&self) {
        self.session_lock_state
            .store(LOCK_STATE_CLEAR, Ordering::SeqCst);
    }

    fn confirm_lock_boundary(&self) {
        let mut current = self.session_token_state();
        *current = None;
        self.session_lock_state
            .store(LOCK_STATE_CLEAR, Ordering::SeqCst);
    }

    fn mark_lock_unconfirmed(&self) {
        let mut current = self.session_token_state();
        self.session_lock_state
            .store(LOCK_STATE_UNCONFIRMED, Ordering::SeqCst);
        *current = None;
    }

    pub(super) fn session_boundary_generation(&self) -> u64 {
        self.session_boundary_generation.load(Ordering::SeqCst)
    }

    pub(super) fn ensure_session_boundary_generation(
        &self,
        expected: u64,
    ) -> Result<(), ClientError> {
        (self.session_boundary_generation() == expected)
            .then_some(())
            .ok_or(ClientError::SessionContextChanged)
    }

    pub(super) async fn acquire_session_transition(
        &self,
        operation: &str,
    ) -> Result<tokio::sync::RwLockWriteGuard<'_, ()>, ClientError> {
        tokio::time::timeout(TRANSITION_GATE_TIMEOUT, self.session_transition.write())
            .await
            .map_err(|_| {
                ClientError::InvalidSessionTransition(format!(
                    "timed out waiting for the session transition gate; no {operation} request was sent"
                ))
            })
    }

    /// Restore the only safe client-side invariant after a panic: no cached
    /// authentication. A later explicit unlock or token assignment may then
    /// establish a fresh session.
    fn session_token_state(&self) -> MutexGuard<'_, Option<String>> {
        match self.session_token.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!(
                    "client session-token mutex poisoned; clearing cached authentication state"
                );
                let mut guard = poisoned.into_inner();
                *guard = None;
                self.session_token.clear_poison();
                guard
            }
        }
    }

    fn raw_session_token(&self) -> Option<String> {
        self.session_token_state().clone()
    }

    // ── Lifecycle ──────────────────────────────────────────────

    /// Query daemon status (locked / unlocked, active compartment, etc.).
    pub async fn status(&self) -> Result<StatusResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/status");
        self.send(builder).await
    }

    pub async fn biometric_challenge(&self) -> Result<BiometricChallengeResponse, ClientError> {
        let builder = self.request(Method::POST, "/api/biometric/challenge");
        self.send(builder).await
    }

    pub async fn biometric_enroll(
        &self,
        request: BiometricEnrollRequest,
    ) -> Result<BiometricEnrollResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/biometric/enroll")
            .json(&request);
        self.send(builder).await
    }

    pub async fn revoke_session(&self) -> Result<SessionRevokeResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        self.ensure_session_requests_allowed()?;
        let _transition = self.acquire_session_transition("session revoke").await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        let builder = self.request(Method::POST, "/api/session/revoke");
        let (response, expected_token): (SessionRevokeResponse, _) =
            self.send_with_session_context_guarded(builder).await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        if response.status != "revoked" || !response.requires_reauth {
            return Err(ClientError::InvalidSessionTransition(format!(
                "session revoke returned status {:?} with requires_reauth={}",
                response.status, response.requires_reauth
            )));
        }
        self.clear_session_token_if_current(expected_token.as_deref())?;
        Ok(response)
    }

    // ── Compartments ───────────────────────────────────────────

    pub async fn list_compartments(&self) -> Result<Vec<CompartmentInfo>, ClientError> {
        let builder = self.request(Method::GET, "/api/compartment/list");
        Ok(self
            .send::<CompartmentListResponse>(builder)
            .await?
            .compartments)
    }

    /// Switch compartments through an owned worker so dropping the caller's
    /// future cannot abandon a daemon-side T -> T2 commit. The worker either
    /// adopts the validated replacement token or runs fail-closed containment.
    pub async fn switch_compartment(
        &self,
        id: usize,
    ) -> Result<SwitchCompartmentResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        let client = self.clone();
        await_session_worker(
            "compartment switch",
            tokio::spawn(async move {
                client
                    .switch_compartment_owned(id, boundary_generation)
                    .await
            }),
        )
        .await
    }

    async fn switch_compartment_owned(
        &self,
        id: usize,
        boundary_generation: u64,
    ) -> Result<SwitchCompartmentResponse, ClientError> {
        let _transition = self
            .acquire_session_transition("compartment switch")
            .await?;
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        let builder = self
            .request(Method::POST, "/api/compartment/switch")
            .json(&CompartmentSwitchRequest { id });
        let request = builder.build()?;
        let expected_token = request_session_token(&request).ok_or_else(|| {
            ClientError::InvalidSessionTransition(
                "compartment switch request had no session token".into(),
            )
        })?;

        let response = match tokio::time::timeout(
            TRANSITION_RESPONSE_TIMEOUT,
            self.send_built_with_session_context::<SwitchCompartmentResponse>(
                request,
                Some(&expected_token),
            ),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error @ ClientError::Api { status, .. })) if status.is_client_error() => {
                return Err(error);
            }
            Ok(Err(error)) => {
                return Err(self
                    .contain_ambiguous_session_transition(
                        Some(&expected_token),
                        format!("daemon response could not be validated: {error}"),
                    )
                    .await);
            }
            Err(_) => {
                return Err(self
                    .contain_ambiguous_session_transition(
                        Some(&expected_token),
                        "daemon response exceeded the compartment-switch deadline",
                    )
                    .await);
            }
        };
        if response.status != "switched" {
            return Err(self
                .contain_ambiguous_session_transition(
                    Some(&expected_token),
                    format!("compartment switch returned status {:?}", response.status),
                )
                .await);
        }
        if response.compartment_id != id {
            return Err(self
                .contain_ambiguous_session_transition(
                    Some(&expected_token),
                    format!(
                        "compartment switch returned id {}, expected {id}",
                        response.compartment_id
                    ),
                )
                .await);
        }
        if !is_canonical_session_token(&response.session_token)
            || response.session_token == expected_token
        {
            return Err(self
                .contain_ambiguous_session_transition(
                    Some(&expected_token),
                    "compartment switch did not return a distinct canonical token",
                )
                .await);
        }
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(boundary_generation)?;
        self.adopt_validated_session_token(Some(&expected_token), &response.session_token)?;
        Ok(response)
    }

    // ── FIDO2 ──────────────────────────────────────────────────

    pub async fn fido2_status(&self) -> Result<Fido2StatusResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/status");
        self.send(builder).await
    }

    pub async fn fido2_detect(&self) -> Result<Fido2DetectResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/detect");
        self.send(builder).await
    }

    pub async fn fido2_list_keys(&self) -> Result<Fido2ListResponse, ClientError> {
        let builder = self.request(Method::GET, "/api/fido2/list");
        self.send(builder).await
    }

    pub async fn fido2_register(
        &self,
        request: Fido2RegisterRequest,
    ) -> Result<Fido2RegisterResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/register")
            .json(&request);
        self.send(builder).await
    }

    pub async fn fido2_remove(
        &self,
        request: Fido2RemoveRequest,
    ) -> Result<Fido2RemoveResponse, ClientError> {
        let builder = self
            .request(Method::POST, "/api/fido2/remove")
            .json(&request);
        self.send(builder).await
    }
}

async fn await_session_worker<T: Send + 'static>(
    operation: &'static str,
    worker: tokio::task::JoinHandle<Result<T, ClientError>>,
) -> Result<T, ClientError> {
    worker.await.map_err(|error| {
        ClientError::InvalidSessionTransition(format!("{operation} worker failed: {error}"))
    })?
}

fn is_canonical_session_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
