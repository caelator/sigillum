//! Vault lifecycle: passphrase unlock, compartment locking, and session revocation.
//!
//! Manages passphrase-based unlock of compartments with salt derivation,
//! master key verification, and session-based access control.

use std::time::Duration;

use sigillum_api::request::{CapabilitySessionRequest, PassphraseRequest};
use sigillum_api::response::{
    CapabilitySessionResponse, LockResponse, SessionRevokeResponse, UnlockResponse,
    UnlockedCompartment,
};
use sigillum_core::VaultLifecycle;
use sigillum_core::utils::{derive_key_with_salt, load_wrapped_master_key};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::SHARD_SLOTS;
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;

use super::{
    DEFAULT_CAPABILITY_SESSION_TTL_SECS, ServiceError, ServiceResult, SigillumService,
    capability_scopes,
};

impl SigillumService {
    pub(crate) async fn unlock_with_passphrase(
        &self,
        token: Option<&str>,
        body: PassphraseRequest,
    ) -> ServiceResult<UnlockResponse> {
        if self.state.is_unlocked() && self.optional_session(token).is_some() {
            return Err(ServiceError::conflict("Vault is already unlocked."));
        }

        // Rate-limit: reject early if the caller is in a cooldown window.
        if let Err(retry_after) = self.state.check_unlock_throttle() {
            return Err(ServiceError::too_many_requests(format!(
                "Too many failed unlock attempts. Retry in {retry_after}s."
            )));
        }

        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        if let Some(response) = self
            .reauthenticate_unlocked_passphrase(passphrase.as_str())
            .await?
        {
            return Ok(response);
        }

        let mut unlocked_metas = Vec::new();

        let passphrase_matches =
            scan_passphrase_matches(self.state.base_dir.clone(), passphrase.as_str()).await?;
        let _guard = self.state.operation_guard().await;

        for (id, master_key, meta) in passphrase_matches {
            self.state.ensure_vault(id);
            let verified = self.state.with_vault(id, |vault| {
                vault.load_master_key(master_key);
                let verified = vault.verify_master_key();
                if !verified {
                    vault.zeroize_master_key();
                }
                verified
            });
            if verified == Some(true) {
                unlocked_metas.push(meta.clone());
                self.state.unlock_compartment(id, master_key, meta);
            }
        }

        if unlocked_metas.is_empty() {
            self.state.record_unlock_failure();
            return Err(ServiceError::unauthorized(
                "No compartment matched this passphrase.",
            ));
        }

        self.passphrase_unlock_response(unlocked_metas)
    }

    async fn reauthenticate_unlocked_passphrase(
        &self,
        passphrase: &str,
    ) -> ServiceResult<Option<UnlockResponse>> {
        if !self.state.is_unlocked() {
            return Ok(None);
        }

        let master_keys = self.state.extract_all_master_keys_with_meta();
        if master_keys.is_empty() {
            return Ok(None);
        }

        let base_dir = self.state.base_dir.clone();
        let passphrase = passphrase.to_owned();
        let matched_metas = tokio::task::spawn_blocking(move || {
            let passphrase = Zeroizing::new(passphrase);
            let mut matched_metas = Vec::new();
            for (meta, loaded_master_key) in master_keys {
                let compartment_dir = base_dir.join("compartments").join(meta.id.to_string());
                let salt = match std::fs::read(compartment_dir.join("passphrase.salt")) {
                    Ok(salt) if salt.len() == 32 => salt,
                    _ => continue,
                };
                let Ok(wrap_key) = derive_key_with_salt(passphrase.as_str(), &salt) else {
                    continue;
                };
                let Some(unwrapped_master_key) = load_wrapped_master_key(
                    &wrap_key,
                    &compartment_dir.join("passphrase_wrapped_key.enc"),
                ) else {
                    continue;
                };
                if unwrapped_master_key.as_ref() == loaded_master_key.as_ref() {
                    matched_metas.push(meta);
                }
            }
            matched_metas
        })
        .await
        .map_err(|error| {
            ServiceError::internal(format!("Passphrase reauth worker failed: {error}"))
        })?;

        if matched_metas.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.passphrase_unlock_response(matched_metas)?))
    }

    fn passphrase_unlock_response(
        &self,
        unlocked_metas: Vec<sigillum_fido2::config::CompartmentMeta>,
    ) -> ServiceResult<UnlockResponse> {
        self.state.reset_unlock_throttle();
        let token = self.state.create_session(None);
        let ids: Vec<usize> = unlocked_metas.iter().map(|meta| meta.id).collect();
        self.record_audit(
            self.state.default_active_compartment_id(),
            AuditEventSpec::UnlockPassphrase {
                compartment_ids: ids,
                count: unlocked_metas.len(),
            },
        )?;

        Ok(UnlockResponse {
            status: "unlocked".into(),
            method: "passphrase".into(),
            cascading: None,
            session_token: token,
            unlocked_compartments: unlocked_metas
                .into_iter()
                .map(|meta| UnlockedCompartment {
                    id: meta.id,
                    label: meta.label,
                    threshold: meta.threshold,
                    passphrase_mode: meta.passphrase_mode,
                })
                .collect(),
            active_compartment_id: self.state.default_active_compartment_id(),
        })
    }

    pub(crate) async fn lock_all(&self, token: Option<&str>) -> ServiceResult<LockResponse> {
        let _ = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        self.state.lock_all();
        self.record_audit(None, AuditEventSpec::LockAll)?;
        Ok(LockResponse {
            status: "locked".into(),
            message: "All compartments locked. Master keys zeroized.".into(),
        })
    }

    pub(crate) async fn revoke_session(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<SessionRevokeResponse> {
        let token = self.require_session(token)?.to_string();
        let _guard = self.state.operation_guard().await;
        let compartment_id = self.state.active_compartment_id_for(&token);
        self.state.revoke_session(&token);
        self.record_audit(compartment_id, AuditEventSpec::SessionRevoke)?;
        Ok(SessionRevokeResponse {
            status: "revoked".into(),
            requires_reauth: true,
        })
    }

    pub(crate) fn mint_capability_session(
        &self,
        token: Option<&str>,
        body: CapabilitySessionRequest,
    ) -> ServiceResult<CapabilitySessionResponse> {
        let token = self.require_full_session(token)?;
        let unknown = body
            .scopes
            .iter()
            .find(|scope| !capability_scopes::is_known(scope));
        if let Some(scope) = unknown {
            return Err(ServiceError::bad_request(format!(
                "Unknown daemon capability scope: {scope}"
            )));
        }
        let ttl = Duration::from_secs(body.ttl_secs.unwrap_or(DEFAULT_CAPABILITY_SESSION_TTL_SECS));
        let active = self.state.active_compartment_id_for(token);
        let (session_token, expires_at_unix) =
            self.state
                .create_capability_session(active, body.scopes.clone(), ttl);
        Ok(CapabilitySessionResponse {
            status: "minted".into(),
            session_token,
            scopes: body.scopes,
            expires_at_unix,
        })
    }
}

async fn scan_passphrase_matches(
    base_dir: std::path::PathBuf,
    passphrase: &str,
) -> ServiceResult<Vec<(usize, [u8; 32], sigillum_fido2::config::CompartmentMeta)>> {
    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8);
    let worker_count = worker_count.min(SHARD_SLOTS.max(1));

    let mut tasks = Vec::with_capacity(worker_count);
    for worker_index in 0..worker_count {
        let base_dir = base_dir.clone();
        let passphrase = passphrase.to_owned();
        tasks.push(tokio::task::spawn_blocking(move || {
            let passphrase = Zeroizing::new(passphrase);
            let mut matches = Vec::new();

            for id in (worker_index..SHARD_SLOTS).step_by(worker_count) {
                let salt_path = base_dir
                    .join("compartments")
                    .join(id.to_string())
                    .join("passphrase.salt");
                let wrapped_path = base_dir
                    .join("compartments")
                    .join(id.to_string())
                    .join("passphrase_wrapped_key.enc");

                let salt = match std::fs::read(&salt_path) {
                    Ok(salt) if salt.len() == 32 => salt,
                    _ => continue,
                };

                let Ok(wrap_key) = derive_key_with_salt(passphrase.as_str(), &salt) else {
                    continue;
                };
                let Some(master_key) = load_wrapped_master_key(&wrap_key, &wrapped_path) else {
                    continue;
                };

                let Ok(meta) = Fido2Manager::load_compartment_meta(&base_dir, id, &master_key)
                else {
                    continue;
                };

                matches.push((id, *master_key, meta));
            }

            matches
        }));
    }

    let mut matches = Vec::new();
    for task in tasks {
        let worker_matches = task.await.map_err(|error| {
            ServiceError::internal(format!("Passphrase unlock worker failed: {error}"))
        })?;
        matches.extend(worker_matches);
    }
    matches.sort_by_key(|(_, _, meta)| meta.threshold);
    Ok(matches)
}
