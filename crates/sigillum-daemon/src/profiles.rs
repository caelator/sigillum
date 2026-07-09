//! Persistent storage for operational profiles (EVM providers and stealth wallets).
//!
//! Profiles bind reusable configuration — RPC endpoints, fee policies, wallet
//! labels — to named entries so deposit, queue, and maintenance operations can
//! reference them without re-specifying connection details each time.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::{
    EthSeedWalletProfile, EthStealthWalletProfile, EthXpubWalletProfile, EvmProviderProfile,
};

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProfileRegistry {
    #[serde(default)]
    pub evm_providers: Vec<EvmProviderProfile>,
    #[serde(default)]
    pub eth_stealth_wallets: Vec<EthStealthWalletProfile>,
    #[serde(default)]
    pub eth_xpub_wallets: Vec<EthXpubWalletProfile>,
    #[serde(default)]
    pub eth_seed_wallets: Vec<EthSeedWalletProfile>,
}

impl JsonDocument for ProfileRegistry {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.profiles", 1);
}

pub fn load_profiles(base_dir: &std::path::Path) -> Result<ProfileRegistry, std::io::Error> {
    let path = profiles_path(base_dir);
    Ok(crate::json_store::load_json_document(&path)?.unwrap_or_default())
}

pub fn save_profiles(
    base_dir: &std::path::Path,
    registry: &ProfileRegistry,
) -> Result<(), std::io::Error> {
    let path = profiles_path(base_dir);
    crate::json_store::save_json_document(&path, registry)
}

pub fn profiles_path(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("profiles.json")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = TempDir::new().unwrap();
        let registry = load_profiles(dir.path()).unwrap();
        assert!(registry.evm_providers.is_empty());
        assert!(registry.eth_stealth_wallets.is_empty());
        assert!(registry.eth_xpub_wallets.is_empty());
        assert!(registry.eth_seed_wallets.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();

        let mut registry = ProfileRegistry::default();
        registry.evm_providers.push(EvmProviderProfile {
            name: "mainnet".into(),
            rpc_url: "https://eth.example.com".into(),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
            fee_estimation_enabled: false,
        });

        save_profiles(dir.path(), &registry).unwrap();
        let loaded = load_profiles(dir.path()).unwrap();

        assert_eq!(loaded.evm_providers.len(), 1);
        assert_eq!(loaded.evm_providers[0].name, "mainnet");
        assert_eq!(loaded.evm_providers[0].rpc_url, "https://eth.example.com");
        assert_eq!(loaded.evm_providers[0].chain_id, 1);
        assert!(loaded.eth_stealth_wallets.is_empty());
        assert!(loaded.eth_xpub_wallets.is_empty());
        assert!(loaded.eth_seed_wallets.is_empty());
    }

    #[test]
    fn corrupted_json_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = profiles_path(dir.path());
        std::fs::write(&path, b"not valid json {{{").unwrap();

        let result = load_profiles(dir.path());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        let registry = ProfileRegistry::default();
        save_profiles(&nested, &registry).unwrap();
        assert!(profiles_path(&nested).exists());
    }

    #[test]
    fn restores_from_backup_when_live_file_is_missing() {
        let dir = TempDir::new().unwrap();

        let mut registry = ProfileRegistry::default();
        registry.evm_providers.push(EvmProviderProfile {
            name: "mainnet".into(),
            rpc_url: "https://eth.example.com".into(),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
            fee_estimation_enabled: false,
        });

        save_profiles(dir.path(), &registry).unwrap();
        std::fs::remove_file(profiles_path(dir.path())).unwrap();

        let loaded = load_profiles(dir.path()).unwrap();
        assert_eq!(loaded.evm_providers.len(), 1);
        assert!(loaded.eth_xpub_wallets.is_empty());
        assert!(loaded.eth_seed_wallets.is_empty());
        assert!(profiles_path(dir.path()).exists());
    }

    #[test]
    fn corrupt_live_file_is_quarantined_and_restored_from_backup() {
        let dir = TempDir::new().unwrap();

        let mut registry = ProfileRegistry::default();
        registry.eth_stealth_wallets.push(EthStealthWalletProfile {
            name: "treasury".into(),
            wallet: "wallet-a".into(),
            short_name: "eth".into(),
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            default_destination_address: None,
            execution_enabled: false,
        });
        registry.eth_xpub_wallets.push(EthXpubWalletProfile {
            name: "project-main".into(),
            project_account: 7,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            external_receive_xpub: None,
            external_receive_path: None,
            external_account_xpub: None,
            external_account_path: None,
            default_destination_address: None,
            execution_enabled: false,
        });
        registry.eth_seed_wallets.push(EthSeedWalletProfile {
            name: "seed-main".into(),
            label: Some("Seed main".into()),
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            word_count: 12,
            mnemonic_secret_key: "wallet.seed.seed-main.mnemonic".into(),
            account_path: "m/44'/60'/0'".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "xpub661MyMwAqRbcFexample".into(),
            first_receive_address: "0x1111111111111111111111111111111111111111".into(),
            default_destination_address: None,
            control_xpub: None,
            sponsor_address: None,
            hot_address: None,
            treasury_address: None,
            execution_enabled: false,
        });

        save_profiles(dir.path(), &registry).unwrap();
        std::fs::write(profiles_path(dir.path()), b"broken").unwrap();

        let loaded = load_profiles(dir.path()).unwrap();
        assert_eq!(loaded.eth_stealth_wallets.len(), 1);
        assert_eq!(loaded.eth_xpub_wallets.len(), 1);
        assert_eq!(loaded.eth_seed_wallets.len(), 1);

        let corrupt_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("profiles.json.corrupt-")
            })
            .count();
        assert_eq!(corrupt_files, 1);
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let registry = ProfileRegistry::default();

        save_profiles(dir.path(), &registry).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profiles_path(dir.path())).unwrap()).unwrap();
        assert_eq!(saved["schema"], json!("sigillum.profiles"));
        assert_eq!(saved["schema_version"], json!(1));
        assert!(saved["data"]["evm_providers"].is_array());
        assert!(saved["data"]["eth_stealth_wallets"].is_array());
        assert!(saved["data"]["eth_xpub_wallets"].is_array());
        assert!(saved["data"]["eth_seed_wallets"].is_array());
    }

    #[test]
    fn legacy_unwrapped_profiles_still_load() {
        let dir = TempDir::new().unwrap();
        let path = profiles_path(dir.path());
        let registry = ProfileRegistry::default();
        std::fs::write(&path, serde_json::to_vec_pretty(&registry).unwrap()).unwrap();

        let loaded = load_profiles(dir.path()).unwrap();
        assert!(loaded.evm_providers.is_empty());
        assert!(loaded.eth_stealth_wallets.is_empty());
        assert!(loaded.eth_xpub_wallets.is_empty());
        assert!(loaded.eth_seed_wallets.is_empty());
    }
}
