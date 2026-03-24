//! FIDO2 configuration persistence and compartment metadata types.
//!
//! This module manages the persistent state of FIDO2-secured vaults. The key design principle
//! is **deniability**: `Fido2Config` stored in `fido2_keys.json` contains only registered keys
//! and total share counts—never compartment definitions. Compartment metadata is discovered at
//! unlock time by decrypting tagged shards with the derived hmac-secret.
//!
//! ## Configuration Structure
//!
//! - **`fido2_keys.json`**: Plaintext JSON containing:
//!   - `total_shares`: Total number of shards across all hardware keys.
//!   - `keys`: Array of registered hardware key metadata (label, credential ID, public key, etc.)
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
/// Contains only registered FIDO2 hardware key metadata and the total share count.
/// Compartment definitions themselves are never stored here—they are discovered at
/// unlock time by attempting to decrypt tagged shards with each derived hmac-secret.
/// This design provides deniability: an observer cannot determine which hardware keys
/// correspond to which compartments without the hardware devices.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Fido2Config {
    /// Total number of shards distributed across all registered keys.
    pub total_shares: usize,
    /// Array of registered FIDO2 hardware keys.
    pub keys: Vec<RegisteredKey>,
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
