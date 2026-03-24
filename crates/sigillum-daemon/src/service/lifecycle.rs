//! Vault lifecycle: passphrase unlock, compartment locking, and session revocation.
//!
//! Manages passphrase-based unlock of compartments with salt derivation,
//! master key verification, and session-based access control.

use sigillum_api::request::PassphraseRequest;
use sigillum_api::response::{
    LockResponse, SessionRevokeResponse, UnlockResponse, UnlockedCompartment,
};
use sigillum_core::VaultLifecycle;
use sigillum_core::utils::{derive_key_with_salt, load_wrapped_master_key};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::SHARD_SLOTS;
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;

use super::{ServiceError, ServiceResult, SigillumService};

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

        let _guard = self.state.operation_guard().await;
        let mut unlocked_metas = Vec::new();

        for (id, master_key, meta) in
            scan_passphrase_matches(self.state.base_dir.clone(), passphrase.as_str()).await?
        {
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

                let wrap_key = derive_key_with_salt(passphrase.as_str(), &salt);
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
