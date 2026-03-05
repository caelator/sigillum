//! # Sigillum FIDO2
//!
//! FIDO2 hardware key registration, compartment-based quorum unlock, and Shamir
//! secret sharing for Sigillum vault management.
//!
//! Each compartment has its own master key and threshold. The number of FIDO2 key
//! taps determines which compartment is unlocked. Each registered key holds
//! encrypted shards for ALL compartments.

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

use config::{CompartmentDef, Fido2Config, RegisteredKey, load_config, save_config};
use error::Fido2Error;
use types::{CompartmentInfo, Fido2Status, KeyInfo, QuorumEvent};

/// Result of registering a FIDO2 key.
pub struct RegisterResult {
    /// Master keys generated or passed through, keyed by compartment id.
    pub compartment_keys: Vec<(usize, Zeroizing<[u8; 32]>)>,
    /// Whether this was the first key registered.
    pub is_first_key: bool,
    /// Total registered keys after this registration.
    pub total_keys: usize,
}

/// Manages FIDO2 key registration, compartment quorum authentication, and Shamir shards.
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

    /// Expose config for external callers (CLI/daemon setup wizards).
    pub fn load_config_raw(&self) -> Fido2Config {
        self.load()
    }

    /// Save config from external callers (CLI/daemon setup wizards).
    pub fn save_config_raw(&self, config: &Fido2Config) -> Result<(), Fido2Error> {
        self.save(config)
    }

    // ── Compartment management ────────────────────────────────────

    /// Add a compartment definition. Validates threshold uniqueness.
    pub fn add_compartment(&self, def: CompartmentDef) -> Result<(), Fido2Error> {
        let mut config = self.load();
        if config.compartments.iter().any(|c| c.threshold == def.threshold) {
            return Err(Fido2Error::DuplicateThreshold { threshold: def.threshold });
        }
        config.compartments.push(def);
        self.save(&config)
    }

    /// Remove a compartment. Removes all shards for that compartment from all keys.
    pub fn remove_compartment(&self, id: usize) -> Result<(), Fido2Error> {
        let mut config = self.load();
        let idx = config.compartments.iter().position(|c| c.id == id)
            .ok_or(Fido2Error::CompartmentNotFound { id })?;
        config.compartments.remove(idx);
        let id_str = id.to_string();
        for key in &mut config.keys {
            key.shards.remove(&id_str);
        }
        self.save(&config)
    }

    /// Resolve compartment by threshold.
    pub fn resolve_compartment(&self, threshold: usize) -> Option<CompartmentDef> {
        self.load().resolve_compartment(threshold).cloned()
    }

    // ── Query ────────────────────────────────────────────────────

    pub fn status(&self) -> Fido2Status {
        let config = self.load();
        Fido2Status {
            enabled: config.is_fido2_enabled(),
            key_count: config.keys.len(),
            compartments: config.compartments.iter().map(|c| CompartmentInfo {
                id: c.id,
                label: c.label.clone(),
                threshold: c.threshold,
                has_passphrase: c.passphrase_mode.is_some(),
            }).collect(),
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
                compartment_ids: k.shards.keys()
                    .filter_map(|s| s.parse::<usize>().ok())
                    .collect(),
            })
            .collect()
    }

    pub fn is_enabled(&self) -> bool {
        self.load().is_fido2_enabled()
    }

    // ── Registration ─────────────────────────────────────────────

    /// Register a FIDO2 key across all compartments.
    ///
    /// - First key: `compartment_master_keys` is empty. New master keys are generated for each compartment.
    /// - Nth key: `compartment_master_keys` must contain `(compartment_id, master_key)` for each compartment.
    ///   All existing shards are re-split with fresh polynomials.
    #[cfg(feature = "hid")]
    pub fn register_key(
        &self,
        pin: &str,
        label: &str,
        compartment_master_keys: &[(usize, &[u8; 32])],
    ) -> Result<RegisterResult, Fido2Error> {
        let mut config = self.load();

        if config.compartments.is_empty() {
            return Err(Fido2Error::Config("no compartments defined".into()));
        }

        if config.keys.iter().any(|k| k.label == label) {
            return Err(Fido2Error::DuplicateKey { label: label.into() });
        }

        // Step 1: MakeCredential
        let cred = hid::make_credential(pin)?;
        let cred_id_hex = hex::encode(&cred.credential_id);

        if config.keys.iter().any(|k| k.credential_id_hex == cred_id_hex) {
            return Err(Fido2Error::DuplicateKey {
                label: format!("{label} (same physical key)"),
            });
        }

        // Step 2: Get hmac-secret for new key
        let new_hmac = hid::get_hmac_secret(&cred.credential_id, pin)?;

        let is_first = config.keys.is_empty();
        let total = config.keys.len() + 1;
        let mut result_keys: Vec<(usize, Zeroizing<[u8; 32]>)> = Vec::new();
        let mut new_key_shards = std::collections::HashMap::new();

        // For each compartment: determine master key, split, encrypt shards
        for comp in &config.compartments {
            let master_key = if is_first {
                let mut mk = Zeroizing::new([0u8; 32]);
                OsRng.fill_bytes(mk.as_mut());
                mk
            } else {
                let mk = compartment_master_keys.iter()
                    .find(|(id, _)| *id == comp.id)
                    .map(|(_, k)| *k)
                    .ok_or_else(|| Fido2Error::Other(format!(
                        "master key required for compartment {} to add Nth key", comp.id
                    )))?;
                Zeroizing::new(*mk)
            };

            let threshold = comp.threshold.min(total);
            let shards = crypto::split_master_key(&master_key, threshold, total)?;

            // Re-encrypt existing keys' shards for this compartment
            for (i, key) in config.keys.iter_mut().enumerate() {
                let existing_cred_id = hex::decode(&key.credential_id_hex)
                    .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
                let existing_hmac = hid::get_hmac_secret(&existing_cred_id, pin)?;
                let encrypted = crypto::encrypt_shard(&existing_hmac, &shards[i])?;
                key.shards.insert(comp.id.to_string(), hex::encode(&encrypted));
            }

            // Encrypt new key's shard for this compartment
            let encrypted_new = crypto::encrypt_shard(&new_hmac, &shards[total - 1])?;
            new_key_shards.insert(comp.id.to_string(), hex::encode(&encrypted_new));

            result_keys.push((comp.id, master_key));
        }

        let now = chrono::Utc::now().to_rfc3339();
        config.keys.push(RegisteredKey {
            label: label.into(),
            credential_id_hex: cred_id_hex,
            public_key_der_hex: hex::encode(&cred.public_key_der),
            public_key_pem: cred.public_key_pem,
            shards: new_key_shards,
            registered_at: now,
        });

        config.total_shares = config.keys.len();
        self.save(&config)?;

        Ok(RegisterResult {
            compartment_keys: result_keys,
            is_first_key: is_first,
            total_keys: config.keys.len(),
        })
    }

    /// Remove a registered key and re-split shards for remaining keys.
    #[cfg(feature = "hid")]
    pub fn remove_key(
        &self,
        label: &str,
        compartment_master_keys: &[(usize, &[u8; 32])],
        pin: &str,
    ) -> Result<(), Fido2Error> {
        let mut config = self.load();

        let idx = config.keys.iter().position(|k| k.label == label)
            .ok_or_else(|| Fido2Error::KeyNotFound { label: label.into() })?;

        let remaining = config.keys.len() - 1;
        // Check all compartment thresholds
        for comp in &config.compartments {
            if remaining > 0 && remaining < comp.threshold {
                return Err(Fido2Error::RemovalBelowQuorum {
                    remaining,
                    threshold: comp.threshold,
                });
            }
        }

        config.keys.remove(idx);

        if config.keys.is_empty() {
            config.total_shares = 0;
            self.save(&config)?;
            return Ok(());
        }

        // Re-split for remaining keys, for each compartment
        for comp in &config.compartments {
            let mk = compartment_master_keys.iter()
                .find(|(id, _)| *id == comp.id)
                .map(|(_, k)| *k)
                .ok_or_else(|| Fido2Error::Other(format!(
                    "master key required for compartment {} to re-split", comp.id
                )))?;

            let threshold = comp.threshold.min(remaining);
            let shards = crypto::split_master_key(mk, threshold, remaining)?;

            for (i, key) in config.keys.iter_mut().enumerate() {
                let cred_id = hex::decode(&key.credential_id_hex)
                    .map_err(|e| Fido2Error::Config(format!("bad credential_id hex: {e}")))?;
                let hmac = hid::get_hmac_secret(&cred_id, pin)?;
                let encrypted = crypto::encrypt_shard(&hmac, &shards[i])?;
                key.shards.insert(comp.id.to_string(), hex::encode(&encrypted));
            }
        }

        config.total_shares = config.keys.len();
        self.save(&config)?;
        Ok(())
    }

    // ── Unlock ───────────────────────────────────────────────────

    /// Authenticate for a specific compartment by tapping `target_threshold` keys.
    ///
    /// Returns `(compartment_id, reconstructed_master_key)`.
    #[cfg(feature = "hid")]
    pub fn authenticate_compartment(
        &self,
        pins: &[String],
        target_threshold: usize,
        event_tx: Option<std::sync::mpsc::Sender<QuorumEvent>>,
    ) -> Result<(usize, Zeroizing<[u8; 32]>), Fido2Error> {
        let config = self.load();

        if config.keys.is_empty() {
            return Err(Fido2Error::NoKeysRegistered);
        }

        let comp = config.resolve_compartment(target_threshold)
            .ok_or(Fido2Error::NoCompartmentForThreshold { threshold: target_threshold })?;

        if config.keys.len() < comp.threshold {
            return Err(Fido2Error::QuorumNotMet {
                required: comp.threshold,
                available: config.keys.len(),
            });
        }

        let emit = |evt: QuorumEvent| {
            if let Some(tx) = &event_tx {
                let _ = tx.send(evt);
            }
        };

        emit(QuorumEvent::CompartmentSelected {
            compartment_id: comp.id,
            compartment_label: comp.label.clone(),
            threshold: comp.threshold,
        });

        let comp_id_str = comp.id.to_string();
        let threshold = comp.threshold;
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

                let shard_hex = match key.shards.get(&comp_id_str) {
                    Some(s) => s,
                    None => continue,
                };

                let cred_id = match hex::decode(&key.credential_id_hex) {
                    Ok(id) => id,
                    Err(_) => continue,
                };

                match hid::get_hmac_secret(&cred_id, pin) {
                    Ok(hmac) => {
                        let encrypted = hex::decode(shard_hex).map_err(|e| {
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
        Ok((comp.id, master_key))
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
        assert!(s.compartments.is_empty());
    }

    #[test]
    fn list_keys_empty() {
        let (mgr, _dir) = test_manager();
        assert!(mgr.list_keys().is_empty());
    }

    #[test]
    fn add_compartment_validates_duplicate_threshold() {
        let (mgr, _dir) = test_manager();
        mgr.add_compartment(CompartmentDef {
            id: 0, label: "hot".into(), threshold: 1, passphrase_mode: None,
        }).unwrap();
        let err = mgr.add_compartment(CompartmentDef {
            id: 1, label: "also-hot".into(), threshold: 1, passphrase_mode: None,
        });
        assert!(err.is_err());
    }

    #[test]
    fn add_and_remove_compartment() {
        let (mgr, _dir) = test_manager();
        mgr.add_compartment(CompartmentDef {
            id: 0, label: "hot".into(), threshold: 1, passphrase_mode: None,
        }).unwrap();
        mgr.add_compartment(CompartmentDef {
            id: 1, label: "cold".into(), threshold: 2, passphrase_mode: None,
        }).unwrap();

        let s = mgr.status();
        assert_eq!(s.compartments.len(), 2);

        mgr.remove_compartment(0).unwrap();
        let s = mgr.status();
        assert_eq!(s.compartments.len(), 1);
        assert_eq!(s.compartments[0].label, "cold");
    }

    #[test]
    fn remove_nonexistent_compartment_errors() {
        let (mgr, _dir) = test_manager();
        assert!(mgr.remove_compartment(99).is_err());
    }

    #[test]
    fn resolve_compartment_by_threshold() {
        let (mgr, _dir) = test_manager();
        mgr.add_compartment(CompartmentDef {
            id: 0, label: "hot".into(), threshold: 1, passphrase_mode: None,
        }).unwrap();
        mgr.add_compartment(CompartmentDef {
            id: 1, label: "cold".into(), threshold: 2, passphrase_mode: None,
        }).unwrap();

        assert_eq!(mgr.resolve_compartment(1).unwrap().label, "hot");
        assert_eq!(mgr.resolve_compartment(2).unwrap().label, "cold");
        assert!(mgr.resolve_compartment(3).is_none());
    }
}
