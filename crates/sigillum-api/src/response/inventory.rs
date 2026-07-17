use serde::{Deserialize, Serialize};

use super::{
    WalletAddressActivityState, WalletAddressClassification, WalletAssetKind, WalletPlanStatus,
    WalletPlanStepAction, WalletPlanStepStatus, WalletSignerStatus, WalletSimulationStatus,
};

pub const DEFAULT_DORMANCY_BLOCK_WINDOW: u64 = 1_000_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryAddress {
    pub id: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    #[serde(default = "default_inventory_chain_id")]
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_index: Option<u32>,
    pub address_index: u32,
    pub activity_state: WalletAddressActivityState,
    pub native_balance_wei_hex: String,
    pub transaction_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classifications: Vec<WalletAddressClassification>,
    pub source: String,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletAssetHolding {
    pub id: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    #[serde(default = "default_inventory_chain_id")]
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    pub asset_kind: WalletAssetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_index_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_proof: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spam_label: Option<String>,
    pub amount_hex: String,
    pub source: String,
    pub status: String,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletDiscoveryCheckpoint {
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_index: Option<u32>,
    pub next_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scanned_index: Option<u32>,
    pub consecutive_empty: u32,
    pub completed: bool,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletDiscoveryBlockCursor {
    pub address: String,
    #[serde(default = "default_inventory_chain_id")]
    pub chain_id: u64,
    pub topic_family: String,
    pub last_scanned_block: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletDiscoveryJob {
    pub id: String,
    pub status: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallet_families: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wallet_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain_ids: Vec<u64>,
    pub gap_limit: u32,
    pub max_index: u32,
    pub addresses_scanned: usize,
    pub active_addresses: usize,
    pub holdings_detected: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<WalletDiscoveryCheckpoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_cursors: Vec<WalletDiscoveryBlockCursor>,
    pub started_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Whether same-chain address probes were partitioned across provider
    /// profiles (stable per-address hash assignment). Present and `true`
    /// only when the scan request opted in AND at least one chain had
    /// multiple selected providers; resume replays it so the resumed scan
    /// keeps the same per-provider disjoint coverage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_providers: Option<bool>,
    /// Per-provider observed-address counts for a partitioned scan, so an
    /// operator can verify disjoint coverage (each count sums into
    /// `addresses_scanned`). Empty for non-partitioned scans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_partition_observations: Vec<ProviderPartitionObservation>,
}

/// Observed-address count attributed to one provider profile within a
/// partitioned discovery scan (plan task 3.1).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderPartitionObservation {
    pub provider_profile: String,
    pub chain_id: u64,
    pub addresses_observed: usize,
}

/// Cached NFT metadata and spam-review state for a discovered token.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataCacheEntry {
    pub chain_id: u64,
    pub contract_address: String,
    pub token_id_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub spam_label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spam_reasons: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_skipped_reason: Option<String>,
    pub updated_at_unix: u64,
}

/// Per-collection operator opt-in for NFT metadata fetching.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataCollectionOptIn {
    pub chain_id: u64,
    pub contract_address: String,
    pub enabled: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

/// List of NFT metadata collection opt-ins and the configured IPFS gateway.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataOptInListResponse {
    pub opt_ins: Vec<NftMetadataCollectionOptIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipfs_gateway_url: Option<String>,
}

/// Result of creating or updating an NFT metadata collection opt-in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataOptInMutationResponse {
    pub status: String,
    pub opt_in: NftMetadataCollectionOptIn,
}

/// Current NFT metadata fetch settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataSettingsResponse {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipfs_gateway_url: Option<String>,
}

/// NFT metadata fetch item skipped before a network request was made.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataFetchSkip {
    pub chain_id: u64,
    pub contract_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id_hex: Option<String>,
    pub reason: String,
}

/// Result of an explicit opt-in NFT metadata fetch pass.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftMetadataFetchResponse {
    pub fetched: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<NftMetadataFetchSkip>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<NftMetadataCacheEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryListResponse {
    pub jobs: Vec<WalletDiscoveryJob>,
    pub addresses: Vec<WalletInventoryAddress>,
    pub holdings: Vec<WalletAssetHolding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nft_metadata_cache: Vec<NftMetadataCacheEntry>,
    /// Pagination window metadata for the `addresses` list (the other lists
    /// are always returned in full). Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletInventoryScanResponse {
    pub job: WalletDiscoveryJob,
    pub addresses: Vec<WalletInventoryAddress>,
    pub holdings: Vec<WalletAssetHolding>,
    /// The background operation driving the scan. Present only when the
    /// request set `run_async: true`; `addresses`/`holdings` are then empty
    /// and the job is the accepted (still running) discovery job. Poll
    /// `GET /api/operations/{id}` or `GET /api/discovery/jobs` for progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<crate::response::Operation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfile {
    pub name: String,
    pub chain_family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_profile: Option<String>,
    pub native_symbol: String,
    #[serde(default = "default_chain_native_decimals")]
    pub native_decimals: u8,
    #[serde(default)]
    pub finality_blocks: u64,
    #[serde(default = "default_chain_dormancy_block_window")]
    pub dormancy_block_window: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permit2_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uniswap_v2_router_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub enabled: bool,
    pub source: String,
    #[serde(default)]
    pub builtin: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

fn default_chain_native_decimals() -> u8 {
    18
}

fn default_chain_dormancy_block_window() -> u64 {
    DEFAULT_DORMANCY_BLOCK_WINDOW
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileListResponse {
    pub profiles: Vec<ChainProfile>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainProfileMutationResponse {
    pub status: String,
    pub profile: ChainProfile,
}

fn default_inventory_chain_id() -> u64 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobListResponse {
    pub jobs: Vec<WalletDiscoveryJob>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryJobMutationResponse {
    pub status: String,
    pub job: WalletDiscoveryJob,
    /// The background operation started by a resume (or signaled by a
    /// cancel). Absent when no live operation is involved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<crate::response::Operation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogEntry {
    pub address: String,
    pub label: String,
    pub risk_level: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogListResponse {
    pub entries: Vec<RiskCatalogEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskCatalogMutationResponse {
    pub status: String,
    pub entry: RiskCatalogEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRegistryEntry {
    #[serde(alias = "chainId")]
    pub chain_id: u64,
    pub address: String,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRegistryList {
    pub id: String,
    pub name: String,
    pub compartment_id: usize,
    pub source: String,
    pub entries: Vec<TokenRegistryEntry>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRegistryListResponse {
    pub lists: Vec<TokenRegistryList>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenRegistryMutationResponse {
    pub status: String,
    pub list: TokenRegistryList,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskFinding {
    pub id: String,
    pub category: String,
    pub risk_level: String,
    pub status: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub chain_id: u64,
    pub address: String,
    pub subject_type: String,
    pub subject: String,
    pub source: String,
    pub recommendation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    pub first_seen_at_unix: u64,
    pub last_checked_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskFindingListResponse {
    pub findings: Vec<RiskFinding>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanSummary {
    pub total_steps: usize,
    pub blocked_steps: usize,
    pub review_required_steps: usize,
    pub approved_steps: usize,
    pub executable_steps: usize,
    pub value_items: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanStep {
    pub id: String,
    /// Planner-assigned 0-based ordering hint for export/execution.
    #[serde(default)]
    pub sequence: u32,
    /// Ids of plan steps that must be exported/executed successfully before this step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub action: WalletPlanStepAction,
    pub status: WalletPlanStepStatus,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    #[serde(default = "default_inventory_chain_id")]
    pub chain_id: u64,
    pub address: String,
    pub derivation_path: String,
    pub asset_kind: WalletAssetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_index_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_proof: Vec<String>,
    /// Uniswap v2 exit token0 address computed at plan time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_token0_address: Option<String>,
    /// Uniswap v2 exit token1 address computed at plan time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_token1_address: Option<String>,
    /// Uniswap v2 minimum token0 amount computed at plan time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_amount0_min_hex: Option<String>,
    /// Uniswap v2 minimum token1 amount computed at plan time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_amount1_min_hex: Option<String>,
    /// Uniswap v2 removeLiquidity deadline computed at plan time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_deadline_unix: Option<u64>,
    pub amount_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    pub signer_status: WalletSignerStatus,
    pub simulation_status: WalletSimulationStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub simulation_evidence: Vec<String>,
    pub risk_level: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linkage_warnings: Vec<String>,
    pub auto_eligible: bool,
    pub approved: bool,
    /// Queue job id once this step has been enqueued for execution (W7.2).
    /// Persistent idempotency marker: a step is enqueued at most once; a
    /// failed job requires operator re-approval before re-enqueue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_job_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlan {
    pub id: String,
    pub status: WalletPlanStatus,
    #[serde(default = "default_inventory_chain_id")]
    pub chain_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    /// How the plan came to exist; None = operator-generated,
    /// "treasury_automation" = generated by the W8 maintenance cycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub summary: ConsolidationPlanSummary,
    /// Plan-wide treasury policy violations (e.g. plan native cap exceeded).
    /// Step-level violations live in each step's `blockers` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_violations: Vec<String>,
    /// Plan-wide cross-payer linkage findings (privacy): destinations that
    /// would publicly link multiple distinct payers via a shared recipient.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linkage_findings: Vec<String>,
    pub steps: Vec<ConsolidationPlanStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanListResponse {
    pub plans: Vec<ConsolidationPlan>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsolidationPlanMutationResponse {
    pub status: String,
    pub plan: ConsolidationPlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<ConsolidationPlan>,
}

/// Result of enqueueing a single plan step as a queue job (W7.2).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEnqueueStepResponse {
    pub status: String,
    pub plan_id: String,
    pub step_id: String,
    pub job: super::queue::QueueJob,
}

/// One step enqueued by a bulk plan enqueue (W7.2).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEnqueuedStep {
    pub step_id: String,
    pub job_id: String,
}

/// One step a bulk plan enqueue skipped, with its named fail-closed reason.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEnqueueSkippedStep {
    pub step_id: String,
    pub action: WalletPlanStepAction,
    pub reason: String,
}

/// Result of a bulk plan enqueue (W7.2): steps enqueued in dependency order
/// plus the skipped steps with named reasons.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEnqueuePlanResponse {
    pub status: String,
    pub plan_id: String,
    pub enqueued: Vec<PlanEnqueuedStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<PlanEnqueueSkippedStep>,
}
