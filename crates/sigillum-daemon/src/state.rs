use std::path::PathBuf;

use sigillum_core::FileVault;
use sigillum_fido2::Fido2Manager;

/// Shared daemon state. The `FileVault` lives here so the master key
/// persists across HTTP requests.
pub struct AppState {
    pub vault: FileVault,
    pub fido2: Fido2Manager,
    salt_path: PathBuf,
    wrapped_key_path: PathBuf,
}

impl AppState {
    pub fn new(vault: FileVault) -> Self {
        let base = vault.config_base_dir().to_path_buf();
        let salt_path = base.join("passphrase.salt");
        let wrapped_key_path = base.join("passphrase_wrapped_key.enc");
        let fido2_config_path = base.join("fido2_keys.json");
        let fido2 = Fido2Manager::new(fido2_config_path);
        Self {
            vault,
            fido2,
            salt_path,
            wrapped_key_path,
        }
    }

    pub fn salt_path(&self) -> &PathBuf {
        &self.salt_path
    }

    pub fn wrapped_key_path(&self) -> &PathBuf {
        &self.wrapped_key_path
    }
}
