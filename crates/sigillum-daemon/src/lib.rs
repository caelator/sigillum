//! # Sigillum Daemon
//!
//! HTTP server and web UI for managing a Sigillum vault.
//! Holds the master key in memory so clients never touch
//! key material directly.

pub use sigillum_core::{SecretStore, VaultError, VaultLifecycle};
