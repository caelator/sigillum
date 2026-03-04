use std::path::PathBuf;

use crate::{SecretStore, VaultError, VaultLifecycle};
use secrecy::SecretString;

/// Configuration for the file-backed vault.
pub struct VaultConfig {
    /// Base directory for vault files (e.g., `~/.sigillum`).
    pub base_dir: PathBuf,
    /// Filename for Tier 1 (plaintext) keys.
    pub tier1_file: String,
    /// Filename for Tier 2 (encrypted) secrets.
    pub tier2_file: String,
}

impl Default for VaultConfig {
    fn default() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".sigillum");
        Self {
            base_dir: base,
            tier1_file: "api_keys.json".into(),
            tier2_file: "vault.enc".into(),
        }
    }
}

/// File-backed vault with AES-256-GCM encryption.
pub struct FileVault {
    _config: VaultConfig,
}

impl FileVault {
    pub fn new(config: VaultConfig) -> Self {
        Self { _config: config }
    }
}

impl SecretStore for FileVault {
    fn get_api_key(&self, _key: &str) -> Option<SecretString> {
        todo!("extract from bri CortexVault")
    }

    fn set_api_key(&self, _key: &str, _value: &str) -> Result<(), VaultError> {
        todo!()
    }

    fn delete_api_key(&self, _key: &str) -> Result<(), VaultError> {
        todo!()
    }

    fn list_api_keys(&self) -> Vec<String> {
        todo!()
    }

    fn get_secret(&self, _key: &str) -> Option<SecretString> {
        todo!()
    }

    fn set_secret(&self, _key: &str, _value: &str) -> Result<(), VaultError> {
        todo!()
    }

    fn delete_secret(&self, _key: &str) -> Result<(), VaultError> {
        todo!()
    }

    fn list_secrets(&self) -> Vec<String> {
        todo!()
    }

    fn has_key(&self, _key: &str) -> bool {
        todo!()
    }

    fn is_unlocked(&self) -> bool {
        todo!()
    }
}

impl VaultLifecycle for FileVault {
    fn load_master_key(&self, _key: [u8; 32]) {
        todo!()
    }

    fn zeroize_master_key(&self) {
        todo!()
    }

    fn initialize(&self, _master_key: &[u8; 32]) -> Result<(), VaultError> {
        todo!()
    }
}
