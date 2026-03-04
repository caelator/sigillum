//! # Sigillum
//!
//! Secure secret management with hardware-backed encryption.
//!
//! Sigillum provides a two-tier secret store with AES-256-GCM encryption,
//! optional FIDO2 hardware key unlock, and a daemon mode with web UI.

pub use sigillum_core::*;
