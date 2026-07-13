//! Process-global Lock boundary and ambiguous-transition containment.

use reqwest::StatusCode;

use super::{FALLBACK_LOCK_TIMEOUT, await_session_worker};
use crate::{ClientError, GenericStatusResponse, SigillumClient};

impl SigillumClient {
    pub(super) async fn contain_ambiguous_session_transition(
        &self,
        expected_token: Option<&str>,
        cause: impl Into<String>,
    ) -> ClientError {
        let cause = cause.into();
        if let Err(error) = self.begin_lock_intent() {
            return error;
        }

        // Revoke only the authority carried by the failed request; never
        // substitute a newer explicit owner as fallback Lock authority.
        if !self.revoke_session_token_for_lock(expected_token) {
            self.clear_lock_boundary();
            return ClientError::SessionContextChanged;
        }

        let Some(lock_token) = expected_token else {
            self.mark_lock_unconfirmed();
            return ClientError::SessionTransitionLockUnconfirmed(format!(
                "{cause}; no captured session token was available for fallback Lock"
            ));
        };

        if self.raw_lock_confirmed(lock_token).await {
            self.confirm_lock_boundary();
            ClientError::SessionTransitionLocked(cause)
        } else {
            self.mark_lock_unconfirmed();
            ClientError::SessionTransitionLockUnconfirmed(cause)
        }
    }

    async fn raw_lock_confirmed(&self, expected_token: &str) -> bool {
        let lock_attempt = async {
            let response = self
                .http
                .post(format!("{}/api/lock", self.base_url))
                .bearer_auth(expected_token)
                .send()
                .await?;
            let status = response.status();
            if status == StatusCode::LOCKED {
                return Ok::<bool, reqwest::Error>(true);
            }
            if !status.is_success() {
                return Ok(false);
            }
            let response = response.json::<serde_json::Value>().await?;
            Ok(
                response.get("status").and_then(|value| value.as_str()) == Some("locked")
                    && response.get("error").is_none(),
            )
        };
        matches!(
            tokio::time::timeout(FALLBACK_LOCK_TIMEOUT, lock_attempt).await,
            Ok(Ok(true))
        )
    }

    /// Request process-global Lock through an owned worker. When a token is
    /// available, Lock bypasses the session gate so a hung ordinary request
    /// cannot delay the daemon's preemptive latch.
    pub async fn lock(&self) -> Result<GenericStatusResponse, ClientError> {
        self.begin_lock_intent()?;
        let client = self.clone();
        await_session_worker(
            "Lock",
            tokio::spawn(async move { client.lock_owned().await }),
        )
        .await
    }

    async fn lock_owned(&self) -> Result<GenericStatusResponse, ClientError> {
        if let Some(token) = self.raw_session_token() {
            return self.lock_with_token(&token).await;
        }

        let _transition = match self.acquire_session_transition("Lock").await {
            Ok(transition) => transition,
            Err(error) => {
                self.mark_lock_unconfirmed();
                return Err(ClientError::SessionTransitionLockUnconfirmed(format!(
                    "Lock could not wait for an in-flight session establishment: {error}"
                )));
            }
        };
        let Some(token) = self.raw_session_token() else {
            self.mark_lock_unconfirmed();
            return Err(ClientError::SessionTransitionLockUnconfirmed(
                "Lock could not be authenticated because no session token was available".into(),
            ));
        };
        self.lock_with_token(&token).await
    }

    async fn lock_with_token(
        &self,
        expected_token: &str,
    ) -> Result<GenericStatusResponse, ClientError> {
        for _ in 0..2 {
            if self.raw_lock_confirmed(expected_token).await {
                // Process-global Lock clears even a concurrently issued T2.
                self.confirm_lock_boundary();
                return Ok(GenericStatusResponse {
                    status: "locked".into(),
                });
            }
        }

        self.mark_lock_unconfirmed();
        Err(ClientError::SessionTransitionLockUnconfirmed(
            "two authenticated Lock attempts completed without structural confirmation".into(),
        ))
    }
}
