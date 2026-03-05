use std::path::PathBuf;

use sigillum_core::FileVault;

/// Shared daemon state. The `FileVault` lives here so the master key
/// persists across HTTP requests.
pub struct AppState {
    pub vault: FileVault,
    salt_path: PathBuf,
}

impl AppState {
    pub fn new(vault: FileVault) -> Self {
        let salt_path = vault.config_base_dir().join("passphrase.salt");
        Self { vault, salt_path }
    }

    pub fn salt_path(&self) -> &PathBuf {
        &self.salt_path
    }
}
