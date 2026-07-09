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
