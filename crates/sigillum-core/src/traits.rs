use secrecy::SecretString;

use crate::VaultError;

/// Core secret storage interface.
///
/// Provides two-tier secret management:
/// - Tier 1: API keys stored in plaintext (no unlock required)
/// - Tier 2: Secrets encrypted with AES-256-GCM (unlock required)
pub trait SecretStore: Send + Sync {
    // — Tier 1 (plaintext, no unlock) —

    fn get_api_key(&self, key: &str) -> Option<SecretString>;
    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_api_key(&self, key: &str) -> Result<(), VaultError>;
    fn list_api_keys(&self) -> Vec<String>;

    // — Tier 2 (encrypted, requires unlock) —

    fn get_secret(&self, key: &str) -> Option<SecretString>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_secret(&self, key: &str) -> Result<(), VaultError>;
    fn list_secrets(&self) -> Vec<String>;

    // — Common —

    fn has_key(&self, key: &str) -> bool;
    fn is_unlocked(&self) -> bool;
}

/// Vault lifecycle management (unlock, lock, initialize).
///
/// Separated from `SecretStore` because most consumers only need
/// read/write access — only the unlock manager (CLI, daemon, FIDO2)
/// needs lifecycle control.
pub trait VaultLifecycle: SecretStore {
    fn load_master_key(&self, key: [u8; 32]);
    fn zeroize_master_key(&self);
    fn initialize(&self, master_key: &[u8; 32]) -> Result<(), VaultError>;
}
