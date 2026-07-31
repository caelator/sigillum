//! # Sigillum API
//!
//! Shared request/response types for the Sigillum daemon API.
//!
//! This crate defines the contract between client and server: all request/response types
//! must implement `PartialEq` and `Eq` to enable deterministic error handling and testing,
//! and `Serialize`/`Deserialize` to support JSON over HTTP.
//!
//! Why PartialEq/Eq matters: These traits enable the server to compare canonicalized
//! payloads for idempotency (preventing duplicate operations) and allow clients to assert
//! expected responses in tests. This is critical for a secret management system where
//! side effects must be traceable and reproducible.
//!
//! Serde contract: All types must round-trip through serde_json without loss of precision.
//! No custom serialization or ad-hoc validation — all validation is explicit and typed
//! in the `validation` module.

pub mod error_codes;
pub mod request;
pub mod response;
pub mod route_paths;
pub mod validation;

pub use request::*;
pub use response::*;
pub use validation::{Validate, ValidationFailure};
