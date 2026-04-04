//! Secret generation and atomic persistence.

use sigillum_api::request::{GenerateStoreKind, GenerateStoreRequest};
use sigillum_api::response::GenerateStoreResponse;
use sigillum_core::SecretStore;
use sigillum_generator::{generate_passphrase, generate_password};

use crate::audit_log::AuditEventSpec;

use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) async fn generate_and_store(
        &self,
        token: Option<&str>,
        body: GenerateStoreRequest,
    ) -> ServiceResult<GenerateStoreResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let active_compartment_id = self.state.active_compartment_id_for(token);
        let key = body.key;

        let (value, kind) = match body.kind {
            GenerateStoreKind::Password { length, charset } => (
                generate_password(charset.as_str(), length)
                    .map_err(|error| ServiceError::bad_request(error.to_string()))?,
                "password",
            ),
            GenerateStoreKind::Passphrase {
                word_count,
                separator,
            } => (
                generate_passphrase(word_count, &separator)
                    .map_err(|error| ServiceError::bad_request(error.to_string()))?,
                "passphrase",
            ),
        };

        self.with_active_vault(token, |vault, _| {
            if !vault.is_unlocked() {
                return Err(ServiceError::forbidden("Vault is locked."));
            }
            Ok(vault.set_secret(&key, &value)?)
        })?;
        self.record_audit(
            active_compartment_id,
            AuditEventSpec::SecretSet { key: key.clone() },
        )?;

        Ok(GenerateStoreResponse {
            status: "stored".into(),
            key,
            value,
            kind: kind.into(),
        })
    }
}
