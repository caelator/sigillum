//! FIDO2 configuration persistence and compartment metadata types.
//!
//! This module manages the persistent state of FIDO2-secured vaults. The key design principle
//! is **deniability**: `Fido2Config` stored in `fido2_keys.json` contains registered keys,
//! share counts, and internal recovery metadata—never compartment definitions. Compartment
//! metadata is discovered at unlock time by decrypting tagged shards with the derived
//! hmac-secret.
//!
//! ## Configuration Structure
//!
//! - **`fido2_keys.json`**: Plaintext JSON containing:
//!   - `total_shares`: Total number of shards across all hardware keys.
//!   - `keys`: Array of registered hardware key metadata (label, credential ID, public key, etc.)
//!   - `generation` and `last_mutation`: Internal crash-recovery metadata that binds the
//!     latest atomic manager write to a daemon operation without revealing compartments.
//!   - Each key has exactly `SHARD_SLOTS` (100) hex-encoded shard blobs: real shards use
//!     AES-256-GCM encryption, padding entries are random bytes indistinguishable without the key.
//!
//! - **`compartments/{id}/meta.enc`**: Encrypted compartment metadata (AES-256-GCM with
//!   the compartment's master key, zero-padded to 128 bytes).
//!
//! ## Design Rationale
//!
//! Storing compartments only in encrypted form provides plausible deniability: an observer
//! examining the vault directory cannot determine which hardware keys correspond to which
//! compartments without possession of those hardware keys. The shard padding further obfuscates
//! the actual number of compartments.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigillum_core::utils::atomic_write;

use crate::error::Fido2Error;

#[must_use = "check the Result to ensure data was persisted"]
pub fn atomic_write_bytes(path: &Path, data: &[u8]) -> Result<(), std::io::Error> {
    atomic_write(path, data)
}

/// Number of shard slots per registered FIDO2 key.
///
/// Always exactly 100. Real shards use a subset of these slots, with the remainder
/// filled with random padding blobs. This prevents an observer from determining the
/// actual number of compartments by inspecting the config file.
pub const SHARD_SLOTS: usize = 100;

/// Encrypted compartment metadata stored in each compartment's `meta.enc` file.
///
/// Contains the compartment's identity, label, Shamir threshold, and optional passphrase mode.
/// Encrypted with AES-256-GCM using the compartment's master key and zero-padded to 128 bytes.
/// Never stored in plaintext in `fido2_keys.json` to maintain deniability.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CompartmentMeta {
    /// Unique compartment ID within the vault.
    pub id: usize,
    /// Human-readable label for this compartment (e.g., "Production", "Legacy").
    pub label: String,
    /// Shamir threshold for this compartment (e.g., 2 means 2-of-3 recovery).
    pub threshold: usize,
    /// Optional passphrase mode (e.g., "wrapped" for passphrase-wrapping flows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase_mode: Option<String>,
}

/// Persisted FIDO2 configuration state stored in `fido2_keys.json`.
///
/// Contains registered FIDO2 hardware key metadata, the total share count, and
/// internal crash-recovery metadata.
/// Compartment definitions themselves are never stored here—they are discovered at
/// unlock time by attempting to decrypt tagged shards with each derived hmac-secret.
/// This design provides deniability: an observer cannot determine which hardware keys
/// correspond to which compartments without the hardware devices.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Fido2Config {
    /// Monotonic generation incremented by every manager-mediated config mutation.
    ///
    /// Older configs deserialize at generation zero. Recovery uses this value together
    /// with [`last_mutation`](Self::last_mutation) to distinguish the exact write that
    /// belongs to an interrupted daemon operation from unrelated or newer writes.
    #[serde(default)]
    pub generation: u64,
    /// Total number of shards distributed across all registered keys.
    pub total_shares: usize,
    /// Array of registered FIDO2 hardware keys.
    pub keys: Vec<RegisteredKey>,
    /// Receipt for the most recent manager-mediated mutation.
    ///
    /// This is embedded in the same atomic JSON write as the key state: a sidecar
    /// receipt could become durable without the config (or vice versa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_mutation: Option<Fido2MutationReceipt>,
}

/// Causal receipt embedded in the FIDO2 config's atomic state transition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Fido2MutationReceipt {
    /// Journal operation ID, or a standalone ID for CLI/raw manager writes.
    pub operation_id: String,
    /// Stable operation kind (for example `fido2.register`).
    pub kind: String,
    /// Key label associated with the mutation, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Resulting config generation.
    pub generation: u64,
    /// Resulting number of registered keys.
    pub result_key_count: usize,
    /// SHA-256 of the resulting generation, share count, and key state.
    pub state_fingerprint: String,
}

/// A single registered FIDO2 hardware key and its associated shards.
///
/// Stores metadata about a hardware key and exactly [`SHARD_SLOTS`] (100) hex-encoded
/// shard blobs. Real shards are encrypted with the hmac-secret derived from the key and
/// contain the compartment ID and Shamir share data. Padding entries are random bytes
/// indistinguishable from real shards without the hmac-secret.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisteredKey {
    /// Human-readable label for this hardware key (e.g., "YubiKey #1").
    pub label: String,
    /// Credential ID in hex encoding (unique per credential).
    pub credential_id_hex: String,
    /// Public key in DER format, hex-encoded.
    pub public_key_der_hex: String,
    /// Public key in PEM format (for easy inspection and verification).
    pub public_key_pem: String,
    /// Exactly [`SHARD_SLOTS`] hex-encoded blobs: real shards are `AES-256-GCM(hmac_secret,
    /// comp_id || shard_data)`, padding entries are random bytes.
    pub shards: Vec<String>,
    /// ISO 8601 timestamp when this key was registered.
    pub registered_at: String,
}

impl Fido2Config {
    /// Check if FIDO2 authentication is configured (at least one key is registered).
    pub fn is_fido2_enabled(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Stamp the exact mutation that produced this config before the atomic save.
    pub fn record_mutation(
        &mut self,
        operation_id: impl Into<String>,
        kind: impl Into<String>,
        subject: Option<String>,
    ) -> Result<(), Fido2Error> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Fido2Error::Config("FIDO2 config generation overflow".into()))?;
        self.last_mutation = None;
        let state_fingerprint = config_state_fingerprint(self)?;
        self.last_mutation = Some(Fido2MutationReceipt {
            operation_id: operation_id.into(),
            kind: kind.into(),
            subject,
            generation: self.generation,
            result_key_count: self.keys.len(),
            state_fingerprint,
        });
        Ok(())
    }

    /// Verify that the latest receipt is causally bound to the current config state.
    pub fn mutation_receipt_matches(
        &self,
        operation_id: &str,
        kind: &str,
        subject: Option<&str>,
    ) -> Result<bool, Fido2Error> {
        let Some(receipt) = self.last_mutation.as_ref() else {
            return Ok(false);
        };
        Ok(receipt.operation_id == operation_id
            && receipt.kind == kind
            && receipt.subject.as_deref() == subject
            && receipt.generation == self.generation
            && receipt.result_key_count == self.keys.len()
            && receipt.state_fingerprint == config_state_fingerprint(self)?)
    }
}

/// Compute the canonical fingerprint used by mutation receipts.
///
/// The receipt itself is excluded to avoid a circular hash. Its generation, result
/// count, operation identity, and kind are checked separately by recovery.
pub fn config_state_fingerprint(config: &Fido2Config) -> Result<String, Fido2Error> {
    #[derive(Serialize)]
    struct FingerprintInput<'a> {
        domain: &'static str,
        generation: u64,
        total_shares: usize,
        keys: &'a [RegisteredKey],
    }

    let encoded = serde_json::to_vec(&FingerprintInput {
        domain: "sigillum.fido2-config-state.v1",
        generation: config.generation,
        total_shares: config.total_shares,
        keys: &config.keys,
    })
    .map_err(|error| Fido2Error::Config(format!("fingerprint config: {error}")))?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

/// Load FIDO2 configuration from disk, returning an empty default if the file is missing.
///
/// Reads and parses `fido2_keys.json`. If the file does not exist, returns a default
/// empty configuration. If parsing fails, returns an error.
pub fn load_config(path: &Path) -> Result<Fido2Config, Fido2Error> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| Fido2Error::Config(format!("parse {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Fido2Config::default()),
        Err(e) => Err(Fido2Error::Config(format!("read {}: {e}", path.display()))),
    }
}

/// Save FIDO2 configuration to disk atomically.
///
/// Serializes the config to pretty-printed JSON and writes it to `fido2_keys.json`
/// using atomic write to prevent data loss on crash. Creates parent directories if needed.
#[must_use = "check the Result to ensure config was persisted"]
pub fn save_config(path: &Path, config: &Fido2Config) -> Result<(), Fido2Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Fido2Error::Config(format!("create dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| Fido2Error::Config(format!("serialize: {e}")))?;
    atomic_write_bytes(path, json.as_bytes())
        .map_err(|e| Fido2Error::Config(format!("write: {e}")))?;
    Ok(())
}

/// Generate random hex-encoded padding blobs for deniability.
///
/// Creates `count` random hex strings of length `byte_len * 2`, each indistinguishable
/// from encrypted shards to an observer without the hmac-secret key. Attempting to decrypt
/// a dummy blob will fail at the AEAD authentication tag verification step.
pub fn generate_dummy_shards(count: usize, byte_len: usize) -> Vec<String> {
    use rand::RngCore;
    use rand::rngs::OsRng;

    (0..count)
        .map(|_| {
            let mut buf = vec![0u8; byte_len];
            OsRng.fill_bytes(&mut buf);
            hex::encode(&buf)
        })
        .collect()
}

/// Validate that all compartments have unique Shamir thresholds.
///
/// Returns an error if any two compartments share the same threshold, as duplicate
/// thresholds would create ambiguity during share reconstruction.
#[must_use = "check the Result for threshold validation errors"]
pub fn validate_thresholds(metas: &[CompartmentMeta]) -> Result<(), Fido2Error> {
    let mut seen = std::collections::HashSet::new();
    for m in metas {
        if !seen.insert(m.threshold) {
            return Err(Fido2Error::DuplicateThreshold {
                threshold: m.threshold,
            });
        }
    }
    Ok(())
}

/// Calculate the next available compartment ID given a set of existing compartments.
///
/// Returns one more than the maximum ID in the provided metas, or 0 if the list is empty.
pub fn next_compartment_id(metas: &[CompartmentMeta]) -> usize {
    metas.iter().map(|m| m.id).max().map_or(0, |m| m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");

        let config = Fido2Config {
            total_shares: 5,
            keys: vec![RegisteredKey {
                label: "test-key".into(),
                credential_id_hex: "aabb".into(),
                public_key_der_hex: "ccdd".into(),
                public_key_pem: "pem".into(),
                shards: vec!["eeff00".into(), "aabb11".into()],
                registered_at: "2026-03-05".into(),
            }],
            ..Default::default()
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.total_shares, 5);
        assert_eq!(loaded.keys.len(), 1);
        assert_eq!(loaded.keys[0].label, "test-key");
        assert_eq!(loaded.keys[0].shards.len(), 2);
        assert_eq!(loaded.keys[0].shards[0], "eeff00");
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let config = load_config(&path).unwrap();
        assert!(config.keys.is_empty());
        assert_eq!(config.total_shares, 0);
    }

    #[test]
    fn malformed_config_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");
        std::fs::write(&path, "{not json").unwrap();

        assert!(matches!(load_config(&path), Err(Fido2Error::Config(_))));
    }

    #[test]
    fn legacy_config_defaults_generation_and_receipt() {
        let config: Fido2Config = serde_json::from_str(r#"{"total_shares":0,"keys":[]}"#).unwrap();

        assert_eq!(config.generation, 0);
        assert!(config.last_mutation.is_none());
    }

    #[test]
    fn mutation_receipt_binds_generation_count_and_state_fingerprint() {
        let mut config = Fido2Config {
            total_shares: 1,
            keys: vec![RegisteredKey {
                label: "key".into(),
                credential_id_hex: "aabb".into(),
                public_key_der_hex: "ccdd".into(),
                public_key_pem: "pem".into(),
                shards: vec!["00".into()],
                registered_at: "2026-03-05".into(),
            }],
            ..Default::default()
        };

        config
            .record_mutation("op-1", "fido2.register", Some("key".into()))
            .unwrap();
        assert_eq!(config.generation, 1);
        assert!(
            config
                .mutation_receipt_matches("op-1", "fido2.register", Some("key"))
                .unwrap()
        );

        config.total_shares = 2;
        assert!(
            !config
                .mutation_receipt_matches("op-1", "fido2.register", Some("key"))
                .unwrap()
        );
    }

    #[test]
    fn validate_duplicate_thresholds() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "a".into(),
                threshold: 2,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 1,
                label: "b".into(),
                threshold: 2,
                passphrase_mode: None,
            },
        ];
        assert!(validate_thresholds(&metas).is_err());
    }

    #[test]
    fn validate_unique_thresholds() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "a".into(),
                threshold: 1,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 1,
                label: "b".into(),
                threshold: 2,
                passphrase_mode: None,
            },
        ];
        assert!(validate_thresholds(&metas).is_ok());
    }

    #[test]
    fn next_compartment_id_works() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "a".into(),
                threshold: 1,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 2,
                label: "b".into(),
                threshold: 2,
                passphrase_mode: None,
            },
        ];
        assert_eq!(next_compartment_id(&metas), 3);
        assert_eq!(next_compartment_id(&[]), 0);
    }

    #[test]
    fn dummy_shards_correct_length() {
        let dummies = generate_dummy_shards(5, 61);
        assert_eq!(dummies.len(), 5);
        for d in &dummies {
            assert_eq!(d.len(), 122); // 61 bytes * 2 hex chars
        }
    }

    #[test]
    fn compartment_meta_roundtrip() {
        let meta = CompartmentMeta {
            id: 3,
            label: "legacy".into(),
            threshold: 3,
            passphrase_mode: Some("wrapped".into()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let decoded: CompartmentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, decoded);
    }
}
