//! Error types for FIDO2 hardware key operations.
//!
//! Covers device communication, credential management, Shamir reconstruction,
//! and configuration persistence failures.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Fido2Error {
    #[error("no FIDO2 device found")]
    NoDevice,

    #[error(
        "multiple FIDO2 devices are attached ({count}). Leave only the target hardware key inserted for this step, then retry."
    )]
    MultipleDevicesDetected { count: usize },

    #[error("timeout: {operation} did not complete within {timeout_secs}s")]
    Timeout {
        operation: String,
        timeout_secs: u64,
    },

    #[error("attestation verification failed")]
    AttestationFailed,

    #[error("device did not return hmac-secret")]
    NoHmacSecret,

    #[error("no attached hardware key matched the expected Sigillum credential")]
    NoMatchingCredential,

    #[error(
        "all attached hardware keys already appear to be registered. Insert the new hardware key you want to add, then retry."
    )]
    NoNewDeviceDetected,

    #[error("incorrect PIN")]
    IncorrectPin,

    #[error(
        "the hardware key requires its current FIDO2 PIN for this operation. Enter the existing PIN and retry."
    )]
    PinRequired,

    #[error(
        "no FIDO2 PIN is configured on the hardware key. Set a PIN on the key first, then retry registration."
    )]
    PinNotSet,

    #[error("a FIDO2 PIN is already configured on the hardware key")]
    PinAlreadySet,

    #[error(
        "PIN authentication is temporarily blocked on the hardware key. Unplug and reinsert it, then retry with the correct PIN."
    )]
    PinAuthBlocked,

    #[error(
        "the hardware key PIN is fully blocked. Reset or recover the key with vendor tooling before trying again."
    )]
    PinBlocked,

    #[error("key already registered: {label}")]
    DuplicateKey { label: String },

    #[error("key not found: {label}")]
    KeyNotFound { label: String },

    #[error("no keys registered")]
    NoKeysRegistered,

    #[error("quorum not met: need {required}, have {available}")]
    QuorumNotMet { required: usize, available: usize },

    #[error("removal would drop below quorum: {remaining} keys < threshold {threshold}")]
    RemovalBelowQuorum { remaining: usize, threshold: usize },

    #[error("shamir reconstruction failed: {0}")]
    ShamirFailed(String),

    #[error("shard encryption failed: {0}")]
    ShardEncryption(String),

    #[error("shard decryption failed: {0}")]
    ShardDecryption(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("another FIDO2 config writer holds {path}")]
    WriterBusy { path: String },

    #[error("CTAP1 device — requires CTAP2 with hmac-secret extension")]
    Ctap1Device,

    #[error("duplicate threshold: {threshold} already assigned to another compartment")]
    DuplicateThreshold { threshold: usize },

    #[error("no compartment configured for threshold {threshold}")]
    NoCompartmentForThreshold { threshold: usize },

    #[error("compartment not found: {id}")]
    CompartmentNotFound { id: usize },

    #[error("{0}")]
    Other(String),
}
