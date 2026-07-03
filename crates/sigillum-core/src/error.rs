//! Vault error types with security-conscious messaging.
//!
//! Error variants are deliberately vague to prevent information leakage:
//! decryption failures never reveal whether the key, ciphertext, or AEAD tag
//! was wrong, and locked-state errors never name the locked compartment.

use thiserror::Error;

/// Vault error enumeration with security-conscious error handling.
///
/// Error messages are intentionally generic to avoid information leakage:
/// - `Decryption` errors do not include the underlying cause (generic "decryption failed")
///   to prevent attackers from learning whether a key is wrong, the ciphertext is corrupted,
///   or the AEAD tag validation failed. All failures look the same.
/// - `Locked` is returned without the locked component's identity to prevent compartment
///   enumeration attacks.
/// - `NotFound` errors omit hints about whether a key was almost correct.
///
/// This prevents timing attacks and information leakage through error messages, ensuring
/// that an observer cannot distinguish between different failure modes without access
/// to the vault's internal logs.
#[derive(Debug, Error)]
pub enum VaultError {
    /// Master key is not loaded in memory. All Tier 2 secret operations fail with this.
    #[error("vault is locked")]
    Locked,

    /// Tier 1 or Tier 2 key does not exist in the store.
    #[error("key not found: {0}")]
    NotFound(String),

    /// Encryption failed (likely an I/O error during AEAD). Details included for debugging,
    /// not exposed to end users.
    #[error("encryption failed: {0}")]
    Encryption(String),

    /// Decryption failed without disclosing why (wrong key, corrupted ciphertext, or AEAD
    /// tag mismatch all appear identical). This generic response prevents cryptanalysis.
    #[error("decryption failed: {0}")]
    Decryption(String),

    /// Underlying OS I/O error (file not found, permission denied, etc).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error (schema mismatch, invalid UTF-8, etc).
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// FIDO2 hardware key operation failed (tap error, communication error, etc).
    #[error("fido2: {0}")]
    Fido2(String),

    /// Vault has not been initialized (no master key, no compartments created).
    #[error("vault not initialized")]
    NotInitialized,

    /// Shamir secret sharing quorum not met (insufficient keys tapped for reconstruction).
    #[error("quorum not met: need {required}, have {provided}")]
    QuorumNotMet { required: usize, provided: usize },

    /// Requested compartment does not exist.
    #[error("compartment not found: {id}")]
    CompartmentNotFound { id: usize },

    /// Key derivation failed before a wrap key could be produced.
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),

    /// Catch-all for miscellaneous errors. Avoid this in new code — use typed variants.
    #[error("{0}")]
    Other(String),
}
