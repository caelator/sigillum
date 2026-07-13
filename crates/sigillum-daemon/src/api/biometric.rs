use std::sync::Arc;

use crate::AppState;
use crate::audit_log::AuditEventSpec;
use crate::json_store::{JsonDocument, JsonSchema, load_json_document, save_json_document};
use crate::service::helpers::decode_hex;
use crate::service::unlock::PinnedSecretBytes;
use crate::service::{ServiceError, ServiceResult, require_full_session_token};
use k256::ecdsa::signature::Verifier;
use k256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sigillum_api::request::{BiometricEnrollRequest, BiometricUnlockRequest};
use sigillum_api::response::{
    BiometricChallengeResponse, BiometricEnrollResponse, UnlockResponse, UnlockedCompartment,
};
use sigillum_core::payload::biometric::BiometricUnlockPayload;
use sigillum_core::{VaultLifecycle, utils::derive_key_with_salt, utils::load_wrapped_master_key};
use sigillum_fido2::Fido2Manager;
use sigillum_fido2::config::CompartmentMeta;
use subtle::ConstantTimeEq;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BiometricEnrollment {
    compartment_id: usize,
    compartment_label: String,
    threshold: usize,
    public_key_hex: String,
    fingerprint_hex: String,
}

impl JsonDocument for BiometricEnrollment {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.biometric-enrollment", 1);
}

pub(crate) async fn issue_challenge(
    state: Arc<AppState>,
) -> ServiceResult<BiometricChallengeResponse> {
    let (challenge_id, nonce, expires_at_unix) = state.issue_biometric_challenge();
    Ok(BiometricChallengeResponse {
        challenge_id_hex: hex::encode(challenge_id),
        nonce_hex: hex::encode(nonce),
        expires_at_unix,
    })
}

pub(crate) async fn enroll(
    state: Arc<AppState>,
    token: Option<&str>,
    body: BiometricEnrollRequest,
) -> ServiceResult<BiometricEnrollResponse> {
    let token = require_full_session_token(&state, token)?.to_owned();
    let compartment_id = state
        .active_compartment_id_for(&token)
        .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
    let _guard = state.operation_guard().await;
    require_full_session_token(&state, Some(&token))?;
    if state.active_compartment_id_for(&token) != Some(compartment_id) {
        return Err(ServiceError::conflict(
            "Session compartment changed while biometric enrollment was waiting.",
        ));
    }

    verify_passphrase_for_compartment(&state, compartment_id, &body.passphrase)?;

    let public_key = decode_hex(&body.public_key_hex, "public_key_hex")?;
    let verifying_key = VerifyingKey::from_sec1_bytes(&public_key)
        .map_err(|error| ServiceError::bad_request(format!("Invalid public_key_hex: {error}")))?;
    let fingerprint_hex = enrollment_fingerprint(verifying_key.to_encoded_point(true).as_bytes());

    let meta = state
        .unlocked_compartments()
        .into_iter()
        .find(|meta| meta.id == compartment_id)
        .ok_or_else(|| ServiceError::forbidden("Active compartment is not unlocked."))?;

    let vault_key = state
        .with_active_vault_for(&token, |vault| vault.extract_master_key())
        .flatten()
        .ok_or_else(|| ServiceError::forbidden("Active compartment is not unlocked."))?;

    let enrollment = BiometricEnrollment {
        compartment_id,
        compartment_label: meta.label.clone(),
        threshold: meta.threshold,
        public_key_hex: hex::encode(verifying_key.to_encoded_point(true).as_bytes()),
        fingerprint_hex: fingerprint_hex.clone(),
    };

    save_json_document(&state.biometric_enrollment_path(), &enrollment).map_err(|error| {
        ServiceError::internal(format!("Failed to persist biometric enrollment: {error}"))
    })?;
    state
        .record_audit_event(
            Some(compartment_id),
            AuditEventSpec::BiometricEnroll {
                fingerprint_hex: fingerprint_hex.clone(),
            },
        )
        .map_err(|error| ServiceError::internal(format!("Failed to write audit log: {error}")))?;

    Ok(BiometricEnrollResponse {
        status: "enrolled".into(),
        compartment_id,
        fingerprint_hex,
        vault_key_hex: hex::encode(*vault_key),
    })
}

pub(crate) async fn unlock(
    state: Arc<AppState>,
    body: BiometricUnlockRequest,
) -> ServiceResult<UnlockResponse> {
    let lock_generation = state
        .unlock_generation_if_ready()
        .ok_or_else(|| ServiceError::locked("Daemon is locking."))?;
    if state.is_unlocked() {
        return Err(ServiceError::conflict("Vault is already unlocked."));
    }

    let enrollment = load_enrollment(&state)?;
    let payload_bytes = decode_hex(&body.payload_hex, "payload_hex")?;
    let payload = BiometricUnlockPayload::decode(&payload_bytes).map_err(|error| {
        ServiceError::bad_request(format!("Invalid biometric payload: {error}"))
    })?;
    let nonce = state
        .consume_biometric_challenge(&payload.challenge_id)
        .ok_or_else(|| {
            ServiceError::unauthorized("Biometric challenge is missing, expired, or already used.")
        })?;

    verify_signature(&enrollment, &nonce, &payload.proof)?;

    if payload.key_encoding != 1 {
        return Err(ServiceError::bad_request(format!(
            "Unsupported key_encoding {}.",
            payload.key_encoding
        )));
    }
    if payload.key.len() != 32 {
        return Err(ServiceError::bad_request(
            "Biometric helper returned an invalid vault key length.",
        ));
    }

    let mut pinned = PinnedSecretBytes::new(payload.key.to_vec()).map_err(|error| {
        ServiceError::internal(format!("Failed to pin biometric key material: {error}"))
    })?;
    let _guard = state.operation_guard().await;
    if state.unlock_generation_if_ready() != Some(lock_generation) {
        pinned.zeroize();
        return Err(ServiceError::locked(
            "A lock began while biometric unlock was in progress.",
        ));
    }

    let result = pinned
        .with_array_32(|master_key| {
            let verified_meta = load_meta_for_key(&state, enrollment.compartment_id, master_key)?;
            state
                .commit_unlock_if_current(lock_generation, || {
                    state.ensure_vault(enrollment.compartment_id);
                    let verified = state
                        .with_vault(enrollment.compartment_id, |vault| {
                            vault.load_master_key(*master_key);
                            let verified = vault.verify_master_key();
                            if !verified {
                                vault.zeroize_master_key();
                            }
                            verified
                        })
                        .unwrap_or(false);

                    if !verified {
                        state.record_unlock_failure();
                        return Err(ServiceError::unauthorized(
                            "Biometric proof was valid but the vault key was rejected.",
                        ));
                    }

                    state.reset_unlock_throttle();
                    state.unlock_compartment(
                        enrollment.compartment_id,
                        *master_key,
                        verified_meta.clone(),
                    );
                    let session_token = state.create_session(Some(enrollment.compartment_id));
                    state
                        .record_audit_event(
                            Some(enrollment.compartment_id),
                            AuditEventSpec::UnlockBiometric {
                                compartment_id: enrollment.compartment_id,
                                fingerprint_hex: enrollment.fingerprint_hex.clone(),
                            },
                        )
                        .map_err(|error| {
                            ServiceError::internal(format!("Failed to write audit log: {error}"))
                        })?;

                    Ok(UnlockResponse {
                        status: "unlocked".into(),
                        method: "biometric".into(),
                        cascading: Some(false),
                        session_token,
                        unlocked_compartments: vec![UnlockedCompartment {
                            id: verified_meta.id,
                            label: verified_meta.label,
                            threshold: verified_meta.threshold,
                            passphrase_mode: verified_meta.passphrase_mode,
                        }],
                        active_compartment_id: Some(enrollment.compartment_id),
                    })
                })
                .ok_or_else(|| {
                    ServiceError::locked("A lock began while biometric unlock was in progress.")
                })?
        })
        .ok_or_else(|| {
            ServiceError::bad_request("Biometric helper returned an invalid vault key length.")
        })?;
    pinned.zeroize();
    result
}

fn verify_passphrase_for_compartment(
    state: &AppState,
    compartment_id: usize,
    passphrase: &str,
) -> ServiceResult<()> {
    let salt = std::fs::read(state.salt_path(compartment_id)).map_err(|_| {
        ServiceError::bad_request("No passphrase is configured for the active compartment.")
    })?;
    if salt.len() != 32 {
        return Err(ServiceError::internal("Stored passphrase salt is invalid."));
    }

    let wrap_key = derive_key_with_salt(passphrase, &salt)?;
    let Some(master_key) =
        load_wrapped_master_key(&wrap_key, &state.wrapped_key_path(compartment_id))
    else {
        return Err(ServiceError::unauthorized(
            "Passphrase confirmation failed.",
        ));
    };
    let verified = state
        .with_vault(compartment_id, |vault| {
            vault
                .with_master_key(|loaded| bool::from(loaded.ct_eq(master_key.as_ref())))
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !verified {
        return Err(ServiceError::unauthorized(
            "Passphrase confirmation failed.",
        ));
    }
    Ok(())
}

fn load_enrollment(state: &AppState) -> ServiceResult<BiometricEnrollment> {
    load_json_document::<BiometricEnrollment>(&state.biometric_enrollment_path())
        .map_err(|error| {
            ServiceError::internal(format!("Failed to read biometric enrollment: {error}"))
        })?
        .ok_or_else(|| ServiceError::not_found("No biometric enrollment is configured."))
}

fn verify_signature(
    enrollment: &BiometricEnrollment,
    nonce: &[u8; 32],
    proof: &[u8],
) -> ServiceResult<()> {
    let public_key = hex::decode(&enrollment.public_key_hex).map_err(|error| {
        ServiceError::internal(format!("Stored biometric public key is invalid: {error}"))
    })?;
    let verifying_key = VerifyingKey::from_sec1_bytes(&public_key).map_err(|error| {
        ServiceError::internal(format!("Stored biometric public key is invalid: {error}"))
    })?;
    let signature = Signature::from_der(proof).map_err(|error| {
        ServiceError::unauthorized(format!("Biometric proof is invalid: {error}"))
    })?;
    verifying_key
        .verify(nonce, &signature)
        .map_err(|_| ServiceError::unauthorized("Biometric proof verification failed."))
}

fn load_meta_for_key(
    state: &AppState,
    compartment_id: usize,
    master_key: &[u8; 32],
) -> ServiceResult<CompartmentMeta> {
    Fido2Manager::load_compartment_meta(&state.base_dir, compartment_id, master_key).map_err(
        |error| ServiceError::internal(format!("Failed to load compartment metadata: {error}")),
    )
}

fn enrollment_fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    hex::encode(&digest[..16])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use k256::ecdsa::signature::Signer;
    use k256::ecdsa::{Signature, SigningKey};
    use sigillum_api::request::{
        BiometricEnrollRequest, BiometricUnlockRequest, CompartmentInitRequest,
        CompartmentSwitchRequest,
    };
    use sigillum_core::VaultLifecycle;
    use sigillum_core::payload::biometric::BiometricUnlockPayload;
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::{enroll, unlock, verify_passphrase_for_compartment};
    use crate::AppState;
    use crate::service::SigillumService;

    const PASSPHRASE: &str = "biometric-race-passphrase";

    async fn initialized_state() -> (TempDir, Arc<AppState>, SigillumService, String) {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        let service = SigillumService::new(state.clone());
        let initialized = service
            .init_compartment(
                None,
                CompartmentInitRequest {
                    id: 0,
                    passphrase: PASSPHRASE.into(),
                    label: Some("default".into()),
                    threshold: Some(1),
                },
            )
            .await
            .unwrap();
        (dir, state, service, initialized.session_token)
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes((&[7u8; 32]).into()).unwrap()
    }

    fn enroll_request(signing_key: &SigningKey) -> BiometricEnrollRequest {
        BiometricEnrollRequest {
            public_key_hex: hex::encode(
                signing_key
                    .verifying_key()
                    .to_encoded_point(true)
                    .as_bytes(),
            ),
            passphrase: PASSPHRASE.into(),
        }
    }

    #[tokio::test]
    async fn biometric_enroll_rejects_token_rotated_while_waiting() {
        let (_dir, state, service, session) = initialized_state().await;
        state.unlock_compartment(
            1,
            [8u8; 32],
            CompartmentMeta {
                id: 1,
                label: "secure".into(),
                threshold: 2,
                passphrase_mode: None,
            },
        );
        let signing_key = signing_key();
        let held_operation = state.operation_guard().await;

        let mut switch = Box::pin(
            service.switch_compartment(Some(&session), CompartmentSwitchRequest { id: 1 }),
        );
        tokio::select! {
            biased;
            result = &mut switch => panic!("switch must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        let mut enrollment = Box::pin(enroll(
            state.clone(),
            Some(&session),
            enroll_request(&signing_key),
        ));
        tokio::select! {
            biased;
            result = &mut enrollment => panic!("enrollment must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        drop(held_operation);
        let switched = switch.await.unwrap();
        let error = enrollment
            .await
            .expect_err("old-token enrollment must fail after rotation");
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(!state.biometric_enrollment_path().exists());
        assert_eq!(
            state.active_compartment_id_for(&switched.session_token),
            Some(1)
        );
    }

    #[tokio::test]
    async fn lock_generation_preempts_biometric_unlock_waiting_for_operation_mutex() {
        let (_dir, state, service, session) = initialized_state().await;
        let signing_key = signing_key();
        let enrollment = enroll(state.clone(), Some(&session), enroll_request(&signing_key))
            .await
            .unwrap();
        let master_key: [u8; 32] = hex::decode(&enrollment.vault_key_hex)
            .unwrap()
            .try_into()
            .unwrap();
        let meta = state.unlocked_compartments().into_iter().next().unwrap();
        state.lock_all();

        let (challenge_id, nonce, _) = state.issue_biometric_challenge();
        let proof: Signature = signing_key.sign(&nonce);
        let payload = BiometricUnlockPayload::new(
            challenge_id,
            proof.to_der().as_bytes().to_vec(),
            1,
            master_key.to_vec(),
        )
        .unwrap();

        let held_operation = state.operation_guard().await;
        let mut biometric_unlock = Box::pin(unlock(
            state.clone(),
            BiometricUnlockRequest {
                payload_hex: hex::encode(payload.encode()),
            },
        ));
        tokio::select! {
            biased;
            result = &mut biometric_unlock => panic!("biometric unlock must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }

        // Model a concurrent unlock winning first, followed immediately by a
        // real lock request while the original biometric attempt is queued.
        state.unlock_compartment(0, master_key, meta);
        let intervening_session = state.create_session(Some(0));
        let mut lock = Box::pin(service.lock_all(Some(&intervening_session)));
        tokio::select! {
            biased;
            result = &mut lock => panic!("lock must wait for the operation mutex: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert!(state.is_locking());

        drop(held_operation);
        let error = biometric_unlock
            .await
            .expect_err("stale biometric unlock must not reopen after a lock");
        assert_eq!(error.status(), axum::http::StatusCode::LOCKED);
        assert_eq!(lock.await.unwrap().status, "locked");
        assert!(!state.is_unlocked());
        assert_eq!(state.session_count(), 0);
    }

    #[tokio::test]
    async fn passphrase_confirmation_does_not_replace_live_master_key() {
        let (_dir, state, _service, session) = initialized_state().await;
        let before = state
            .with_vault(0, |vault| vault.extract_master_key())
            .flatten()
            .unwrap();

        verify_passphrase_for_compartment(&state, 0, PASSPHRASE).unwrap();

        let after = state
            .with_vault(0, |vault| vault.extract_master_key())
            .flatten()
            .unwrap();
        assert_eq!(&*after, &*before);
        assert!(state.verify_token(&session));
    }
}
