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
use crate::service::chains::ensure_builtin_chain_profiles;

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
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.wallet-inventory", 14);

    fn from_enveloped_json(
        path: &std::path::Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        match version {
            1..=14 => {
                let mut state: Self = serde_json::from_value(data).map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "failed to parse sigillum.wallet-inventory schema payload {}: {error}",
                            path.display()
                        ),
                    )
                })?;
                ensure_builtin_chain_profiles(&mut state.chain_profiles);
                Ok(state)
            }
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
    let mut state: WalletInventoryState =
        crate::json_store::load_json_document(&path)?.unwrap_or_default();
    ensure_builtin_chain_profiles(&mut state.chain_profiles);
    Ok(state)
}

pub fn save_wallet_inventory(
    base_dir: &std::path::Path,
    state: &WalletInventoryState,
) -> Result<(), std::io::Error> {
    let path = wallet_inventory_path(base_dir);
    let mut state = state.clone();
    ensure_builtin_chain_profiles(&mut state.chain_profiles);
    crate::json_store::save_json_document(&path, &state)
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
            chain_ids: vec![1],
            gap_limit: 20,
            max_index: 100,
            addresses_scanned: 1,
            active_addresses: 1,
            holdings_detected: 1,
            checkpoints: Vec::new(),
            block_cursors: Vec::new(),
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
            native_decimals: 18,
            finality_blocks: 0,
            dormancy_block_window: sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
            permit2_address: None,
            explorer_url: None,
            capabilities: vec!["native".into(), "erc20".into()],
            enabled: true,
            source: "operator".into(),
            builtin: true,
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
            last_activity_block: None,
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
        assert_eq!(state.chain_profiles.len(), 5);
        assert!(state.chain_profiles.iter().all(|profile| profile.builtin));
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
        let mut future_holding = sample_holding();
        future_holding.id = "holding_future".into();
        future_holding.asset_kind = sigillum_api::WalletAssetKind::Other("future_asset".into());
        state.holdings.push(future_holding);
        state.consolidation_plans.push(ConsolidationPlan {
            id: "plan_future".into(),
            status: sigillum_api::WalletPlanStatus::Other("partially_approved".into()),
            chain_id: 1,
            destination_address: None,
            created_at_unix: 1,
            updated_at_unix: 2,
            summary: sigillum_api::ConsolidationPlanSummary {
                total_steps: 1,
                blocked_steps: 0,
                review_required_steps: 1,
                approved_steps: 0,
                executable_steps: 0,
                value_items: 1,
            },
            policy_violations: Vec::new(),
            linkage_findings: Vec::new(),
            steps: vec![sigillum_api::ConsolidationPlanStep {
                id: "step_future".into(),
                action: sigillum_api::WalletPlanStepAction::Other("future_action".into()),
                status: sigillum_api::WalletPlanStepStatus::ReviewRequired,
                wallet_family: "eth-seed".into(),
                wallet_profile: "seed-main".into(),
                provider_profile: "mainnet".into(),
                chain_id: 1,
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                asset_kind: sigillum_api::WalletAssetKind::Other("future_asset".into()),
                asset_address: None,
                token_id_hex: None,
                counterparty_address: None,
                protocol_address: None,
                claim_adapter: None,
                claim_index_hex: None,
                claim_proof: Vec::new(),
                amount_hex: "0x1".into(),
                destination_address: None,
                signer_status: sigillum_api::WalletSignerStatus::Other("future_signer".into()),
                simulation_status: sigillum_api::WalletSimulationStatus::Other(
                    "future_simulation".into(),
                ),
                simulation_evidence: Vec::new(),
                risk_level: "low".into(),
                blockers: Vec::new(),
                linkage_warnings: Vec::new(),
                auto_eligible: false,
                approved: false,
            }],
        });
        state.risk_catalog.push(sample_risk_catalog_entry());
        state.receive_allocations.push(sample_receive_allocation());

        save_wallet_inventory(dir.path(), &state).unwrap();
        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(loaded.jobs.len(), 1);
        assert_eq!(loaded.chain_profiles.len(), 5);
        assert_eq!(loaded.watch_address_book.len(), 1);
        assert_eq!(loaded.addresses.len(), 1);
        assert_eq!(loaded.holdings.len(), 2);
        assert_eq!(loaded.risk_catalog.len(), 1);
        assert_eq!(loaded.addresses[0].wallet_profile, "seed-main");
        assert!(loaded.treasury_policy.is_none());
        assert_eq!(loaded.receive_allocations.len(), 1);
        assert_eq!(loaded.receive_allocations[0].address_index, 3);
        assert_eq!(loaded.receive_allocations[0].status, "active");
        assert_eq!(loaded.receive_allocations[0].purpose, "counterparty-acme");
        assert!(loaded.receive_allocations[0].retired_at_unix.is_none());
        assert_eq!(
            loaded.holdings[1].asset_kind,
            sigillum_api::WalletAssetKind::Other("future_asset".into())
        );
        assert_eq!(
            loaded.consolidation_plans[0].status,
            sigillum_api::WalletPlanStatus::Other("partially_approved".into())
        );
        assert_eq!(
            loaded.consolidation_plans[0].steps[0].action,
            sigillum_api::WalletPlanStepAction::Other("future_action".into())
        );
    }

    #[test]
    fn v12_inventory_without_block_cursors_loads_with_empty_cursors() {
        let dir = TempDir::new().unwrap();
        let mut state = WalletInventoryState::default();
        state.jobs.push(sample_job());
        let envelope = json!({
            "schema": "sigillum.wallet-inventory",
            "schema_version": 12,
            "data": serde_json::to_value(&state).unwrap(),
        });
        std::fs::write(
            wallet_inventory_path(dir.path()),
            serde_json::to_vec_pretty(&envelope).unwrap(),
        )
        .unwrap();

        let loaded = load_wallet_inventory(dir.path()).unwrap();
        assert_eq!(loaded.jobs.len(), 1);
        assert!(loaded.jobs[0].block_cursors.is_empty());
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
        assert_eq!(saved["schema_version"], json!(14));
        assert_eq!(saved["data"]["chain_profiles"].as_array().unwrap().len(), 5);
        assert!(saved["data"]["watch_address_book"].is_array());
        assert!(saved["data"]["jobs"].is_array());
        assert!(saved["data"]["addresses"].is_array());
        assert!(saved["data"]["holdings"].is_array());
        assert!(saved["data"]["risk_catalog"].is_array());
    }

    #[test]
    fn legacy_v11_inventory_loads_with_chain_registry_and_mainnet_defaults() {
        let dir = TempDir::new().unwrap();
        let path = wallet_inventory_path(dir.path());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.wallet-inventory",
                "schema_version": 11,
                "data": {
                    "chain_profiles": [{
                        "name": "custom-rollup",
                        "chain_family": "evm",
                        "chain_id": 999,
                        "native_symbol": "ETH",
                        "enabled": true,
                        "source": "operator",
                        "created_at_unix": 1,
                        "updated_at_unix": 2
                    }],
                    "addresses": [{
                        "id": "addr_legacy",
                        "wallet_family": "eth-seed",
                        "wallet_profile": "seed-main",
                        "provider_profile": "mainnet",
                        "address": "0x1111111111111111111111111111111111111111",
                        "derivation_path": "m/44'/60'/0'/0/0",
                        "address_index": 0,
                        "activity_state": "funded",
                        "native_balance_wei_hex": "0x1",
                        "transaction_count": 1,
                        "source": "legacy",
                        "first_seen_at_unix": 1,
                        "last_checked_at_unix": 2
                    }],
                    "holdings": [{
                        "id": "holding_legacy",
                        "wallet_family": "eth-seed",
                        "wallet_profile": "seed-main",
                        "provider_profile": "mainnet",
                        "address": "0x1111111111111111111111111111111111111111",
                        "derivation_path": "m/44'/60'/0'/0/0",
                        "asset_kind": "native",
                        "amount_hex": "0x1",
                        "source": "legacy",
                        "status": "detected",
                        "first_seen_at_unix": 1,
                        "last_checked_at_unix": 2
                    }],
                    "consolidation_plans": [{
                        "id": "plan_legacy",
                        "status": "review_required",
                        "created_at_unix": 1,
                        "updated_at_unix": 2,
                        "summary": {
                            "total_steps": 1,
                            "blocked_steps": 0,
                            "review_required_steps": 1,
                            "approved_steps": 0,
                            "executable_steps": 0,
                            "value_items": 1
                        },
                        "steps": [{
                            "id": "step_legacy",
                            "action": "sweep_native",
                            "status": "review_required",
                            "wallet_family": "eth-seed",
                            "wallet_profile": "seed-main",
                            "provider_profile": "mainnet",
                            "address": "0x1111111111111111111111111111111111111111",
                            "derivation_path": "m/44'/60'/0'/0/0",
                            "asset_kind": "native",
                            "amount_hex": "0x1",
                            "signer_status": "unknown",
                            "simulation_status": "not_run",
                            "risk_level": "low",
                            "auto_eligible": false,
                            "approved": false
                        }]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(loaded.chain_profiles.len(), 6);
        let custom = loaded
            .chain_profiles
            .iter()
            .find(|profile| profile.name == "custom-rollup")
            .expect("custom chain profile loaded");
        assert!(!custom.builtin);
        assert_eq!(custom.native_decimals, 18);
        assert_eq!(custom.finality_blocks, 0);
        assert!(custom.permit2_address.is_none());
        assert_eq!(loaded.addresses[0].chain_id, 1);
        assert_eq!(loaded.holdings[0].chain_id, 1);
        assert_eq!(loaded.consolidation_plans[0].steps[0].chain_id, 1);
    }

    #[test]
    fn legacy_v13_inventory_defaults_dormancy_window_and_last_activity_block() {
        let dir = TempDir::new().unwrap();
        let path = wallet_inventory_path(dir.path());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.wallet-inventory",
                "schema_version": 13,
                "data": {
                    "chain_profiles": [{
                        "name": "custom-rollup",
                        "chain_family": "evm",
                        "chain_id": 999,
                        "native_symbol": "ETH",
                        "enabled": true,
                        "source": "operator",
                        "created_at_unix": 1,
                        "updated_at_unix": 2
                    }],
                    "addresses": [{
                        "id": "addr_legacy",
                        "wallet_family": "eth-seed",
                        "wallet_profile": "seed-main",
                        "provider_profile": "mainnet",
                        "chain_id": 999,
                        "address": "0x1111111111111111111111111111111111111111",
                        "derivation_path": "m/44'/60'/0'/0/0",
                        "address_index": 0,
                        "activity_state": "funded",
                        "native_balance_wei_hex": "0x1",
                        "transaction_count": 1,
                        "source": "legacy",
                        "first_seen_at_unix": 1,
                        "last_checked_at_unix": 2
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_wallet_inventory(dir.path()).unwrap();

        assert_eq!(
            loaded.addresses[0].last_activity_block, None,
            "legacy addresses default to no derived activity block"
        );
        let custom = loaded
            .chain_profiles
            .iter()
            .find(|profile| profile.name == "custom-rollup")
            .expect("custom profile loaded");
        assert_eq!(
            custom.dormancy_block_window,
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW
        );
        assert!(
            loaded
                .chain_profiles
                .iter()
                .filter(|profile| profile.builtin)
                .all(|profile| profile.dormancy_block_window
                    == sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW)
        );
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
