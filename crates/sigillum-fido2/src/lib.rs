//! # Sigillum FIDO2
//!
//! FIDO2 hardware key registration, quorum-based unlock, and Shamir secret
//! sharing for Sigillum vault management.
//!
//! The `Fido2Manager` is decoupled from `FileVault` — it returns master keys
//! to the caller, which loads them into the vault. This keeps the FIDO2 crate
//! independently testable and IO-agnostic.

pub mod config;
pub mod crypto;
pub mod error;
#[cfg(feature = "hid")]
pub mod hid;
pub mod types;

use std::path::PathBuf;

use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

use config::{Fido2Config, RegisteredKey, load_config, save_config};
use error::Fido2Error;
use types::{Fido2Status, KeyInfo, QuorumEvent};

/// Result of registering a FIDO2 key.
pub struct RegisterResult {
    /// The master key (generated on first key, passed through on Nth key).
    pub master_key: Zeroizing<[u8; 32]>,
    /// Whether this was the first key registered (vault needs initialization).
    pub is_first_key: bool,
    /// Total registered keys after this registration.
    pub total_keys: usize,
}

/// Manages FIDO2 key registration, quorum authentication, and Shamir shards.
///
/// Does NOT hold or access the vault directly. The caller bridges between
/// `Fido2Manager` outputs and `VaultLifecycle`.
pub struct Fido2Manager {
    config_path: PathBuf,
}

impl Fido2Manager {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }

    fn load(&self) -> Fido2Config {
        load_config(&self.config_path)
    }

    fn save(&self, config: &Fido2Config) -> Result<(), Fido2Error> {
        save_config(&self.config_path, config)
    }

    /// Expose config for external callers (CLI setup wizard).
    pub fn load_config_raw(&self) -> Fido2Config {
        self.load()
    }

    /// Save config from external callers (CLI setup wizard).
    pub fn save_config_raw(&self, config: &Fido2Config) -> Result<(), Fido2Error> {
        self.save(config)
    }

    // ── Query ────────────────────────────────────────────────────

    pub fn status(&self) -> Fido2Status {
        let config = self.load();
        Fido2Status {
            enabled: config.is_fido2_enabled(),
            key_count: config.keys.len(),
            quorum_threshold: config.quorum_threshold,
            unlock_method: config.unlock_method.clone(),
        }
    }

    pub fn list_keys(&self) -> Vec<KeyInfo> {
        self.load()
            .keys
            .iter()
            .map(|k| KeyInfo {
                label: k.label.clone(),
                credential_id_short: k.credential_id_hex.chars().take(16).collect(),
                registered_at: k.registered_at.clone(),
            })
            .collect()
    }

    pub fn is_enabled(&self) -> bool {
        self.load().is_fido2_enabled()
    }

    pub fn quorum_threshold(&self) -> usize {
        self.load().quorum_threshold
    }

    pub fn unlock_method(&self) -> String {
        self.load().unlock_method.clone()
    }

    // ── Registration ─────────────────────────────────────────────

    /// Register a FIDO2 key.
    ///
    /// - First key: `existing_master_key` is `None`. A new master key is generated.
    /// - Nth key: `existing_master_key` must be provided (extracted from unlocked vault).
    ///   All existing shards are re-split with a fresh polynomial.
    #[cfg(feature = "hid")]
    pub fn register_key(
        &self,
        pin: &str,
        label: &str,
        existing_master_key: Option<&[u8; 32]>,
    ) -> Result<RegisterResult, Fido2Error> {
        let mut config = self.load();

        // Check for duplicate label
        if config.keys.iter().any(|k| k.label == label) {
            return Err(Fido2Error::DuplicateKey {
                label: label.into(),
            });
        }

        // Step 1: MakeCredential
        let cred = hid::make_credential(pin)?;
        let cred_id_hex = hex::encode(&cred.credential_id);

        // Check for duplicate credential
        if config.keys.iter().any(|k| k.credential_id_hex == cred_id_hex) {
            return Err(Fido2Error::DuplicateKey {
                label: format!("{label} (same physical key)"),
            });
        }

        // Step 2: Get hmac-secret
        let hmac = hid::get_hmac_secret(&cred.credential_id, pin)?;

        // Step 3: Determine master key
        let is_first = config.keys.is_empty();
        let master_key = if is_first {
            let mut mk = Zeroizing::new([0u8; 32]);
            OsRng.fill_bytes(mk.as_mut());
            mk
        } else {
            let mk = existing_master_key
                .ok_or_else(|| Fido2Error::Other("master key required to add Nth key".into()))?;
            Zeroizing::new(*mk)
        };

        // Step 4: Split master key (threshold-of-total)
        let total = config.keys.len() + 1;
        let threshold = config.quorum_threshold.min(total);
        let shards = crypto::split_master_key(&master_key, threshold, total)?;

        // Step 5: Re-encrypt existing keys' shards (tap each one)
        for (i, key) in config.keys.iter_mut().enumerate() {
            let existing_cred_id = hex::decode(&key.credential_id_hex)
                .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
            let existing_hmac = hid::get_hmac_secret(&existing_cred_id, pin)?;
            let encrypted = crypto::encrypt_shard(&existing_hmac, &shards[i])?;
            key.encrypted_shard_hex = hex::encode(&encrypted);
        }

        // Step 6: Encrypt new key's shard
        let encrypted_new = crypto::encrypt_shard(&hmac, &shards[total - 1])?;

        let now = chrono::Utc::now().to_rfc3339();
        config.keys.push(RegisteredKey {
            label: label.into(),
            credential_id_hex: cred_id_hex,
            public_key_der_hex: hex::encode(&cred.public_key_der),
            public_key_pem: cred.public_key_pem,
            encrypted_shard_hex: hex::encode(&encrypted_new),
            registered_at: now,
        });

        if is_first {
            config.unlock_method = "fido2".into();
        }

        self.save(&config)?;

        Ok(RegisterResult {
            master_key,
            is_first_key: is_first,
            total_keys: config.keys.len(),
        })
    }

    /// Remove a registered key and re-split shards for remaining keys.
    #[cfg(feature = "hid")]
    pub fn remove_key(
        &self,
        label: &str,
        master_key: &[u8; 32],
        pin: &str,
    ) -> Result<(), Fido2Error> {
        let mut config = self.load();

        let idx = config
            .keys
            .iter()
            .position(|k| k.label == label)
            .ok_or_else(|| Fido2Error::KeyNotFound {
                label: label.into(),
            })?;

        let remaining = config.keys.len() - 1;
        if remaining > 0 && remaining < config.quorum_threshold {
            return Err(Fido2Error::RemovalBelowQuorum {
                remaining,
                threshold: config.quorum_threshold,
            });
        }

        config.keys.remove(idx);

        if config.keys.is_empty() {
            config.unlock_method = "passphrase".into();
            self.save(&config)?;
            return Ok(());
        }

        // Re-split for remaining keys
        let threshold = config.quorum_threshold.min(remaining);
        let shards = crypto::split_master_key(master_key, threshold, remaining)?;

        for (i, key) in config.keys.iter_mut().enumerate() {
            let cred_id = hex::decode(&key.credential_id_hex)
                .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
            let hmac = hid::get_hmac_secret(&cred_id, pin)?;
            let encrypted = crypto::encrypt_shard(&hmac, &shards[i])?;
            key.encrypted_shard_hex = hex::encode(&encrypted);
        }

        self.save(&config)?;
        Ok(())
    }

    /// Set the quorum threshold.
    pub fn set_quorum(&self, threshold: usize) -> Result<(), Fido2Error> {
        let mut config = self.load();

        if threshold == 0 {
            return Err(Fido2Error::Other("threshold must be >= 1".into()));
        }
        if threshold > config.keys.len() && !config.keys.is_empty() {
            return Err(Fido2Error::Other(format!(
                "threshold {} exceeds registered key count {}",
                threshold,
                config.keys.len()
            )));
        }

        config.quorum_threshold = threshold;
        self.save(&config)?;
        Ok(())
    }

    // ── Unlock ───────────────────────────────────────────────────

    /// Authenticate quorum: tap M keys, decrypt M shards, reconstruct master key.
    ///
    /// `pins` should have one PIN per round. If all keys share the same PIN,
    /// pass a single-element vec (it will be reused).
    ///
    /// Returns the reconstructed master key. Caller loads it into the vault.
    #[cfg(feature = "hid")]
    pub fn authenticate_quorum(
        &self,
        pins: &[String],
        event_tx: Option<std::sync::mpsc::Sender<QuorumEvent>>,
    ) -> Result<Zeroizing<[u8; 32]>, Fido2Error> {
        let config = self.load();

        if config.keys.is_empty() {
            return Err(Fido2Error::NoKeysRegistered);
        }

        let threshold = config.quorum_threshold;
        if config.keys.len() < threshold {
            return Err(Fido2Error::QuorumNotMet {
                required: threshold,
                available: config.keys.len(),
            });
        }

        let emit = |evt: QuorumEvent| {
            if let Some(tx) = &event_tx {
                let _ = tx.send(evt);
            }
        };

        let mut decrypted_shards: Vec<Vec<u8>> = Vec::with_capacity(threshold);
        let mut used_cred_ids: Vec<String> = Vec::new();

        for round in 0..threshold {
            let pin = pins.get(round).or(pins.first()).ok_or_else(|| {
                Fido2Error::Other("no PIN provided".into())
            })?;

            emit(QuorumEvent::RoundStart {
                round: round + 1,
                total: threshold,
            });

            let mut matched = false;
            for key in &config.keys {
                if used_cred_ids.contains(&key.credential_id_hex) {
                    continue;
                }

                let cred_id = match hex::decode(&key.credential_id_hex) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                match hid::get_hmac_secret(&cred_id, pin) {
                    Ok(hmac) => {
                        let encrypted = hex::decode(&key.encrypted_shard_hex).map_err(|e| {
                            Fido2Error::Config(format!("bad shard hex: {e}"))
                        })?;
                        let shard = crypto::decrypt_shard(&hmac, &encrypted)?;
                        decrypted_shards.push(shard);
                        used_cred_ids.push(key.credential_id_hex.clone());

                        emit(QuorumEvent::RoundComplete {
                            round: round + 1,
                            total: threshold,
                            key_label: key.label.clone(),
                        });

                        matched = true;
                        break;
                    }
                    Err(_) => continue,
                }
            }

            if !matched {
                emit(QuorumEvent::Error {
                    message: "No matching key found for this round".into(),
                });
                return Err(Fido2Error::NoDevice);
            }

            if round + 1 < threshold {
                emit(QuorumEvent::SwapKeys {
                    round: round + 1,
                    total: threshold,
                });
            }
        }

        let master_key = crypto::reconstruct_master_key(&decrypted_shards)?;
        emit(QuorumEvent::Unlocked);
        Ok(master_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manager() -> (Fido2Manager, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");
        (Fido2Manager::new(path), dir)
    }

    #[test]
    fn status_empty() {
        let (mgr, _dir) = test_manager();
        let s = mgr.status();
        assert!(!s.enabled);
        assert_eq!(s.key_count, 0);
        assert_eq!(s.quorum_threshold, 1);
        assert_eq!(s.unlock_method, "passphrase");
    }

    #[test]
    fn list_keys_empty() {
        let (mgr, _dir) = test_manager();
        assert!(mgr.list_keys().is_empty());
    }

    #[test]
    fn set_quorum_validates() {
        let (mgr, _dir) = test_manager();
        assert!(mgr.set_quorum(0).is_err());
    }
}
