//! # Sigillum Client
//!
//! SDK for connecting to a running Sigillum daemon.
//! Implements `SecretStore` over HTTP so consumers are
//! transport-agnostic.

pub use sigillum_core::{SecretStore, VaultError};
