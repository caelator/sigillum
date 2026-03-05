use thiserror::Error;

#[derive(Debug, Error)]
pub enum Fido2Error {
    #[error("no FIDO2 device found")]
    NoDevice,

    #[error("timeout: {operation} did not complete within {timeout_secs}s")]
    Timeout {
        operation: String,
        timeout_secs: u64,
    },

    #[error("attestation verification failed")]
    AttestationFailed,

    #[error("device did not return hmac-secret")]
    NoHmacSecret,

    #[error("incorrect PIN")]
    IncorrectPin,

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
