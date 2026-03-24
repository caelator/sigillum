//! Core vault abstractions: secret storage and lifecycle management.
//!
//! Defines two traits that together form the vault contract:
//! - [`SecretStore`] — read/write access to the two-tier secret model
//! - [`VaultLifecycle`] — lock, unlock, initialize, and master key management
//!
//! Separating these follows the principle of least authority: most callers
//! only need [`SecretStore`] and never touch lifecycle operations.

use secrecy::SecretString;
use zeroize::Zeroizing;

use crate::VaultError;

/// Core secret storage interface.
///
/// Provides two-tier secret management:
/// - Tier 1: API keys stored in plaintext (no unlock required)
/// - Tier 2: Secrets encrypted with AES-256-GCM (unlock required)
///
/// This trait is read-only: callers fetch secrets without managing vault state.
/// State management (lock/unlock/initialize) is separated into `VaultLifecycle`
/// to follow the principle of least authority — most callers don't need access to
/// those privileged operations.
pub trait SecretStore: Send + Sync {
    // — Tier 1 (plaintext, no unlock) —

    fn read_api_key(&self, key: &str) -> Result<Option<SecretString>, VaultError>;
    fn set_api_key(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_api_key(&self, key: &str) -> Result<(), VaultError>;
    fn read_api_keys(&self) -> Result<Vec<String>, VaultError>;

    // — Tier 2 (encrypted, requires unlock) —

    fn read_secret(&self, key: &str) -> Result<Option<SecretString>, VaultError>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), VaultError>;
    fn delete_secret(&self, key: &str) -> Result<(), VaultError>;
    fn read_secrets(&self) -> Result<Vec<String>, VaultError>;

    // — Common —

    fn contains_key(&self, key: &str) -> Result<bool, VaultError>;
    fn is_unlocked(&self) -> bool;

    /// Backward-compatible convenience shim.
    fn get_api_key(&self, key: &str) -> Option<SecretString> {
        self.read_api_key(key).ok().flatten()
    }

    /// Backward-compatible convenience shim.
    fn list_api_keys(&self) -> Vec<String> {
        self.read_api_keys().unwrap_or_default()
    }

    /// Backward-compatible convenience shim.
    fn get_secret(&self, key: &str) -> Option<SecretString> {
        self.read_secret(key).ok().flatten()
    }

    /// Backward-compatible convenience shim.
    fn list_secrets(&self) -> Vec<String> {
        self.read_secrets().unwrap_or_default()
    }

    /// Backward-compatible convenience shim.
    fn has_key(&self, key: &str) -> bool {
        self.contains_key(key).unwrap_or(false)
    }
}

/// Vault lifecycle management (unlock, lock, initialize).
///
/// Separated from `SecretStore` following the principle of least authority:
/// Most consumers only need read/write access via `SecretStore`. Only privileged
/// components (daemon unlock handlers, FIDO2 manager, CLI) need lifecycle control.
/// This separation prevents accidental lock/unlock operations or master key extraction
/// in code paths that don't require it.
///
/// The lifecycle methods are intentionally not exposed through the generic
/// `SecretStore` trait — they must be explicitly requested via `VaultLifecycle`.
pub trait VaultLifecycle: SecretStore {
    fn load_master_key(&self, key: [u8; 32]);
    fn zeroize_master_key(&self);
    fn initialize(&self, master_key: &[u8; 32]) -> Result<(), VaultError>;

    /// Extract a copy of the master key (for FIDO2 re-splitting on Nth key registration).
    /// Returns `None` if locked.
    fn extract_master_key(&self) -> Option<Zeroizing<[u8; 32]>>;
}
