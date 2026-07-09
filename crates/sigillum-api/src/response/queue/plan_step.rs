//! Plan-step execution queue payload (W7.2).
//!
//! Carries an approved, simulated consolidation plan step into the queue as a
//! first-class job. The prepared call parameters are COPIED verbatim from the
//! step's preflight-prepared calldata at enqueue time — execution (W7.3) must
//! never rebuild calldata. `simulation_evidence_hash_hex` commits to the
//! prepared call plus the step's simulation evidence so W7.3 can detect any
//! tampering between preflight and execution before signing.
//!
//! This payload must never contain key material, session tokens, or any other
//! secret: only public addresses, ids, hex-encoded transaction parameters,
//! and hashes.

use serde::{Deserialize, Serialize};

use super::super::{WalletAssetKind, WalletPlanStepAction};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStepExecutionPayload {
    pub plan_id: String,
    pub step_id: String,
    pub chain_id: u64,
    /// Source address the step moves value from, with derivation evidence.
    pub source_address: String,
    pub derivation_path: String,
    pub wallet_family: String,
    pub wallet_profile: String,
    pub provider_profile: String,
    pub action: WalletPlanStepAction,
    pub asset_kind: WalletAssetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_address: Option<String>,
    pub amount_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_address: Option<String>,
    /// Prepared call copied from the step's preflight (label, target, data, value).
    pub call_label: String,
    pub call_target_address: String,
    pub call_data_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_value_wei_hex: Option<String>,
    /// SHA-256 over the prepared call + simulation evidence (hex encoded).
    /// W7.3 recomputes and verifies this before signing.
    pub simulation_evidence_hash_hex: String,
    /// Fee basis captured from the step's W6.2 simulation evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas_hex: Option<String>,
    /// Queue job ids of this step's `depends_on` prerequisites (W6.4 ordering).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisite_job_ids: Vec<String>,
}
