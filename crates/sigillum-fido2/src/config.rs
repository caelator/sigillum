use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Fido2Error;

/// A single compartment definition: threshold determines which credential
/// combination unlocks this compartment.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CompartmentDef {
    pub id: usize,
    pub label: String,
    /// Number of FIDO2 key taps required to unlock this compartment.
    pub threshold: usize,
    /// "direct" | "wrapped" | null — only relevant when passphrase is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase_mode: Option<String>,
}

/// Persisted FIDO2 configuration: compartments, registered keys, per-compartment shards.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Fido2Config {
    pub compartments: Vec<CompartmentDef>,
    pub total_shares: usize,
    pub keys: Vec<RegisteredKey>,
}

/// A single registered FIDO2 hardware key.
/// Each key holds encrypted shards for ALL compartments.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisteredKey {
    pub label: String,
    pub credential_id_hex: String,
    pub public_key_der_hex: String,
    pub public_key_pem: String,
    /// compartment_id (as string key) → hex-encoded `nonce(12) || AES-256-GCM(hmac_secret, shard)`
    pub shards: HashMap<String, String>,
    pub registered_at: String,
}

impl Default for Fido2Config {
    fn default() -> Self {
        Self {
            compartments: Vec::new(),
            total_shares: 0,
            keys: Vec::new(),
        }
    }
}

impl Fido2Config {
    pub fn is_fido2_enabled(&self) -> bool {
        !self.keys.is_empty() && !self.compartments.is_empty()
    }

    /// Find compartment by threshold value.
    pub fn resolve_compartment(&self, threshold: usize) -> Option<&CompartmentDef> {
        self.compartments.iter().find(|c| c.threshold == threshold)
    }

    /// Find compartment by id.
    pub fn compartment_by_id(&self, id: usize) -> Option<&CompartmentDef> {
        self.compartments.iter().find(|c| c.id == id)
    }

    /// Validate that all thresholds are unique.
    pub fn validate_thresholds(&self) -> Result<(), Fido2Error> {
        let mut seen = std::collections::HashSet::new();
        for c in &self.compartments {
            if !seen.insert(c.threshold) {
                return Err(Fido2Error::DuplicateThreshold {
                    threshold: c.threshold,
                });
            }
        }
        Ok(())
    }

    /// Next available compartment id.
    pub fn next_compartment_id(&self) -> usize {
        self.compartments.iter().map(|c| c.id).max().map_or(0, |m| m + 1)
    }
}

/// Load config from disk, returning default if file doesn't exist.
pub fn load_config(path: &Path) -> Fido2Config {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Fido2Config::default(),
    }
}

/// Save config to disk with restrictive permissions.
pub fn save_config(path: &Path, config: &Fido2Config) -> Result<(), Fido2Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Fido2Error::Config(format!("create dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| Fido2Error::Config(format!("serialize: {e}")))?;
    std::fs::write(path, &json).map_err(|e| Fido2Error::Config(format!("write: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");

        let mut shards = HashMap::new();
        shards.insert("0".to_string(), "eeff00".to_string());
        shards.insert("1".to_string(), "aabb11".to_string());

        let config = Fido2Config {
            compartments: vec![
                CompartmentDef {
                    id: 0,
                    label: "hot".into(),
                    threshold: 1,
                    passphrase_mode: Some("wrapped".into()),
                },
                CompartmentDef {
                    id: 1,
                    label: "cold".into(),
                    threshold: 2,
                    passphrase_mode: None,
                },
            ],
            total_shares: 5,
            keys: vec![RegisteredKey {
                label: "test-key".into(),
                credential_id_hex: "aabb".into(),
                public_key_der_hex: "ccdd".into(),
                public_key_pem: "pem".into(),
                shards,
                registered_at: "2026-03-05".into(),
            }],
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path);

        assert_eq!(loaded.compartments.len(), 2);
        assert_eq!(loaded.compartments[0].label, "hot");
        assert_eq!(loaded.compartments[0].threshold, 1);
        assert_eq!(loaded.compartments[1].label, "cold");
        assert_eq!(loaded.compartments[1].threshold, 2);
        assert_eq!(loaded.total_shares, 5);
        assert_eq!(loaded.keys.len(), 1);
        assert_eq!(loaded.keys[0].label, "test-key");
        assert_eq!(loaded.keys[0].shards.len(), 2);
        assert_eq!(loaded.keys[0].shards["0"], "eeff00");
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let config = load_config(&path);
        assert!(config.compartments.is_empty());
        assert!(config.keys.is_empty());
        assert_eq!(config.total_shares, 0);
    }

    #[test]
    fn resolve_compartment_by_threshold() {
        let config = Fido2Config {
            compartments: vec![
                CompartmentDef { id: 0, label: "hot".into(), threshold: 1, passphrase_mode: None },
                CompartmentDef { id: 1, label: "cold".into(), threshold: 2, passphrase_mode: None },
                CompartmentDef { id: 2, label: "legacy".into(), threshold: 3, passphrase_mode: None },
            ],
            total_shares: 5,
            keys: Vec::new(),
        };

        assert_eq!(config.resolve_compartment(1).unwrap().label, "hot");
        assert_eq!(config.resolve_compartment(2).unwrap().label, "cold");
        assert_eq!(config.resolve_compartment(3).unwrap().label, "legacy");
        assert!(config.resolve_compartment(4).is_none());
    }

    #[test]
    fn validate_duplicate_thresholds() {
        let config = Fido2Config {
            compartments: vec![
                CompartmentDef { id: 0, label: "a".into(), threshold: 2, passphrase_mode: None },
                CompartmentDef { id: 1, label: "b".into(), threshold: 2, passphrase_mode: None },
            ],
            total_shares: 3,
            keys: Vec::new(),
        };
        assert!(config.validate_thresholds().is_err());
    }

    #[test]
    fn next_compartment_id() {
        let config = Fido2Config {
            compartments: vec![
                CompartmentDef { id: 0, label: "a".into(), threshold: 1, passphrase_mode: None },
                CompartmentDef { id: 2, label: "b".into(), threshold: 2, passphrase_mode: None },
            ],
            total_shares: 3,
            keys: Vec::new(),
        };
        assert_eq!(config.next_compartment_id(), 3);

        let empty = Fido2Config::default();
        assert_eq!(empty.next_compartment_id(), 0);
    }
}
