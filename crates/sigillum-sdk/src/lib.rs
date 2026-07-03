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

#[cfg(test)]
mod tests {
    #[test]
    fn prelude_client_and_core_reexports_are_available() {
        let client =
            crate::prelude::SigillumClient::new("http://127.0.0.1:3200").expect("client builds");
        let config = crate::VaultConfig::default();

        assert_eq!(client.session_token(), None);
        assert_eq!(config.tier1_file, "api_keys.json");
        assert_eq!(config.tier2_file, "vault.enc");
    }
}
