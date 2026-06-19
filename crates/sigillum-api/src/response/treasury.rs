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
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

fn default_require_simulation() -> bool {
    true
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
