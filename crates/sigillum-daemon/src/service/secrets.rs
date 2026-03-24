//! Secret and API key CRUD operations with tier-based access.
//!
//! Manages creation, retrieval, deletion, and inter-compartment sharing
//! of secrets and API keys with differentiated tiers and access control.

use secrecy::ExposeSecret;
use sigillum_api::{
    KeyListResponse, KeyMutationResponse, KeyOnlyRequest, KeyValueRequest, KeyValueResponse,
    PushResponse, SecretsPushRequest,
};
use sigillum_core::SecretStore;

use crate::audit_log::AuditEventSpec;

use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn list_api_keys(&self, token: Option<&str>) -> ServiceResult<KeyListResponse> {
        let token = self.require_session(token)?;
        let keys = self.with_active_vault(token, |vault, _| Ok(vault.read_api_keys()?))?;
        Ok(KeyListResponse { keys })
    }

    pub(crate) fn get_api_key(
        &self,
        token: Option<&str>,
        body: KeyOnlyRequest,
    ) -> ServiceResult<KeyValueResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let value = self.with_active_vault(token, |vault, _| {
            vault
                .read_api_key(&key)?
                .map(|value| value.expose_secret().to_string())
                .ok_or_else(|| ServiceError::not_found(format!("API key '{key}' not found")))
        })?;
        Ok(KeyValueResponse { key, value })
    }

    pub(crate) async fn set_api_key(
        &self,
        token: Option<&str>,
        body: KeyValueRequest,
    ) -> ServiceResult<KeyMutationResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let value = body
            .value
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ServiceError::bad_request("value is required"))?;
        let _guard = self.state.operation_guard().await;
        let active_compartment_id = self.state.active_compartment_id_for(token);
        self.with_active_vault(token, |vault, _| Ok(vault.set_api_key(&key, &value)?))?;
        self.record_audit(
            active_compartment_id,
            AuditEventSpec::ApiKeySet { key: key.clone() },
        )?;
        Ok(KeyMutationResponse {
            status: "ok".into(),
            key,
            tier: Some(1),
        })
    }

    pub(crate) async fn delete_api_key(
        &self,
        token: Option<&str>,
        body: KeyOnlyRequest,
    ) -> ServiceResult<KeyMutationResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let _guard = self.state.operation_guard().await;
        let active_compartment_id = self.state.active_compartment_id_for(token);
        self.with_active_vault(token, |vault, _| Ok(vault.delete_api_key(&key)?))?;
        self.record_audit(
            active_compartment_id,
            AuditEventSpec::ApiKeyDelete { key: key.clone() },
        )?;
        Ok(KeyMutationResponse {
            status: "deleted".into(),
            key,
            tier: None,
        })
    }

    pub(crate) fn list_secrets(&self, token: Option<&str>) -> ServiceResult<KeyListResponse> {
        let token = self.require_session(token)?;
        let keys = self.with_active_vault(token, |vault, _| {
            if !vault.is_unlocked() {
                return Err(ServiceError::forbidden("Vault is locked."));
            }
            Ok(vault.read_secrets()?)
        })?;
        Ok(KeyListResponse { keys })
    }

    pub(crate) fn get_secret(
        &self,
        token: Option<&str>,
        body: KeyOnlyRequest,
    ) -> ServiceResult<KeyValueResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let value = self.with_active_vault(token, |vault, _| {
            if !vault.is_unlocked() {
                return Err(ServiceError::forbidden("Vault is locked."));
            }
            vault
                .read_secret(&key)?
                .map(|value| value.expose_secret().to_string())
                .ok_or_else(|| ServiceError::not_found(format!("Secret '{key}' not found")))
        })?;
        Ok(KeyValueResponse { key, value })
    }

    pub(crate) async fn set_secret(
        &self,
        token: Option<&str>,
        body: KeyValueRequest,
    ) -> ServiceResult<KeyMutationResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let value = body
            .value
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ServiceError::bad_request("value is required"))?;
        let _guard = self.state.operation_guard().await;
        let active_compartment_id = self.state.active_compartment_id_for(token);
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
        Ok(KeyMutationResponse {
            status: "ok".into(),
            key,
            tier: Some(2),
        })
    }

    pub(crate) async fn delete_secret(
        &self,
        token: Option<&str>,
        body: KeyOnlyRequest,
    ) -> ServiceResult<KeyMutationResponse> {
        let token = self.require_session(token)?;
        let key = body.key;
        let _guard = self.state.operation_guard().await;
        let active_compartment_id = self.state.active_compartment_id_for(token);
        self.with_active_vault(token, |vault, _| {
            if !vault.is_unlocked() {
                return Err(ServiceError::forbidden("Vault is locked."));
            }
            Ok(vault.delete_secret(&key)?)
        })?;
        self.record_audit(
            active_compartment_id,
            AuditEventSpec::SecretDelete { key: key.clone() },
        )?;
        Ok(KeyMutationResponse {
            status: "deleted".into(),
            key,
            tier: None,
        })
    }

    pub(crate) async fn push_secret(
        &self,
        token: Option<&str>,
        body: SecretsPushRequest,
    ) -> ServiceResult<PushResponse> {
        let _ = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let unlocked = self.state.unlocked_compartments();
        if unlocked.len() < 2 {
            return Err(ServiceError::forbidden("Access denied."));
        }
        if !unlocked.iter().any(|meta| meta.id == body.from_compartment) {
            return Err(ServiceError::not_found("Source compartment not unlocked."));
        }
        if !unlocked.iter().any(|meta| meta.id == body.to_compartment) {
            return Err(ServiceError::not_found("Target compartment not unlocked."));
        }

        let tier = body.tier.unwrap_or(2);
        if tier != 1 && tier != 2 {
            return Err(ServiceError::bad_request(
                "tier must be 1 (api-key) or 2 (secret).",
            ));
        }

        let key = body.key;
        let target_key = body.new_key.unwrap_or_else(|| key.clone());
        let value = if tier == 1 {
            self.with_vault(body.from_compartment, |vault| {
                vault
                    .read_api_key(&key)?
                    .map(|value| value.expose_secret().to_string())
                    .ok_or_else(|| {
                        ServiceError::not_found(format!("Key '{key}' not found in source."))
                    })
            })?
        } else {
            self.with_vault(body.from_compartment, |vault| {
                if !vault.is_unlocked() {
                    return Err(ServiceError::forbidden("Vault is locked."));
                }
                vault
                    .read_secret(&key)?
                    .map(|value| value.expose_secret().to_string())
                    .ok_or_else(|| {
                        ServiceError::not_found(format!("Key '{key}' not found in source."))
                    })
            })?
        };

        self.with_vault(body.to_compartment, |vault| {
            if tier == 1 {
                Ok(vault.set_api_key(&target_key, &value)?)
            } else {
                if !vault.is_unlocked() {
                    return Err(ServiceError::forbidden("Vault is locked."));
                }
                Ok(vault.set_secret(&target_key, &value)?)
            }
        })?;

        self.record_audit(
            Some(body.to_compartment),
            AuditEventSpec::SecretPush {
                from_compartment: body.from_compartment,
                to_compartment: body.to_compartment,
                key: key.clone(),
                new_key: target_key.clone(),
                tier,
            },
        )?;

        Ok(PushResponse {
            status: "pushed".into(),
            from: body.from_compartment,
            to: body.to_compartment,
            key: target_key,
        })
    }
}
