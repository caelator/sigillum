//! Persistent storage for wallet inventory and discovery findings.
//!
//! Discovery is intentionally separate from deposit tracking: deposits are
//! operator-created monitors, while inventory records what Sigillum found while
//! scanning imported wallet profiles and configured chains.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::{
    ChainProfile, ConsolidationPlan, RiskFinding, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress,
};

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WalletInventoryState {
    #[serde(default)]
    pub chain_profiles: Vec<ChainProfile>,
    #[serde(default)]
    pub jobs: Vec<WalletDiscoveryJob>,
    #[serde(default)]
    pub addresses: Vec<WalletInventoryAddress>,
    #[serde(default)]
    pub holdings: Vec<WalletAssetHolding>,
    #[serde(default)]
    pub risk_findings: Vec<RiskFinding>,
    #[serde(default)]
    pub consolidation_plans: Vec<ConsolidationPlan>,
}

impl JsonDocument for WalletInventoryState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.wallet-inventory", 2);

    fn from_enveloped_json(
        path: &std::path::Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        match version {
            1 | 2 => serde_json::from_value(data).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "failed to parse sigillum.wallet-inventory schema payload {}: {error}",
                        path.display()
                    ),
                )
            }),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported sigillum.wallet-inventory schema version {} in {}; expected {}",
                    version,
                    path.display(),
                    Self::SCHEMA.version
                ),
            )),
        }
    }
}

pub fn load_wallet_inventory(
    base_dir: &std::path::Path,
) -> Result<WalletInventoryState, std::io::Error> {
    let path = wallet_inventory_path(base_dir);
    Ok(crate::json_store::load_json_document(&path)?.unwrap_or_default())
}

pub fn save_wallet_inventory(
    base_dir: &std::path::Path,
    state: &WalletInventoryState,
) -> Result<(), std::io::Error> {
    let path = wallet_inventory_path(base_dir);
    crate::json_store::save_json_document(&path, state)
}

pub fn wallet_inventory_path(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("wallet_inventory.json")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn sample_job() -> WalletDiscoveryJob {
        WalletDiscoveryJob {
            id: "job_1".into(),
            status: "completed".into(),
            source: "local-rpc".into(),
            wallet_families: vec!["eth-seed".into()],
            wallet_profiles: vec!["seed-main".into()],
            provider_profiles: vec!["mainnet".into()],
            gap_limit: 20,
            max_index: 100,
            addresses_scanned: 1,
            active_addresses: 1,
            holdings_detected: 1,
            started_at_unix: 1,
            completed_at_unix: Some(2),
            last_error: None,
        }
    }

    fn sample_chain_profile() -> ChainProfile {
        ChainProfile {
            name: "ethereum".into(),
            chain_family: "evm".into(),
            chain_id: Some(1),
            provider_profile: Some("mainnet".into()),
            native_symbol: "ETH".into(),
            explorer_url: None,
            capabilities: vec!["native".into(), "erc20".into()],
            enabled: true,
            source: "operator".into(),
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    fn sample_address() -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: "addr_1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: "0x1".into(),
            transaction_count: 0,
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_holding() -> WalletAssetHolding {
        WalletAssetHolding {
            id: "holding_1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: "native".into(),
            asset_address: None,
            token_id_hex: None,
            counterparty_address: None,
            amount_hex: "0x1".into(),
            source: "local-rpc".into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    #[test]
    fn load_returns_default_when_no_file() {
        let dir = TempDir::new().unwrap();
        let state = load_wallet_inventory(dir.path()).unwrap();
        assert!(state.jobs.is_empty());
        assert!(state.addresses.is_empty());
        assert!(state.holdings.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut state = WalletInventoryState::default();
        state.chain_profiles.push(sample_chain_profile());
        state.jobs.push(sample_job());
        state.addresses.push(sample_address());
        state.holdings.push(sample_holding());

        save_wallet_inventory(dir.path(), &state).unwrap();
        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(loaded.jobs.len(), 1);
        assert_eq!(loaded.chain_profiles.len(), 1);
        assert_eq!(loaded.addresses.len(), 1);
        assert_eq!(loaded.holdings.len(), 1);
        assert_eq!(loaded.addresses[0].wallet_profile, "seed-main");
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        save_wallet_inventory(dir.path(), &WalletInventoryState::default()).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(wallet_inventory_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(saved["schema"], json!("sigillum.wallet-inventory"));
        assert_eq!(saved["schema_version"], json!(2));
        assert!(saved["data"]["chain_profiles"].is_array());
        assert!(saved["data"]["jobs"].is_array());
        assert!(saved["data"]["addresses"].is_array());
        assert!(saved["data"]["holdings"].is_array());
    }
}
