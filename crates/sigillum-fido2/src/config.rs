use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Fido2Error;

const DEFAULT_QUORUM: usize = 1;

/// Persisted FIDO2 configuration: registered keys + quorum settings.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Fido2Config {
    pub quorum_threshold: usize,
    /// "passphrase" | "fido2" | "both"
    pub unlock_method: String,
    /// "direct" (Argon2id output IS the master key) or "wrapped" (Argon2id wraps a random key).
    /// Only relevant when unlock_method includes passphrase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase_mode: Option<String>,
    pub keys: Vec<RegisteredKey>,
}

/// A single registered FIDO2 hardware key.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegisteredKey {
    pub label: String,
    pub credential_id_hex: String,
    pub public_key_der_hex: String,
    pub public_key_pem: String,
    /// Hex-encoded: `nonce(12) || AES-256-GCM(hmac_secret, shard)`
    pub encrypted_shard_hex: String,
    pub registered_at: String,
}

impl Default for Fido2Config {
    fn default() -> Self {
        Self {
            quorum_threshold: DEFAULT_QUORUM,
            unlock_method: "passphrase".into(),
            passphrase_mode: None,
            keys: Vec::new(),
        }
    }
}

impl Fido2Config {
    pub fn is_fido2_enabled(&self) -> bool {
        !self.keys.is_empty()
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

        let config = Fido2Config {
            quorum_threshold: 2,
            unlock_method: "fido2".into(),
            passphrase_mode: None,
            keys: vec![RegisteredKey {
                label: "test-key".into(),
                credential_id_hex: "aabb".into(),
                public_key_der_hex: "ccdd".into(),
                public_key_pem: "pem".into(),
                encrypted_shard_hex: "eeff".into(),
                registered_at: "2026-03-05".into(),
            }],
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path);

        assert_eq!(loaded.quorum_threshold, 2);
        assert_eq!(loaded.unlock_method, "fido2");
        assert_eq!(loaded.keys.len(), 1);
        assert_eq!(loaded.keys[0].label, "test-key");
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let config = load_config(&path);
        assert_eq!(config.quorum_threshold, 1);
        assert!(config.keys.is_empty());
        assert_eq!(config.unlock_method, "passphrase");
    }
}
