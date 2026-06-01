//! Wallet inventory and discovery request contracts.

use serde::{Deserialize, Serialize};

/// Configured ERC-20-like DeFi position token to probe during wallet discovery.
///
/// Many first-class DeFi positions are represented by receipt/share tokens:
/// lending aTokens/cTokens, staking receipts, vault shares, and LP tokens. A
/// probe records the protocol provenance separately from ordinary wallet tokens
/// so the inventory can distinguish protocol exposure from spendable ERC-20s.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DefiTokenProbe {
    pub protocol: String,
    pub token_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_address: Option<String>,
}

/// Run read-only EVM wallet discovery for imported seed and xpub profiles.
///
/// When no wallet filter is provided, all `eth-seed` and `eth-xpub` profiles
/// are scanned. When no provider filter is provided, every configured EVM
/// provider profile is scanned so one derived address can be checked across
/// multiple L1/L2 networks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryScanRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub token_addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_erc20_transfers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_discovery_from_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_discovery_to_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_discovery_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_erc20_allowances: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowance_spender_addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowance_discovery_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_permit2_allowances: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permit2_contract_addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permit2_spender_addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permit2_allowance_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_erc721_transfers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_erc1155_transfers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_nft_operator_approvals: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nft_operator_addresses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft_operator_approval_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discover_defi_token_positions: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defi_token_probes: Vec<DefiTokenProbe>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defi_position_limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft_discovery_from_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft_discovery_to_block: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nft_discovery_limit: Option<usize>,
}

/// Create or update a local chain profile used by discovery and planning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileUpsertRequest {
    pub name: String,
    pub chain_family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Delete a local chain profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileDeleteRequest {
    pub name: String,
}

/// Mutate a persisted discovery job by ID.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobMutationRequest {
    pub id: String,
}

/// Add or replace a local risk catalog entry for a spender/operator address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogUpsertRequest {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Delete a local risk catalog entry by address.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogDeleteRequest {
    pub address: String,
}

/// Generate a dry-run consolidation plan from the current inventory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanGenerateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_watch_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_queue_low_risk: Option<bool>,
}

/// Approve reviewable consolidation plan steps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanApproveRequest {
    pub plan_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
}

/// Simulate/preflight reviewable or approved consolidation plan steps.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanSimulateRequest {
    pub plan_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_ids: Vec<String>,
}
