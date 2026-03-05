use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault is locked")]
    Locked,

    #[error("key not found: {0}")]
    NotFound(String),

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("fido2: {0}")]
    Fido2(String),

    #[error("vault not initialized")]
    NotInitialized,

    #[error("quorum not met: need {required}, have {provided}")]
    QuorumNotMet { required: usize, provided: usize },

    #[error("compartment not found: {id}")]
    CompartmentNotFound { id: usize },

    #[error("{0}")]
    Other(String),
}
