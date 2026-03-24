//! # Sigillum SDK
//!
//! Local-first integration surface that combines the core file-backed types
//! with the async daemon client.

pub use sigillum_api::*;
pub use sigillum_client::{ClientError, SigillumClient};
pub use sigillum_core::*;

pub mod prelude {
    pub use sigillum_client::SigillumClient;
    pub use sigillum_core::{SecretStore, VaultLifecycle};
}
