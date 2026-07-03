//! # Sigillum
//!
//! Secure secret management with hardware-backed encryption.
//!
//! Sigillum provides a two-tier secret store with AES-256-GCM encryption,
//! optional FIDO2 hardware key unlock, and a daemon mode with web UI.

pub use sigillum_core::*;

#[cfg(test)]
mod tests {
    #[test]
    fn file_vault_starts_without_existing_vault() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir should be created");
        let config = crate::VaultConfig {
            base_dir: temp_dir.path().to_path_buf(),
            tier1_file: "api_keys.json".into(),
            tier2_file: "vault.enc".into(),
        };

        let vault = crate::FileVault::new(config);

        assert!(!vault.vault_exists());
        assert_eq!(vault.config_base_dir(), temp_dir.path());
    }
}
