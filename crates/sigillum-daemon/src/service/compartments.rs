//! Compartment lifecycle management.
//!
//! Handles creation, removal, initialization, and switching of vault
//! compartments with FIDO2 and passphrase-based protection.

use std::path::Path;

use rand::RngCore;
use sigillum_api::{
    CompartmentAddRequest, CompartmentAddedResponse, CompartmentInfo, CompartmentInitRequest,
    CompartmentInitializedResponse, CompartmentListResponse, CompartmentRemoveRequest,
    CompartmentRemovedResponse, CompartmentSwitchRequest, SwitchCompartmentResponse,
};
use sigillum_core::VaultLifecycle;
use sigillum_core::utils::{derive_key_from_passphrase, save_salt, save_wrapped_master_key};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::CompartmentMeta;
use zeroize::Zeroize;
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;
use crate::operations::PendingOperationSpec;

use super::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) fn list_compartments(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<CompartmentListResponse> {
        let token = self.require_session(token)?;
        if !self.state.is_unlocked() {
            return Ok(CompartmentListResponse {
                compartments: Vec::new(),
            });
        }

        let active = self.state.active_compartment_id_for(token);
        let compartments = self
            .state
            .unlocked_compartments()
            .into_iter()
            .map(|meta| CompartmentInfo {
                id: meta.id,
                label: meta.label,
                threshold: meta.threshold,
                passphrase_mode: meta.passphrase_mode,
                is_active: active == Some(meta.id),
            })
            .collect();
        Ok(CompartmentListResponse { compartments })
    }

    pub(crate) async fn add_compartment(
        &self,
        token: Option<&str>,
        body: CompartmentAddRequest,
    ) -> ServiceResult<CompartmentAddedResponse> {
        let _ = self.require_session(token)?;
        if body.label.is_empty() {
            return Err(ServiceError::bad_request("label is required"));
        }
        if body.threshold == 0 {
            return Err(ServiceError::bad_request("threshold must be >= 1"));
        }

        let _guard = self.state.operation_guard().await;
        let unlocked = self.state.unlocked_compartments();
        if unlocked.is_empty() {
            return Err(ServiceError::forbidden("Access denied."));
        }
        if unlocked.iter().any(|meta| meta.threshold == body.threshold) {
            return Err(ServiceError::bad_request("Duplicate threshold."));
        }

        let id = sigillum_fido2::config::next_compartment_id(&unlocked);
        let meta = CompartmentMeta {
            id,
            label: body.label.clone(),
            threshold: body.threshold,
            passphrase_mode: body.passphrase_mode.clone(),
        };
        let journal = self.begin_operation(
            PendingOperationSpec::compartment_add(body.label.clone(), body.threshold),
            Some(format!("compartment/{id}")),
        )?;

        let mut master_key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut master_key);

        self.state.ensure_vault(id);
        self.with_vault(id, |vault| {
            vault
                .initialize(&master_key)
                .map_err(|error| ServiceError::internal(format!("Init failed: {error}")))
        })?;

        Fido2Manager::save_compartment_meta(&self.state.base_dir, &meta, &master_key)
            .map_err(|error| ServiceError::internal(format!("Save meta failed: {error}")))?;

        self.state.unlock_compartment(id, master_key, meta);
        master_key.zeroize();
        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;

        self.record_audit(
            Some(id),
            AuditEventSpec::CompartmentAdd {
                label: body.label.clone(),
                threshold: body.threshold,
            },
        )?;

        Ok(CompartmentAddedResponse {
            status: "added".into(),
            id,
            label: body.label,
            threshold: body.threshold,
        })
    }

    pub(crate) async fn remove_compartment(
        &self,
        token: Option<&str>,
        body: CompartmentRemoveRequest,
    ) -> ServiceResult<CompartmentRemovedResponse> {
        let _ = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        if !self.state.is_unlocked() {
            return Err(ServiceError::forbidden("Access denied."));
        }

        let id = body.id;
        let unlocked = self.state.unlocked_compartments();
        if !unlocked.iter().any(|meta| meta.id == id) {
            return Err(ServiceError::not_found("Compartment not found."));
        }

        let dir = self.state.compartment_dir(id);
        let replacement = dir.with_extension("replacing");
        let backup = dir.with_extension("rollback");
        recover_compartment_replacement(&dir, &replacement, &backup)?;

        let journal = self.begin_operation(
            PendingOperationSpec::compartment_remove(id),
            Some(format!("compartment/{id}")),
        )?;
        prepare_dummy_compartment_dir(&replacement)?;

        if let Err(error) = std::fs::rename(&dir, &backup) {
            let _ = std::fs::remove_dir_all(&replacement);
            return Err(ServiceError::internal(format!(
                "Failed to stage removal: {error}"
            )));
        }

        if let Err(error) = std::fs::rename(&replacement, &dir) {
            let _ = std::fs::rename(&backup, &dir);
            let _ = std::fs::remove_dir_all(&replacement);
            return Err(ServiceError::internal(format!(
                "Failed to finalize compartment replacement: {error}"
            )));
        }

        std::fs::remove_dir_all(&backup).map_err(|error| {
            ServiceError::internal(format!("Failed to purge rollback directory: {error}"))
        })?;

        self.state.remove_compartment(id);
        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(Some(id), AuditEventSpec::CompartmentRemove { id })?;

        Ok(CompartmentRemovedResponse {
            status: "removed".into(),
            id,
        })
    }

    pub(crate) async fn init_compartment(
        &self,
        token: Option<&str>,
        body: CompartmentInitRequest,
    ) -> ServiceResult<CompartmentInitializedResponse> {
        let existing_session = if self.state.is_initialized() {
            Some(self.require_session(token)?.to_string())
        } else {
            None
        };

        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        let _guard = self.state.operation_guard().await;
        let journal = self.begin_operation(
            PendingOperationSpec::compartment_init(body.label.clone(), body.threshold),
            Some(format!("compartment/{}", body.id)),
        )?;
        self.state.ensure_vault(body.id);

        let mut master_key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut master_key);

        let (wrap_key, salt) = derive_key_from_passphrase(passphrase.as_str())?;

        self.with_vault(body.id, |vault| {
            vault
                .initialize(&master_key)
                .map_err(|error| ServiceError::internal(format!("Init failed: {error}")))
        })?;

        save_salt(&salt, &self.state.salt_path(body.id))
            .map_err(|error| ServiceError::internal(format!("Save salt failed: {error}")))?;
        save_wrapped_master_key(
            &master_key,
            &wrap_key,
            &self.state.wrapped_key_path(body.id),
        )
        .map_err(|error| ServiceError::internal(format!("Save wrapped key failed: {error}")))?;

        let meta = CompartmentMeta {
            id: body.id,
            label: body.label.unwrap_or_else(|| "default".into()),
            threshold: body.threshold.unwrap_or(1),
            passphrase_mode: Some("wrapped".into()),
        };
        Fido2Manager::save_compartment_meta(&self.state.base_dir, &meta, &master_key)
            .map_err(|error| ServiceError::internal(format!("Save meta failed: {error}")))?;
        Fido2Manager::setup_dummy_directories(&self.state.base_dir, &[body.id]).map_err(
            |error| ServiceError::internal(format!("Failed to setup directories: {error}")),
        )?;
        std::fs::write(self.state.base_dir.join(".initialized"), b"1")
            .map_err(|error| ServiceError::internal(format!("Failed to write marker: {error}")))?;

        self.state
            .unlock_compartment(body.id, master_key, meta.clone());
        master_key.zeroize();

        let session_token = match existing_session {
            Some(token) => {
                self.state
                    .switch_active_for(&token, body.id)
                    .map_err(ServiceError::forbidden)?;
                token
            }
            None => self.state.create_session(Some(body.id)),
        };

        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(
            Some(body.id),
            AuditEventSpec::CompartmentInit {
                label: meta.label.clone(),
                threshold: meta.threshold,
            },
        )?;

        Ok(CompartmentInitializedResponse {
            status: "initialized".into(),
            compartment_id: body.id,
            compartment_label: meta.label,
            session_token,
        })
    }

    pub(crate) async fn switch_compartment(
        &self,
        token: Option<&str>,
        body: CompartmentSwitchRequest,
    ) -> ServiceResult<SwitchCompartmentResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        self.state
            .switch_active_for(token, body.id)
            .map_err(ServiceError::forbidden)?;

        let label = self
            .state
            .unlocked_compartments()
            .into_iter()
            .find(|meta| meta.id == body.id)
            .map(|meta| meta.label)
            .unwrap_or_default();

        self.record_audit(
            Some(body.id),
            AuditEventSpec::CompartmentSwitch {
                label: label.clone(),
            },
        )?;

        Ok(SwitchCompartmentResponse {
            status: "switched".into(),
            compartment_id: body.id,
            compartment_label: label,
        })
    }
}

fn prepare_dummy_compartment_dir(dir: &Path) -> ServiceResult<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|error| {
            ServiceError::internal(format!("Failed to reset staging dir: {error}"))
        })?;
    }
    std::fs::create_dir_all(dir).map_err(|error| {
        ServiceError::internal(format!("Failed to create staging dir: {error}"))
    })?;

    sigillum_fido2::crypto::generate_dummy_file(&dir.join("meta.enc"), 156, 156).map_err(
        |error| ServiceError::internal(format!("Failed to prepare dummy files: {error}")),
    )?;
    sigillum_fido2::crypto::generate_dummy_file(&dir.join("vault.enc"), 30, 30).map_err(
        |error| ServiceError::internal(format!("Failed to prepare dummy files: {error}")),
    )?;
    sigillum_fido2::crypto::generate_dummy_file(&dir.join("passphrase.salt"), 32, 32).map_err(
        |error| ServiceError::internal(format!("Failed to prepare dummy files: {error}")),
    )?;
    sigillum_fido2::crypto::generate_dummy_file(&dir.join("passphrase_wrapped_key.enc"), 60, 60)
        .map_err(|error| {
            ServiceError::internal(format!("Failed to prepare dummy files: {error}"))
        })?;
    Ok(())
}

fn recover_compartment_replacement(
    live_dir: &Path,
    replacement_dir: &Path,
    rollback_dir: &Path,
) -> ServiceResult<()> {
    if live_dir.exists() {
        if rollback_dir.exists() {
            std::fs::remove_dir_all(rollback_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to remove stale rollback dir: {error}"))
            })?;
        }
        if replacement_dir.exists() {
            std::fs::remove_dir_all(replacement_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to remove stale replacement dir: {error}"))
            })?;
        }
        return Ok(());
    }

    if rollback_dir.exists() {
        if replacement_dir.exists() {
            let _ = std::fs::remove_dir_all(replacement_dir);
        }
        std::fs::rename(rollback_dir, live_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to restore rollback dir: {error}"))
        })?;
        return Ok(());
    }

    if replacement_dir.exists() {
        std::fs::rename(replacement_dir, live_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to recover replacement dir: {error}"))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn replacement_recovery_prefers_live_tree() {
        let dir = TempDir::new().unwrap();
        let live = dir.path().join("0");
        let replacement = dir.path().join("0.replacing");
        let rollback = dir.path().join("0.rollback");

        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::create_dir_all(&rollback).unwrap();

        recover_compartment_replacement(&live, &replacement, &rollback).unwrap();

        assert!(live.exists());
        assert!(!replacement.exists());
        assert!(!rollback.exists());
    }

    #[test]
    fn replacement_recovery_restores_rollback_when_live_is_missing() {
        let dir = TempDir::new().unwrap();
        let live = dir.path().join("0");
        let replacement = dir.path().join("0.replacing");
        let rollback = dir.path().join("0.rollback");

        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::create_dir_all(&rollback).unwrap();

        recover_compartment_replacement(&live, &replacement, &rollback).unwrap();

        assert!(live.exists());
        assert!(!replacement.exists());
        assert!(!rollback.exists());
    }
}
