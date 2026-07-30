//! FIDO2 hardware key operations for vault initialization and unlock.
//!
//! Provides FIDO2 device detection, key registration, removal, and
//! cascading unlock across multiple compartments.

use sigillum_api::{
    Fido2DetectResponse, Fido2KeyInfo, Fido2ListResponse, Fido2RegisterRequest,
    Fido2RegisterResponse, Fido2RemoveRequest, Fido2RemoveResponse, Fido2SetPinRequest,
    Fido2SetPinResponse, Fido2SetupRequest, Fido2SetupResponse, Fido2StatusResponse,
    Fido2UnlockRequest, UnlockResponse, UnlockedCompartment,
};
use sigillum_core::VaultLifecycle;
use sigillum_core::utils::{derive_key_from_passphrase, save_salt, save_wrapped_master_key};
use sigillum_fido2::config::CompartmentMeta;
use sigillum_fido2::error::Fido2Error;
use sigillum_fido2::{Fido2Manager, Fido2MutationContext};

use crate::audit_log::AuditEventSpec;
use crate::operations::PendingOperationSpec;

use super::{ServiceError, ServiceResult, SigillumService};

fn map_other_fido2_message(action: &str, message: &str) -> ServiceError {
    let normalized = message.to_ascii_uppercase();
    if normalized.contains("ALREADY SET") {
        return ServiceError::conflict(format!(
            "{action} failed: this hardware key already has a FIDO2 PIN configured. Use the existing PIN instead."
        ));
    }
    if normalized.contains("PIN_REQUIRED") || normalized.contains("0X36") {
        return ServiceError::bad_request(format!(
            "{action} failed: this hardware key is configured to require its current FIDO2 PIN for this step. Enter the existing PIN and retry, or use a touch-only key."
        ));
    }
    if normalized.contains("PIN_POLICY") || normalized.contains("0X37") {
        return ServiceError::bad_request(format!(
            "{action} failed: the new FIDO2 PIN does not meet this hardware key's PIN policy. Use at least 4 characters and avoid unsupported patterns."
        ));
    }
    if normalized.contains("PIN_NOT_SET") || normalized.contains("0X35") {
        return ServiceError::bad_request(format!(
            "{action} failed: this hardware key does not have a FIDO2 PIN configured yet. Set a PIN on the key first, then retry."
        ));
    }
    if normalized.contains("PIN_AUTH_BLOCKED") || normalized.contains("0X34") {
        return ServiceError::bad_request(format!(
            "{action} failed: the hardware key temporarily blocked PIN authentication after too many attempts. Unplug and reinsert the key to power-cycle it, then retry with the correct PIN."
        ));
    }
    if normalized.contains("PIN_BLOCKED") || normalized.contains("0X32") {
        return ServiceError::bad_request(format!(
            "{action} failed: the hardware key PIN is blocked. Reset or recover the key outside Sigillum before retrying."
        ));
    }
    if normalized.contains("PIN_INVALID") || normalized.contains("0X31") {
        return ServiceError::bad_request(format!(
            "{action} failed: incorrect FIDO2 PIN. Verify the PIN and retry."
        ));
    }
    ServiceError::internal(format!("{action} failed: {message}"))
}

fn map_fido2_service_error(action: &str, error: Fido2Error) -> ServiceError {
    match error {
        Fido2Error::NoDevice => ServiceError::bad_request(format!(
            "{action} failed: no FIDO2 device is detected. Insert the hardware key and retry."
        )),
        Fido2Error::MultipleDevicesDetected { count } => ServiceError::bad_request(format!(
            "{action} failed: {count} FIDO2 hardware key(s) are attached and Sigillum cannot tell which one to use for this step. Leave only the target key inserted, then retry."
        )),
        Fido2Error::Timeout { .. } => ServiceError::bad_request(format!(
            "{action} timed out. Keep the hardware key inserted, touch it when prompted, and retry."
        )),
        Fido2Error::AttestationFailed => {
            ServiceError::internal(format!("{action} failed: attestation verification failed."))
        }
        Fido2Error::NoHmacSecret => ServiceError::bad_request(format!(
            "{action} failed: this hardware key did not return the hmac-secret extension required by Sigillum."
        )),
        Fido2Error::NoMatchingCredential => ServiceError::bad_request(format!(
            "{action} failed: none of the attached hardware keys matched the Sigillum credential needed for this step."
        )),
        Fido2Error::NoNewDeviceDetected => ServiceError::bad_request(format!(
            "{action} failed: every attached hardware key already appears to be registered. Insert the new key you want to add, then retry."
        )),
        Fido2Error::IncorrectPin => ServiceError::bad_request(format!(
            "{action} failed: incorrect FIDO2 PIN. Verify the PIN and retry."
        )),
        Fido2Error::PinRequired => ServiceError::bad_request(format!(
            "{action} failed: this hardware key is configured to require its current FIDO2 PIN for this step. Enter the existing PIN and retry, or use a touch-only key."
        )),
        Fido2Error::PinAlreadySet => ServiceError::conflict(format!(
            "{action} failed: this hardware key already has a FIDO2 PIN configured. Use the existing PIN instead."
        )),
        Fido2Error::PinNotSet => ServiceError::bad_request(format!(
            "{action} failed: this hardware key does not have a FIDO2 PIN configured yet. Set a PIN on the key first, then retry."
        )),
        Fido2Error::PinBlocked => ServiceError::bad_request(format!(
            "{action} failed: the hardware key PIN is blocked. Reset or recover the key outside Sigillum before retrying."
        )),
        Fido2Error::PinAuthBlocked => ServiceError::bad_request(format!(
            "{action} failed: the hardware key temporarily blocked PIN authentication after too many attempts. Unplug and reinsert the key to power-cycle it, then retry with the correct PIN."
        )),
        Fido2Error::DuplicateKey { label } => ServiceError::conflict(format!(
            "{action} failed: a key with this label or credential is already registered ({label})."
        )),
        Fido2Error::KeyNotFound { label } => {
            ServiceError::not_found(format!("{action} failed: key not found ({label})."))
        }
        Fido2Error::NoKeysRegistered => {
            ServiceError::bad_request(format!("{action} failed: no hardware keys are registered."))
        }
        Fido2Error::QuorumNotMet {
            required,
            available,
        } => ServiceError::bad_request(format!(
            "{action} failed: not enough matching hardware keys were provided. Need {required}, matched {available}."
        )),
        Fido2Error::RemovalBelowQuorum {
            remaining,
            threshold,
        } => ServiceError::conflict(format!(
            "{action} failed: removing this key would leave {remaining} active keys below threshold {threshold}."
        )),
        Fido2Error::Config(error) => ServiceError::bad_request(format!("{action} failed: {error}")),
        Fido2Error::WriterBusy { .. } => ServiceError::conflict(format!(
            "{action} could not start because another FIDO2 configuration change is in progress. Wait for it to finish, then retry."
        )),
        Fido2Error::ShamirFailed(error)
        | Fido2Error::ShardEncryption(error)
        | Fido2Error::ShardDecryption(error) => {
            ServiceError::internal(format!("{action} failed: {error}"))
        }
        Fido2Error::Ctap1Device => ServiceError::bad_request(format!(
            "{action} failed: this device only supports CTAP1. Use a CTAP2 hardware key with hmac-secret support."
        )),
        Fido2Error::DuplicateThreshold { threshold } => ServiceError::bad_request(format!(
            "{action} failed: duplicate threshold {threshold} is already assigned to another compartment."
        )),
        Fido2Error::NoCompartmentForThreshold { threshold } => ServiceError::bad_request(format!(
            "{action} failed: no compartment is configured for threshold {threshold}."
        )),
        Fido2Error::CompartmentNotFound { id } => {
            ServiceError::not_found(format!("{action} failed: compartment {id} was not found."))
        }
        Fido2Error::Other(message) => map_other_fido2_message(action, &message),
    }
}

fn optional_pin(pin: Option<&String>) -> Option<&str> {
    pin.and_then(|pin| {
        let pin = pin.as_str();
        (!pin.is_empty()).then_some(pin)
    })
}

impl SigillumService {
    pub(crate) fn fido2_status(&self, token: Option<&str>) -> ServiceResult<Fido2StatusResponse> {
        let _ = self.require_session(token)?;
        let status = self.state.fido2.status().map_err(|error| {
            ServiceError::internal(format!("Failed to load FIDO2 status: {error}"))
        })?;
        Ok(Fido2StatusResponse {
            enabled: status.enabled,
            key_count: status.key_count,
        })
    }

    pub(crate) fn fido2_detect(&self) -> Fido2DetectResponse {
        let device_count = sigillum_fido2::hid::detect_devices();
        Fido2DetectResponse {
            device_present: device_count > 0,
            device_count,
        }
    }

    pub(crate) fn fido2_set_pin(
        &self,
        token: Option<&str>,
        body: Fido2SetPinRequest,
    ) -> ServiceResult<Fido2SetPinResponse> {
        if self.state.is_initialized() {
            let _ = self.require_session(token)?;
        }
        if body.new_pin.len() < 4 {
            return Err(ServiceError::bad_request(
                "FIDO2 PIN must be at least 4 characters long.",
            ));
        }

        self.state
            .fido2
            .set_new_pin(&body.new_pin)
            .map_err(|error| map_fido2_service_error("FIDO2 PIN setup", error))?;

        Ok(Fido2SetPinResponse {
            status: "pin_set".into(),
        })
    }

    pub(crate) fn fido2_list_keys(&self, token: Option<&str>) -> ServiceResult<Fido2ListResponse> {
        let _ = self.require_session(token)?;
        let keys = self.state.fido2.list_keys().map_err(|error| {
            ServiceError::internal(format!("Failed to load FIDO2 keys: {error}"))
        })?;
        Ok(Fido2ListResponse {
            keys: keys
                .into_iter()
                .map(|key| Fido2KeyInfo {
                    label: key.label,
                    credential_id_short: key.credential_id_short,
                    registered_at: key.registered_at,
                })
                .collect(),
        })
    }

    pub(crate) async fn fido2_setup(
        &self,
        body: Fido2SetupRequest,
    ) -> ServiceResult<Fido2SetupResponse> {
        if self.state.is_initialized() {
            return Err(ServiceError::conflict(
                "Already initialized. Use /api/fido2/register to add keys.",
            ));
        }
        if body.label.is_empty() {
            return Err(ServiceError::bad_request("label is required"));
        }
        if body.compartments.is_empty() {
            return Err(ServiceError::bad_request(
                "at least one compartment required",
            ));
        }

        let metas: Vec<CompartmentMeta> = body
            .compartments
            .iter()
            .enumerate()
            .map(|(id, compartment)| CompartmentMeta {
                id,
                label: compartment.label.clone(),
                threshold: compartment.threshold,
                passphrase_mode: compartment.passphrase_mode.clone(),
            })
            .collect();

        if sigillum_fido2::config::validate_thresholds(&metas).is_err() {
            return Err(ServiceError::bad_request("Invalid compartment thresholds."));
        }

        let _guard = self.state.operation_guard().await;
        if self.state.is_initialized() {
            return Err(ServiceError::conflict(
                "Already initialized. Use /api/fido2/register to add keys.",
            ));
        }
        let journal = self.begin_operation(
            PendingOperationSpec::fido2_setup(body.label.clone(), body.compartments.len()),
            Some(body.label.clone()),
        )?;
        let meta_refs: Vec<(CompartmentMeta, &[u8; 32])> = metas
            .iter()
            .map(|meta| (meta.clone(), &[0u8; 32] as &[u8; 32]))
            .collect();

        let result = self
            .state
            .fido2
            .register_key_for_operation(
                optional_pin(body.pin.as_ref()),
                &body.label,
                &meta_refs,
                &[],
                Fido2MutationContext {
                    operation_id: journal.operation_id(),
                    kind: "fido2.setup",
                    subject: Some(&body.label),
                },
            )
            .map_err(|error| map_fido2_service_error("FIDO2 setup", error))?;

        let real_ids: Vec<usize> = result
            .compartment_keys
            .iter()
            .map(|(compartment_id, _)| *compartment_id)
            .collect();

        for (compartment_id, master_key) in &result.compartment_keys {
            self.state.ensure_vault(*compartment_id);
            self.with_vault(*compartment_id, |vault| {
                vault.initialize(master_key).map_err(|error| {
                    ServiceError::internal(format!(
                        "Failed to init compartment {compartment_id}: {error}"
                    ))
                })
            })?;

            let meta = metas
                .iter()
                .find(|meta| meta.id == *compartment_id)
                .ok_or_else(|| {
                    ServiceError::internal(format!("No meta for compartment {compartment_id}"))
                })?;
            Fido2Manager::save_compartment_meta(&self.state.base_dir, meta, master_key).map_err(
                |error| {
                    ServiceError::internal(format!(
                        "Failed to save meta for {compartment_id}: {error}"
                    ))
                },
            )?;
        }

        if let Some(passphrase) = body.passphrase.as_ref() {
            if passphrase.len() >= 8 {
                for (compartment_id, master_key) in &result.compartment_keys {
                    let (wrap_key, salt) = derive_key_from_passphrase(passphrase)?;
                    save_salt(&salt, &self.state.salt_path(*compartment_id)).map_err(|error| {
                        ServiceError::internal(format!("Save salt for {compartment_id}: {error}"))
                    })?;
                    save_wrapped_master_key(
                        master_key,
                        &wrap_key,
                        &self.state.wrapped_key_path(*compartment_id),
                    )
                    .map_err(|error| {
                        ServiceError::internal(format!(
                            "Save wrapped key for {compartment_id}: {error}"
                        ))
                    })?;
                    if let Some(meta) = metas.iter().find(|meta| meta.id == *compartment_id) {
                        let mut updated = meta.clone();
                        updated.passphrase_mode = Some("wrapped".into());
                        Fido2Manager::save_compartment_meta(
                            &self.state.base_dir,
                            &updated,
                            master_key,
                        )
                        .map_err(|error| {
                            ServiceError::internal(format!(
                                "Failed to update meta for {compartment_id}: {error}"
                            ))
                        })?;
                    }
                }
            }
        }

        Fido2Manager::setup_dummy_directories(&self.state.base_dir, &real_ids).map_err(
            |error| ServiceError::internal(format!("Failed to setup directories: {error}")),
        )?;

        let unlock_data: Vec<(CompartmentMeta, [u8; 32])> = result
            .compartment_keys
            .iter()
            .filter_map(|(compartment_id, master_key)| {
                metas
                    .iter()
                    .find(|meta| meta.id == *compartment_id)
                    .map(|meta| (meta.clone(), **master_key))
            })
            .collect();
        self.state.unlock_multiple(&unlock_data);

        let session_token = self.state.create_session(None);
        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(
            self.state.default_active_compartment_id(),
            AuditEventSpec::Fido2Setup {
                label: body.label.clone(),
                compartment_count: body.compartments.len(),
                total_keys: result.total_keys,
            },
        )?;

        Ok(Fido2SetupResponse {
            status: "setup_complete".into(),
            is_first_key: result.is_first_key,
            total_keys: result.total_keys,
            compartments: body.compartments.len(),
            unlocked: true,
            session_token,
        })
    }

    pub(crate) async fn fido2_register(
        &self,
        token: Option<&str>,
        body: Fido2RegisterRequest,
    ) -> ServiceResult<Fido2RegisterResponse> {
        let token = self.require_session(token)?;
        if !self.state.is_initialized() {
            return Err(ServiceError::not_found(
                "Not initialized. Use /api/fido2/setup first.",
            ));
        }

        let skip = body.skip_keys.clone().unwrap_or_default();
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let journal = self.begin_operation(
            PendingOperationSpec::fido2_register(body.poison),
            Some(body.label.clone()),
        )?;

        if body.poison == Some(true) {
            let unlocked = self.state.unlocked_compartments();
            if unlocked.is_empty() {
                return Err(ServiceError::forbidden(
                    "Compartments must be unlocked to read metadata.",
                ));
            }

            let total_keys = self
                .state
                .fido2
                .register_key_poison_for_operation(
                    optional_pin(body.pin.as_ref()),
                    &body.label,
                    &unlocked,
                    Fido2MutationContext {
                        operation_id: journal.operation_id(),
                        kind: "fido2.register",
                        subject: Some(&body.label),
                    },
                )
                .map_err(|error| map_fido2_service_error("FIDO2 registration", error))?;
            journal.complete().map_err(|error| {
                ServiceError::internal(format!("Failed to finalize operation: {error}"))
            })?;
            self.record_audit(
                self.state.default_active_compartment_id(),
                AuditEventSpec::Fido2RegisterPoison {
                    label: body.label.clone(),
                    total_keys,
                },
            )?;

            return Ok(Fido2RegisterResponse {
                status: "registered".into(),
                label: body.label,
                total_keys,
                poison: Some(true),
            });
        }

        let master_keys_with_meta = self.state.extract_all_master_keys_with_meta();
        if master_keys_with_meta.is_empty() {
            return Err(ServiceError::forbidden("Compartments must be unlocked."));
        }

        let master_key_refs: Vec<(CompartmentMeta, &[u8; 32])> = master_keys_with_meta
            .iter()
            .map(|(meta, master_key)| (meta.clone(), &**master_key))
            .collect();

        let result = self
            .state
            .fido2
            .register_key_for_operation(
                optional_pin(body.pin.as_ref()),
                &body.label,
                &master_key_refs,
                &skip,
                Fido2MutationContext {
                    operation_id: journal.operation_id(),
                    kind: "fido2.register",
                    subject: Some(&body.label),
                },
            )
            .map_err(|error| map_fido2_service_error("FIDO2 registration", error))?;
        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(
            self.state.default_active_compartment_id(),
            AuditEventSpec::Fido2Register {
                label: body.label.clone(),
                total_keys: result.total_keys,
            },
        )?;

        Ok(Fido2RegisterResponse {
            status: "registered".into(),
            label: body.label,
            total_keys: result.total_keys,
            poison: None,
        })
    }

    pub(crate) async fn fido2_unlock(
        &self,
        token: Option<&str>,
        body: Fido2UnlockRequest,
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

        if body.tap_count == 0 {
            return Err(ServiceError::bad_request("tap_count must be >= 1."));
        }
        let pins: Vec<String> = body
            .pins
            .into_iter()
            .filter(|pin| !pin.is_empty())
            .collect();

        let _guard = self.state.operation_guard().await;
        let compartment_results = self
            .state
            .fido2
            .authenticate_cascading(&pins, body.tap_count, &self.state.base_dir, None)
            .map_err(|error| {
                self.state.record_unlock_failure();
                map_fido2_service_error("FIDO2 unlock", error)
            })?;

        if compartment_results.is_empty() {
            self.state.record_unlock_failure();
            return Err(ServiceError::unauthorized("No compartments matched."));
        }

        let mut verified = Vec::new();
        for (meta, master_key) in &compartment_results {
            self.state.ensure_vault(meta.id);
            let matched = self.state.with_vault(meta.id, |vault| {
                vault.load_master_key(**master_key);
                let verified = vault.verify_master_key();
                if !verified {
                    vault.zeroize_master_key();
                }
                verified
            });
            if matched == Some(true) {
                verified.push((meta.clone(), **master_key));
            }
        }

        if verified.is_empty() {
            return Err(ServiceError::unauthorized(
                "FIDO2 keys do not match any vault.",
            ));
        }

        self.state.reset_unlock_throttle();
        self.state.unlock_multiple(&verified);
        let session_token = self.state.create_session(None);
        let ids: Vec<usize> = verified.iter().map(|(meta, _)| meta.id).collect();
        self.record_audit(
            self.state.default_active_compartment_id(),
            AuditEventSpec::UnlockFido2 {
                compartment_ids: ids,
                count: verified.len(),
                tap_count: body.tap_count,
            },
        )?;

        Ok(UnlockResponse {
            status: "unlocked".into(),
            method: "fido2".into(),
            cascading: Some(true),
            session_token,
            unlocked_compartments: verified
                .into_iter()
                .map(|(meta, _)| UnlockedCompartment {
                    id: meta.id,
                    label: meta.label,
                    threshold: meta.threshold,
                    passphrase_mode: meta.passphrase_mode,
                })
                .collect(),
            active_compartment_id: self.state.default_active_compartment_id(),
        })
    }

    pub(crate) async fn fido2_remove(
        &self,
        token: Option<&str>,
        body: Fido2RemoveRequest,
    ) -> ServiceResult<Fido2RemoveResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let journal = self.begin_operation(
            PendingOperationSpec::fido2_remove(body.skip_keys.clone().unwrap_or_default()),
            Some(body.label.clone()),
        )?;
        let master_keys_with_meta = self.state.extract_all_master_keys_with_meta();
        if master_keys_with_meta.is_empty() {
            return Err(ServiceError::forbidden("Compartments must be unlocked."));
        }

        let master_key_refs: Vec<(CompartmentMeta, &[u8; 32])> = master_keys_with_meta
            .iter()
            .map(|(meta, master_key)| (meta.clone(), &**master_key))
            .collect();
        let skip = body.skip_keys.clone().unwrap_or_default();

        self.state
            .fido2
            .remove_key_for_operation(
                &body.label,
                &master_key_refs,
                optional_pin(body.pin.as_ref()),
                &skip,
                Fido2MutationContext {
                    operation_id: journal.operation_id(),
                    kind: "fido2.remove",
                    subject: Some(&body.label),
                },
            )
            .map_err(|error| map_fido2_service_error("FIDO2 removal", error))?;

        // H7: Invalidate all sessions — credential material has changed.
        self.state.invalidate_all_sessions();

        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(
            self.state.default_active_compartment_id(),
            AuditEventSpec::Fido2Remove {
                label: body.label.clone(),
                sessions_invalidated: true,
            },
        )?;

        Ok(Fido2RemoveResponse {
            status: "removed".into(),
            label: body.label,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, poll_fn};
    use std::sync::Arc;
    use std::task::Poll;

    use axum::http::StatusCode;
    use sigillum_api::{CompartmentDefinition, Fido2SetupRequest};
    use sigillum_fido2::error::Fido2Error;
    use tempfile::TempDir;

    use super::{SigillumService, map_fido2_service_error};
    use crate::AppState;

    #[tokio::test]
    async fn queued_first_run_setup_rejects_initialization_without_mutating() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        let service = SigillumService::new(state.clone());
        let held_operation = state.operation_guard().await;
        let mut queued = Box::pin(service.fido2_setup(Fido2SetupRequest {
            pin: None,
            label: "queued-key".into(),
            compartments: vec![CompartmentDefinition {
                label: "daily".into(),
                threshold: 1,
                passphrase_mode: None,
            }],
            passphrase: None,
        }));

        poll_fn(|context| match queued.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("setup bypassed the serialized initialization boundary"),
        })
        .await;
        std::fs::write(dir.path().join(".initialized"), b"1").unwrap();
        drop(held_operation);

        let error = queued.await.unwrap_err();
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(
            error.message(),
            "Already initialized. Use /api/fido2/register to add keys."
        );
        assert_eq!(state.pending_operation_count(), 0);
        assert_eq!(state.session_count(), 0);
        assert!(!state.is_unlocked());
        assert!(!dir.path().join("fido2_keys.json").exists());
        assert!(!dir.path().join("compartments").exists());
    }

    #[test]
    fn pin_not_set_error_is_actionable() {
        let error = map_fido2_service_error("FIDO2 setup", Fido2Error::PinNotSet);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("does not have a FIDO2 PIN"));
    }

    #[test]
    fn pin_required_error_is_actionable() {
        let error = map_fido2_service_error("FIDO2 unlock", Fido2Error::PinRequired);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("require its current FIDO2 PIN"));
    }

    #[test]
    fn pin_already_set_error_is_conflict() {
        let error = map_fido2_service_error("FIDO2 PIN setup", Fido2Error::PinAlreadySet);
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert!(error.message().contains("already has a FIDO2 PIN"));
    }

    #[test]
    fn pin_auth_blocked_error_is_actionable() {
        let error = map_fido2_service_error("FIDO2 setup", Fido2Error::PinAuthBlocked);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("Unplug and reinsert"));
        assert!(error.message().contains("power-cycle"));
    }

    #[test]
    fn raw_pin_policy_error_is_normalized() {
        let error = map_fido2_service_error(
            "FIDO2 PIN setup",
            Fido2Error::Other(
                "set_new_pin: response_status err = 0x37 CTAP2_ERR_PIN_POLICY_VIOLATION pin policy"
                    .into(),
            ),
        );
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("PIN policy"));
    }

    #[test]
    fn raw_pin_auth_blocked_error_is_normalized() {
        let error = map_fido2_service_error(
            "FIDO2 setup",
            Fido2Error::Other(
                "make_credential: response_status err = 0x34 CTAP2_ERR_PIN_AUTH_BLOCKED pinAuth blocked.Requires power recycle to reset.".into(),
            ),
        );
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("Unplug and reinsert"));
    }

    #[test]
    fn raw_pin_required_error_is_normalized() {
        let error = map_fido2_service_error(
            "FIDO2 unlock",
            Fido2Error::Other(
                "get_assertion: response_status err = 0x36 CTAP2_ERR_PIN_REQUIRED PIN is required for the selected operation.".into(),
            ),
        );
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("require its current FIDO2 PIN"));
    }

    #[test]
    fn multiple_devices_error_is_actionable() {
        let error = map_fido2_service_error(
            "FIDO2 registration",
            Fido2Error::MultipleDevicesDetected { count: 2 },
        );
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(error.message().contains("cannot tell which one to use"));
        assert!(
            error
                .message()
                .contains("Leave only the target key inserted")
        );
    }

    #[test]
    fn no_new_device_error_is_actionable() {
        let error = map_fido2_service_error("FIDO2 registration", Fido2Error::NoNewDeviceDetected);
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert!(
            error
                .message()
                .contains("Insert the new key you want to add")
        );
    }
}
