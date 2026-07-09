//! Queue processing response contracts.

use serde::{Deserialize, Serialize};

use super::QueueJob;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaintenanceFailureBreakdown {
    #[serde(default)]
    pub provider_error: usize,
    #[serde(default)]
    pub policy_block: usize,
    #[serde(default)]
    pub insufficient_gas: usize,
    #[serde(default)]
    pub validation: usize,
    #[serde(default)]
    pub unknown: usize,
    /// W7.4: on-chain revert discovered via receipt polling (never
    /// auto-retried; distinct from `insufficient_gas`/`validation`).
    #[serde(default)]
    pub on_chain_revert: usize,
    /// W7.4: broadcast rejected after the single allowed retry — nonce too
    /// low twice, or underpriced/replacement-underpriced after the one fee
    /// bump within the policy cap.
    #[serde(default)]
    pub broadcast_rejected: usize,
    /// W7.4: no receipt appeared within the confirmation wall-clock budget;
    /// the broadcast is NEVER assumed to have failed.
    #[serde(default)]
    pub receipt_timeout: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueProcessResponse {
    pub processed: usize,
    pub succeeded: usize,
    #[serde(default)]
    pub blocked: usize,
    #[serde(default)]
    pub retrying: usize,
    #[serde(default)]
    pub operator_action_required: usize,
    pub failed: usize,
    /// W7.4: `PlanStepExecution` jobs whose receipt reached the chain's
    /// configured finality depth this drain (state `confirmed`) — distinct
    /// from `succeeded`, which counts a successful BROADCAST this drain.
    #[serde(default)]
    pub confirmed: usize,
    #[serde(default)]
    pub failures_by_cause: MaintenanceFailureBreakdown,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    pub jobs: Vec<QueueJob>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueExecutionPauseResponse {
    pub status: String,
    pub execution_paused: bool,
}
