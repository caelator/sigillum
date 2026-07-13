//! Cancellation-safe workers for transitions that establish a daemon session.

use reqwest::Method;
use serde::de::DeserializeOwned;
use sigillum_api::request::{
    BiometricUnlockRequest, Fido2SetupRequest, Fido2UnlockRequest, PassphraseRequest,
};

use super::{TRANSITION_RESPONSE_TIMEOUT, await_session_worker};
use crate::{
    ClientError, Fido2SetupResponse, SigillumClient, UnlockResponse, request_session_token,
};

mod validate;

impl SigillumClient {
    pub async fn unlock_with_passphrase(
        &self,
        passphrase: &str,
    ) -> Result<UnlockResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        let client = self.clone();
        let passphrase = passphrase.to_owned();
        await_session_worker(
            "passphrase unlock",
            tokio::spawn(async move {
                client
                    .unlock_with_passphrase_owned(passphrase, boundary_generation)
                    .await
            }),
        )
        .await
    }

    async fn unlock_with_passphrase_owned(
        &self,
        passphrase: String,
        boundary_generation: u64,
    ) -> Result<UnlockResponse, ClientError> {
        let _transition = self.acquire_session_transition("passphrase unlock").await?;
        self.ensure_establishing_boundary(boundary_generation)?;
        let builder = self
            .request(Method::POST, "/api/unlock")
            .json(&PassphraseRequest { passphrase });
        let (response, expected_token) = self
            .send_establishing_request(builder, "passphrase unlock")
            .await?;
        self.finish_unlock_response(response, expected_token, "passphrase", boundary_generation)
            .await
    }

    pub async fn biometric_unlock(
        &self,
        payload_hex: String,
    ) -> Result<UnlockResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        let client = self.clone();
        await_session_worker(
            "biometric unlock",
            tokio::spawn(async move {
                client
                    .biometric_unlock_owned(payload_hex, boundary_generation)
                    .await
            }),
        )
        .await
    }

    async fn biometric_unlock_owned(
        &self,
        payload_hex: String,
        boundary_generation: u64,
    ) -> Result<UnlockResponse, ClientError> {
        let _transition = self.acquire_session_transition("biometric unlock").await?;
        self.ensure_establishing_boundary(boundary_generation)?;
        let builder = self
            .request(Method::POST, "/api/biometric/unlock")
            .json(&BiometricUnlockRequest { payload_hex });
        let (response, expected_token) = self
            .send_establishing_request(builder, "biometric unlock")
            .await?;
        self.finish_unlock_response(response, expected_token, "biometric", boundary_generation)
            .await
    }

    pub async fn fido2_unlock(
        &self,
        request: Fido2UnlockRequest,
    ) -> Result<UnlockResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        let client = self.clone();
        await_session_worker(
            "FIDO2 unlock",
            tokio::spawn(async move {
                client
                    .fido2_unlock_owned(request, boundary_generation)
                    .await
            }),
        )
        .await
    }

    async fn fido2_unlock_owned(
        &self,
        request: Fido2UnlockRequest,
        boundary_generation: u64,
    ) -> Result<UnlockResponse, ClientError> {
        let _transition = self.acquire_session_transition("FIDO2 unlock").await?;
        self.ensure_establishing_boundary(boundary_generation)?;
        let builder = self
            .request(Method::POST, "/api/fido2/unlock")
            .json(&request);
        let (response, expected_token) = self
            .send_establishing_request(builder, "FIDO2 unlock")
            .await?;
        self.finish_unlock_response(response, expected_token, "fido2", boundary_generation)
            .await
    }

    pub async fn fido2_setup(
        &self,
        request: Fido2SetupRequest,
    ) -> Result<Fido2SetupResponse, ClientError> {
        let boundary_generation = self.session_boundary_generation();
        let client = self.clone();
        await_session_worker(
            "FIDO2 setup",
            tokio::spawn(
                async move { client.fido2_setup_owned(request, boundary_generation).await },
            ),
        )
        .await
    }

    async fn fido2_setup_owned(
        &self,
        request: Fido2SetupRequest,
        boundary_generation: u64,
    ) -> Result<Fido2SetupResponse, ClientError> {
        let _transition = self.acquire_session_transition("FIDO2 setup").await?;
        self.ensure_establishing_boundary(boundary_generation)?;
        let expected_compartments = request.compartments.len();
        let builder = self
            .request(Method::POST, "/api/fido2/setup")
            .json(&request);
        let (response, expected_token): (Fido2SetupResponse, _) = self
            .send_establishing_request(builder, "FIDO2 setup")
            .await?;
        if let Err(cause) = validate::fido2_setup(&response, expected_compartments) {
            return Err(self
                .contain_ambiguous_session_transition(expected_token.as_deref(), cause)
                .await);
        }
        self.finish_establishing_response(
            response,
            expected_token,
            boundary_generation,
            |response| &response.session_token,
        )
    }

    async fn send_establishing_request<T: DeserializeOwned>(
        &self,
        builder: reqwest::RequestBuilder,
        operation: &'static str,
    ) -> Result<(T, Option<String>), ClientError> {
        let request = builder.build()?;
        let expected_token = request_session_token(&request);
        match tokio::time::timeout(
            TRANSITION_RESPONSE_TIMEOUT,
            self.send_built_with_session_context(request, expected_token.as_deref()),
        )
        .await
        {
            Ok(Ok(response)) => Ok((response, expected_token)),
            Ok(Err(error @ ClientError::Api { status, .. })) if status.is_client_error() => {
                Err(error)
            }
            Ok(Err(error)) => Err(self
                .contain_ambiguous_session_transition(
                    expected_token.as_deref(),
                    format!("{operation} response could not be validated: {error}"),
                )
                .await),
            Err(_) => Err(self
                .contain_ambiguous_session_transition(
                    expected_token.as_deref(),
                    format!("{operation} exceeded the session-transition deadline"),
                )
                .await),
        }
    }

    async fn finish_unlock_response(
        &self,
        response: UnlockResponse,
        expected_token: Option<String>,
        expected_method: &'static str,
        boundary_generation: u64,
    ) -> Result<UnlockResponse, ClientError> {
        if let Err(cause) = validate::unlock(&response, expected_method) {
            return Err(self
                .contain_ambiguous_session_transition(expected_token.as_deref(), cause)
                .await);
        }
        self.finish_establishing_response(
            response,
            expected_token,
            boundary_generation,
            |response| &response.session_token,
        )
    }

    fn ensure_establishing_boundary(&self, generation: u64) -> Result<(), ClientError> {
        self.ensure_session_requests_allowed()?;
        self.ensure_session_boundary_generation(generation)
    }

    fn finish_establishing_response<T>(
        &self,
        response: T,
        expected_token: Option<String>,
        boundary_generation: u64,
        response_token: impl FnOnce(&T) -> &str,
    ) -> Result<T, ClientError> {
        let token = response_token(&response);
        let boundary_state = self.ensure_session_requests_allowed().err();
        let generation_changed = self
            .ensure_session_boundary_generation(boundary_generation)
            .is_err();

        if boundary_state.is_none() && !generation_changed {
            self.adopt_validated_session_token(expected_token.as_deref(), token)?;
            return Ok(response);
        }

        // A no-token establishment already in flight when Lock began may
        // adopt T2 only inside the writer gate. The waiting Lock worker then
        // consumes T2, while this caller observes only a changed boundary.
        if expected_token.is_none()
            && matches!(boundary_state, Some(ClientError::SessionContextChanged))
        {
            self.adopt_session_token_for_pending_lock(token)?;
            return Err(ClientError::SessionContextChanged);
        }

        Err(boundary_state.unwrap_or(ClientError::SessionContextChanged))
    }
}
