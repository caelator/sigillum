//! # Sigillum Core
//!
//! Traits and file-backed implementation for secure secret management.

mod error;
mod traits;

pub use error::VaultError;
pub use traits::{SecretStore, VaultLifecycle};

#[cfg(feature = "file-backend")]
mod file_vault;
#[cfg(feature = "file-backend")]
pub use file_vault::{FileVault, VaultConfig};
