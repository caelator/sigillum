//! Persistent storage for wallet inventory and discovery findings.
//!
//! Discovery is intentionally separate from deposit tracking: deposits are
//! operator-created monitors, while inventory records what Sigillum found while
//! scanning imported wallet profiles and configured chains.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sigillum_api::{
    ChainProfile, ConsolidationPlan, Counterparty, NftMetadataCacheEntry, RiskCatalogEntry,
    RiskFinding, TreasuryPolicy, TreasuryReceiveAllocation, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress, WatchAddressBookEntry,
};

use crate::json_store::{JsonDocument, JsonSchema};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WalletInventoryState {
    #[serde(default)]
    pub chain_profiles: Vec<ChainProfile>,
    #[serde(default)]
    pub watch_address_book: Vec<WatchAddressBookEntry>,
    #[serde(default)]
    pub jobs: Vec<WalletDiscoveryJob>,
    #[serde(default)]
    pub addresses: Vec<WalletInventoryAddress>,
    #[serde(default)]
    pub holdings: Vec<WalletAssetHolding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nft_metadata_cache: Vec<NftMetadataCacheEntry>,
    #[serde(default)]
    pub risk_catalog: Vec<RiskCatalogEntry>,
    #[serde(default)]
    pub risk_findings: Vec<RiskFinding>,
    #[serde(default)]
    pub consolidation_plans: Vec<ConsolidationPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treasury_policy: Option<TreasuryPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receive_allocations: Vec<TreasuryReceiveAllocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parties: Vec<Counterparty>,
}

impl JsonDocument for WalletInventoryState {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.wallet-inventory", 11);

    fn from_enveloped_json(
        path: &std::path::Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        match version {
            1..=11 => serde_json::from_value(data).map_err(|error| {
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
            checkpoints: Vec::new(),
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
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: "0x1".into(),
            transaction_count: 0,
            classifications: vec![
                "signer_available".into(),
                "gas_available".into(),
                "value_detected".into(),
                "dormant_candidate".into(),
            ],
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_watch_address_book_entry() -> WatchAddressBookEntry {
        WatchAddressBookEntry {
            id: "watch_1".into(),
            address: "0x7777777777777777777777777777777777777777".into(),
            label: "old-ledger".into(),
            tags: vec!["client".into(), "hardware".into()],
            source: "operator".into(),
            enabled: true,
            created_at_unix: 1,
            updated_at_unix: 2,
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
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            metadata_uri: None,
            metadata_name: None,
            spam_label: None,
            amount_hex: "0x1".into(),
            source: "local-rpc".into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_risk_catalog_entry() -> RiskCatalogEntry {
        RiskCatalogEntry {
            address: "0x4444444444444444444444444444444444444444".into(),
            label: "Known router".into(),
            risk_level: "trusted".into(),
            source: "operator".into(),
            notes: vec!["Operator-approved spender".into()],
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    fn sample_receive_allocation() -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: "alloc_1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: "0x2222222222222222222222222222222222222222".into(),
            derivation_path: "m/44'/60'/0'/0/3".into(),
            address_index: 3,
            purpose: "counterparty-acme".into(),
            label: Some("Acme invoices".into()),
            status: "active".into(),
            created_at_unix: 1,
            retired_at_unix: None,
            counterparty_id: None,
        }
    }

    fn sample_treasury_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![sigillum_api::TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".into(),
                label: Some("cold-treasury".into()),
            }],
            max_step_native_wei_hex: Some("0xde0b6b3a7640000".into()),
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            created_at_unix: 1,
            updated_at_unix: 2,
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
        state
            .watch_address_book
            .push(sample_watch_address_book_entry());
        state.jobs.push(sample_job());
        state.addresses.push(sample_address());
        state.holdings.push(sample_holding());
        state.risk_catalog.push(sample_risk_catalog_entry());
        state.receive_allocations.push(sample_receive_allocation());

        save_wallet_inventory(dir.path(), &state).unwrap();
        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(loaded.jobs.len(), 1);
        assert_eq!(loaded.chain_profiles.len(), 1);
        assert_eq!(loaded.watch_address_book.len(), 1);
        assert_eq!(loaded.addresses.len(), 1);
        assert_eq!(loaded.holdings.len(), 1);
        assert_eq!(loaded.risk_catalog.len(), 1);
        assert_eq!(loaded.addresses[0].wallet_profile, "seed-main");
        assert!(loaded.treasury_policy.is_none());
        assert_eq!(loaded.receive_allocations.len(), 1);
        assert_eq!(loaded.receive_allocations[0].address_index, 3);
        assert_eq!(loaded.receive_allocations[0].status, "active");
        assert_eq!(loaded.receive_allocations[0].purpose, "counterparty-acme");
        assert!(loaded.receive_allocations[0].retired_at_unix.is_none());
    }

    #[test]
    fn save_and_load_roundtrip_preserves_treasury_policy() {
        let dir = TempDir::new().unwrap();
        let state = WalletInventoryState {
            treasury_policy: Some(sample_treasury_policy()),
            ..WalletInventoryState::default()
        };

        save_wallet_inventory(dir.path(), &state).unwrap();
        let loaded = load_wallet_inventory(dir.path()).unwrap();

        let policy = loaded.treasury_policy.expect("policy persisted");
        assert!(policy.enabled);
        assert_eq!(policy.allowed_destinations.len(), 1);
        assert_eq!(
            policy.allowed_destinations[0].address,
            "0x9999999999999999999999999999999999999999"
        );
        assert_eq!(
            policy.max_step_native_wei_hex.as_deref(),
            Some("0xde0b6b3a7640000")
        );
        assert!(policy.require_simulation);
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        save_wallet_inventory(dir.path(), &WalletInventoryState::default()).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(wallet_inventory_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(saved["schema"], json!("sigillum.wallet-inventory"));
        assert_eq!(saved["schema_version"], json!(11));
        assert!(saved["data"]["chain_profiles"].is_array());
        assert!(saved["data"]["watch_address_book"].is_array());
        assert!(saved["data"]["jobs"].is_array());
        assert!(saved["data"]["addresses"].is_array());
        assert!(saved["data"]["holdings"].is_array());
        assert!(saved["data"]["risk_catalog"].is_array());
    }

    #[test]
    fn legacy_v10_receive_allocations_load_with_assumed_mainnet_chain_id() {
        let dir = TempDir::new().unwrap();
        let path = wallet_inventory_path(dir.path());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.wallet-inventory",
                "schema_version": 10,
                "data": {
                    "receive_allocations": [{
                        "id": "alloc_1",
                        "wallet_family": "eth-seed",
                        "wallet_profile": "seed-main",
                        "address": "0x2222222222222222222222222222222222222222",
                        "derivation_path": "m/44'/60'/0'/0/3",
                        "address_index": 3,
                        "purpose": "counterparty-acme",
                        "label": "Acme invoices",
                        "status": "active",
                        "created_at_unix": 1
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(loaded.receive_allocations[0].chain_id, 1);
        assert!(loaded.receive_allocations[0].chain_id_assumed);
    }
}
