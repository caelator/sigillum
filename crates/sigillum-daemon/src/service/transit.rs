//! Transit encryption and HMAC operations.
//!
//! Provides AES-256-GCM encryption/decryption and HMAC-SHA256 signing
//! for transient data using compartment-derived keys.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use sigillum_api::{
    TransitDecryptRequest, TransitDecryptResponse, TransitEncryptRequest, TransitEncryptResponse,
    TransitHmacRequest, TransitHmacResponse,
};
use sigillum_core::VaultLifecycle;
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;

use super::helpers::{decode_fixed_hex, decode_hex, decode_optional_hex};
use super::{ServiceError, ServiceResult, SigillumService};

type HmacSha256 = Hmac<Sha256>;

impl SigillumService {
    pub(crate) fn transit_encrypt(
        &self,
        token: Option<&str>,
        body: TransitEncryptRequest,
    ) -> ServiceResult<TransitEncryptResponse> {
        let token = self.require_session(token)?;
        let key_name = body.key;
        let plaintext = Zeroizing::new(decode_hex(&body.plaintext_hex, "plaintext")?);
        let aad = decode_optional_hex(body.aad_hex.as_deref(), "aad")?;

        let (nonce_hex, ciphertext_hex, compartment_id) =
            self.with_active_vault(token, |vault, compartment_id| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::vault_locked("Vault is locked."))?;
                let transit_key = derive_transit_key(master_key.as_ref(), &key_name)?;
                let cipher = Aes256Gcm::new_from_slice(&transit_key).map_err(|error| {
                    ServiceError::internal(format!("Transit cipher init failed: {error}"))
                })?;
                let mut nonce_bytes = [0u8; 12];
                rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
                let payload = Payload {
                    msg: plaintext.as_ref(),
                    aad: aad.as_deref().unwrap_or(&[]),
                };
                let ciphertext = cipher
                    .encrypt(Nonce::from_slice(&nonce_bytes), payload)
                    .map_err(|error| {
                        ServiceError::internal(format!("Transit encryption failed: {error}"))
                    })?;
                Ok((
                    hex::encode(nonce_bytes),
                    hex::encode(ciphertext),
                    compartment_id,
                ))
            })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::TransitEncrypt {
                key: key_name.clone(),
                ciphertext_len: ciphertext_hex.len() / 2,
            },
        )?;

        Ok(TransitEncryptResponse {
            key: key_name,
            nonce_hex,
            ciphertext_hex,
        })
    }

    pub(crate) fn transit_decrypt(
        &self,
        token: Option<&str>,
        body: TransitDecryptRequest,
    ) -> ServiceResult<TransitDecryptResponse> {
        let token = self.require_session(token)?;
        let key_name = body.key;
        let nonce = decode_fixed_hex::<12>(&body.nonce_hex, "nonce")?;
        let ciphertext = decode_hex(&body.ciphertext_hex, "ciphertext")?;
        let aad = decode_optional_hex(body.aad_hex.as_deref(), "aad")?;

        let (plaintext_hex, compartment_id) =
            self.with_active_vault(token, |vault, compartment_id| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::vault_locked("Vault is locked."))?;
                let transit_key = derive_transit_key(master_key.as_ref(), &key_name)?;
                let cipher = Aes256Gcm::new_from_slice(&transit_key).map_err(|error| {
                    ServiceError::internal(format!("Transit cipher init failed: {error}"))
                })?;
                let payload = Payload {
                    msg: ciphertext.as_ref(),
                    aad: aad.as_deref().unwrap_or(&[]),
                };
                let plaintext = cipher
                    .decrypt(Nonce::from_slice(&nonce), payload)
                    .map_err(|_| ServiceError::unauthorized("Transit decryption failed."))?;
                Ok((hex::encode(plaintext), compartment_id))
            })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::TransitDecrypt {
                key: key_name.clone(),
                plaintext_len: plaintext_hex.len() / 2,
            },
        )?;

        Ok(TransitDecryptResponse {
            key: key_name,
            plaintext_hex,
        })
    }

    pub(crate) fn transit_hmac(
        &self,
        token: Option<&str>,
        body: TransitHmacRequest,
    ) -> ServiceResult<TransitHmacResponse> {
        let token = self.require_session(token)?;
        let key_name = body.key;
        let input = decode_hex(&body.input_hex, "input")?;

        let (digest_hex, compartment_id) =
            self.with_active_vault(token, |vault, compartment_id| {
                let master_key = vault
                    .extract_master_key()
                    .ok_or_else(|| ServiceError::vault_locked("Vault is locked."))?;
                let transit_key = derive_transit_key(master_key.as_ref(), &key_name)?;
                let mut mac =
                    <HmacSha256 as Mac>::new_from_slice(&transit_key).map_err(|error| {
                        ServiceError::internal(format!("Transit hmac init failed: {error}"))
                    })?;
                mac.update(&input);
                Ok((hex::encode(mac.finalize().into_bytes()), compartment_id))
            })?;

        self.record_audit(
            Some(compartment_id),
            AuditEventSpec::TransitHmac {
                key: key_name.clone(),
                input_len: input.len(),
            },
        )?;

        Ok(TransitHmacResponse {
            key: key_name,
            digest_hex,
        })
    }
}

fn derive_transit_key(master_key: &[u8], key_name: &str) -> ServiceResult<[u8; 32]> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(master_key).map_err(|error| {
        ServiceError::internal(format!("Transit key derivation failed: {error}"))
    })?;
    mac.update(b"sigillum/transit/v1/");
    mac.update(key_name.as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{TransitDecryptRequest, TransitEncryptRequest, TransitHmacRequest};
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;

    fn meta(id: usize, threshold: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold,
            passphrase_mode: None,
        }
    }

    #[test]
    fn transit_roundtrip_uses_active_compartment_keyspace() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("app state should initialize"));
        state.unlock_compartment(0, [7u8; 32], meta(0, 1, "default"));
        let session = state.create_session(Some(0));
        let service = SigillumService::new(state);

        let encrypted = service
            .transit_encrypt(
                Some(&session),
                TransitEncryptRequest {
                    key: "payments".into(),
                    plaintext_hex: hex::encode(b"secret-data"),
                    aad_hex: Some(hex::encode(b"aad")),
                },
            )
            .unwrap();

        let decrypted = service
            .transit_decrypt(
                Some(&session),
                TransitDecryptRequest {
                    key: "payments".into(),
                    nonce_hex: encrypted.nonce_hex,
                    ciphertext_hex: encrypted.ciphertext_hex,
                    aad_hex: Some(hex::encode(b"aad")),
                },
            )
            .unwrap();
        assert_eq!(
            hex::decode(decrypted.plaintext_hex).unwrap(),
            b"secret-data".to_vec()
        );

        let digest = service
            .transit_hmac(
                Some(&session),
                TransitHmacRequest {
                    key: "payments".into(),
                    input_hex: hex::encode(b"mac-me"),
                },
            )
            .unwrap();
        assert_eq!(hex::decode(digest.digest_hex).unwrap().len(), 32);
    }
}
