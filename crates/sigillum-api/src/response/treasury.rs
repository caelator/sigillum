//! Treasury console response contracts.
//!
//! The treasury overview is a read-only aggregation of wallet inventory,
//! profile routing configuration, risk findings, and consolidation planning
//! state. It exists so a desktop operator can answer "what does the treasury
//! hold, where is it, and what needs review" from one surface without paging
//! through raw inventory rows.

use serde::{Deserialize, Serialize};

/// Per-chain rollup of discovered native value.
///
/// Native amounts are never summed across chains: one chain's wei is not
/// fungible with another's, so each chain reports its own total.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryChainSummary {
    pub chain_id: u64,
    pub native_symbol: String,
    pub address_count: usize,
    pub funded_address_count: usize,
    pub native_total_wei_hex: String,
}

/// Rollup for one wallet group (family + profile) on one chain.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryGroupSummary {
    pub wallet_family: String,
    pub wallet_profile: String,
    pub chain_id: u64,
    pub address_count: usize,
    pub funded_address_count: usize,
    pub native_total_wei_hex: String,
    pub signer_address_count: usize,
    pub watch_only_address_count: usize,
    pub erc20_holding_count: usize,
    pub nft_holding_count: usize,
    pub defi_holding_count: usize,
    pub claimable_holding_count: usize,
    pub approval_exposure_count: usize,
    pub dormant_candidate_count: usize,
}

/// Treasury routing configuration and observed balances for a seed profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryRoutingStatus {
    pub wallet_profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treasury_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_destination_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_native_balance_wei_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub treasury_native_balance_wei_hex: Option<String>,
    /// True when sweeps from this profile have a configured treasury target.
    pub routing_ready: bool,
}

/// Open risk findings grouped by severity.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryRiskSummary {
    pub total_findings: usize,
    pub critical_findings: usize,
    pub high_findings: usize,
    pub medium_findings: usize,
    pub low_findings: usize,
}

/// Consolidation planning posture for the console.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryPlanSummary {
    pub total_plans: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_plan_status: Option<String>,
    pub latest_review_required_steps: usize,
    pub latest_approved_steps: usize,
    pub latest_executable_steps: usize,
    pub latest_blocked_steps: usize,
    /// Plan-level treasury policy violations on the latest plan, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub latest_policy_violations: Vec<String>,
}

/// Receive-address allocation posture for the console.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveSummary {
    pub active_allocations: usize,
    pub retired_allocations: usize,
    /// Distinct purposes among active allocations.
    pub purposes: usize,
}

/// Per-maintenance-cycle W8 treasury automation outcome.
///
/// `generated_steps` and `enqueued_steps` are reported distinctly because
/// generation is review-first and enqueue additionally requires the W7.1 gates
/// and a passed simulation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryAutomationRunSummary {
    pub generated_steps: usize,
    pub enqueued_steps: usize,
    pub skipped_steps: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_reasons: Vec<String>,
}

/// Treasury-console posture for W8 automation.
///
/// Counts aggregate steps of plans whose origin is `treasury_automation`;
/// `enqueued_steps` counts only those with a recorded queue job id.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryAutomationStatus {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hot_overflow_wei_hex: Option<String>,
    pub generated_steps: usize,
    pub enqueued_steps: usize,
}

/// Read-only treasury console aggregation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryOverviewResponse {
    pub generated_at_unix: u64,
    pub tracked_address_count: usize,
    pub funded_address_count: usize,
    pub watch_only_address_count: usize,
    pub signer_address_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<TreasuryChainSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<TreasuryGroupSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routing: Vec<TreasuryRoutingStatus>,
    pub risk: TreasuryRiskSummary,
    pub plans: TreasuryPlanSummary,
    /// Defaults for payloads produced before receive allocations existed.
    #[serde(default)]
    pub receive: TreasuryReceiveSummary,
    #[serde(default)]
    pub automation: TreasuryAutomationStatus,
}

/// Operator-approved consolidation destination.
///
/// The address is the policy key; the label only helps a reviewer recognize
/// the destination later (e.g. "cold-treasury", "ops-safe").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryAllowedDestination {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Operator-defined treasury guardrails for consolidation planning.
///
/// The policy is a local, review-first safety net: it never builds or signs
/// transactions, it only blocks plan steps that route value outside the
/// destination allowlist or above the configured native caps. Caps are
/// 0x-prefixed uint256 wei quantities so they compare exactly against
/// inventory amounts without float loss.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryPolicy {
    /// Disabled policies are kept for editing but enforce nothing.
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_destinations: Vec<TreasuryAllowedDestination>,
    /// Per-step native value ceiling; sweeps above it are blocked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_step_native_wei_hex: Option<String>,
    /// Whole-plan native value ceiling across non-blocked sweep steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_plan_native_wei_hex: Option<String>,
    /// Defaults to true: simulation stays mandatory unless explicitly waived.
    #[serde(default = "default_require_simulation")]
    pub require_simulation: bool,
    #[serde(default)]
    pub allow_raw_digest_signing: bool,
    /// Fail-closed: when true, any step the linkage analyzer flags is hard-blocked.
    #[serde(default)]
    pub block_cross_party_linkage: bool,
    /// Fail-closed opt-in: allows merkle-distributor-v1 claim steps to clear the
    /// claim_execution_disabled blocker only once every execution gate holds
    /// (simulation passed, claim contract trusted or operator-reviewed in the
    /// risk catalog, step explicitly approved).
    #[serde(default)]
    pub allow_claim_execution: bool,
    /// Fail-closed opt-in (default false, per Decision Register D-6): when true,
    /// the consolidation planner may emit fund_gas steps that fund a source
    /// address's gas shortfall from the wallet's sponsor address; cross-party
    /// sponsor funding is still linkage-checked and hard-blocked when
    /// block_cross_party_linkage is on.
    #[serde(default)]
    pub allow_gas_topups: bool,
    /// Optional 0x-prefixed uint256 wei cap on a single fund_gas amount; a
    /// computed top-up above the cap is not emitted and the dependent step stays
    /// gas-blocked with a reason naming this cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gas_topup_wei_hex: Option<String>,
    /// Fail-closed master gate (default false, per Decision Register D-6): no
    /// consolidation plan step may be enqueued or executed unless this gate and
    /// the step's per-family gate hold and execution is not paused.
    #[serde(default)]
    pub allow_plan_execution: bool,
    /// Fail-closed per-family opt-in for sweep steps (native/ERC-20/NFT transfers).
    #[serde(default)]
    pub allow_sweep_execution: bool,
    /// Fail-closed per-family opt-in for approval-revoke steps.
    #[serde(default)]
    pub allow_revoke_execution: bool,
    /// Fail-closed per-family opt-in for DeFi exit-adapter steps. Claim steps
    /// keep using allow_claim_execution; gas top-ups keep using allow_gas_topups.
    #[serde(default)]
    pub allow_exit_execution: bool,
    /// Runtime kill switch: when true, all queue execution halts immediately (no
    /// new job starts at drain time). Honored even when the policy is disabled.
    #[serde(default)]
    pub execution_paused: bool,
    /// Optional 0x-prefixed uint256 wei fee ceiling for W7.4 fee-bump logic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas_cap_hex: Option<String>,
    /// Maximum age in seconds of a step's simulation evidence before approval
    /// downgrades the simulation back to required (forces re-simulation).
    #[serde(default = "default_simulation_freshness_secs")]
    pub simulation_freshness_secs: u64,
    /// Hot-wallet refill floor: sweeps route to the hot address while its
    /// balance is below this threshold. Default 1 ETH (preserves the pre-policy hardcode).
    #[serde(default = "default_hot_floor_wei_hex")]
    pub hot_floor_wei_hex: String,
    /// Hot-wallet refill ceiling (floor <= target). Refills top up to this
    /// level; it does not itself trigger routing to hot. Default 1 ETH.
    #[serde(default = "default_hot_target_wei_hex")]
    pub hot_target_wei_hex: String,
    /// Optional hot-wallet overflow threshold; when the hot address balance
    /// exceeds it, maintenance may plan a hot->treasury sweep of the excess
    /// above hot_target_wei_hex. None disables overflow detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hot_overflow_wei_hex: Option<String>,
    /// Fail-closed opt-in (default false, Decision Register D-6): when true AND
    /// the policy is enabled, the maintenance cycle may GENERATE hot-overflow /
    /// treasury-refill plan steps and auto-enqueue them only when the W7.1
    /// execution gates hold and simulation passed. Off means maintenance
    /// behavior is unchanged.
    #[serde(default)]
    pub allow_treasury_automation: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

fn default_require_simulation() -> bool {
    true
}

fn default_simulation_freshness_secs() -> u64 {
    900
}

fn default_hot_floor_wei_hex() -> String {
    "0xde0b6b3a7640000".to_string()
}

fn default_hot_target_wei_hex() -> String {
    "0xde0b6b3a7640000".to_string()
}

/// Current treasury policy; `None` until an operator configures one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryPolicyResponse {
    pub policy: Option<TreasuryPolicy>,
}

/// Result of a treasury policy update.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryPolicyMutationResponse {
    pub status: String,
    pub policy: TreasuryPolicy,
}

/// One purpose-labeled receive address derived from a wallet profile's xpub.
///
/// Allocations exist so an operator hands out a FRESH address per
/// counterparty/purpose instead of reusing one: derivation is pure local xpub
/// math (no provider or network calls), and per-purpose addresses keep
/// unrelated payments unlinkable on-chain. Retired allocations are kept for
/// history; only `status == "active"` entries should be handed out.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveAllocation {
    pub id: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    #[serde(default = "default_legacy_mainnet_chain_id")]
    pub chain_id: u64,
    #[serde(default = "default_legacy_chain_id_assumed")]
    pub chain_id_assumed: bool,
    pub address: String,
    pub derivation_path: String,
    pub address_index: u32,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// "active" or "retired".
    pub status: String,
    pub created_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retired_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterparty_id: Option<String>,
}

/// A payer/counterparty an operator hands a dedicated receive address to.
///
/// First-class so each party can be issued a fresh address, keeping parties
/// unlinkable on-chain. Parties are referenced by receive allocations via
/// `counterparty_id`; deleting a party unbinds its allocations rather than
/// erasing allocation history.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counterparty {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep_destination_address: Option<String>,
    pub created_at_unix: u64,
}

/// All receive allocations, active and retired.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveAllocationListResponse {
    pub allocations: Vec<TreasuryReceiveAllocation>,
}

/// Result of allocating or rotating a receive address.
///
/// For rotation, `allocation` is the NEW active allocation that replaced the
/// retired one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreasuryReceiveAllocationMutationResponse {
    pub status: String,
    pub allocation: TreasuryReceiveAllocation,
}

/// All known counterparties, newest-first is NOT required; preserve insertion order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterpartyListResponse {
    pub parties: Vec<Counterparty>,
}

/// Result of creating, updating, or deleting a counterparty.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterpartyMutationResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Counterparty>,
}

/// One active receiving surface from either an HD allocation or stealth deposit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingItem {
    pub source_type: String,
    pub address: String,
    pub chain_id: u64,
    #[serde(default = "default_legacy_chain_id_assumed")]
    pub chain_id_assumed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkage_warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance_native_wei_hex: Option<String>,
    pub balance_known: bool,
    pub status: String,
    pub created_at_unix: u64,
}

/// Receiving items grouped by counterparty, with `None` reserved for unassigned items.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingPartyGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterparty: Option<Counterparty>,
    pub item_count: u32,
    pub native_total_wei_hex: String,
    #[serde(default)]
    pub items: Vec<ReceivingItem>,
}

/// Count and balance totals across the receiving overview.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingTotals {
    pub item_count: u32,
    pub hd_count: u32,
    pub stealth_count: u32,
    pub native_total_wei_hex: String,
}

/// Balance freshness coverage for the persisted receiving overview.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingCoverage {
    pub addresses_total: u32,
    pub addresses_with_known_balance: u32,
    pub note: String,
}

/// Read-only merged view of active HD receiving allocations and stealth deposits.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingOverviewResponse {
    pub generated_at_unix: u64,
    pub include_retired: bool,
    #[serde(default)]
    pub groups: Vec<ReceivingPartyGroup>,
    pub totals: ReceivingTotals,
    pub coverage: ReceivingCoverage,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivingRefreshResponse {
    pub generated_at_unix: u64,
    pub addresses_requested: u32,
    pub addresses_refreshed: u32,
    pub addresses_skipped: u32,
    pub stealth_refreshed: bool,
    pub provider_status: String,
    #[serde(default)]
    pub errors: Vec<String>,
}

fn default_legacy_mainnet_chain_id() -> u64 {
    1
}

fn default_legacy_chain_id_assumed() -> bool {
    true
}
