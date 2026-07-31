//! Queue response contracts.

use serde::{Deserialize, Serialize};

mod plan_step;
pub use plan_step::PlanStepExecutionPayload;

mod payload;
pub use payload::QueueJobPayload;

mod receipt;
pub use receipt::QueueJobReceipt;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJob {
    pub id: String,
    pub state: String,
    pub attempts: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_after_unix: Option<u64>,
    #[serde(flatten)]
    pub payload: QueueJobPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_transaction_hash_hex: Option<String>,
    /// W7.4 receipt confirmation fields, flattened (see `QueueJobReceipt`).
    #[serde(flatten)]
    pub receipt: QueueJobReceipt,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobListResponse {
    pub jobs: Vec<QueueJob>,
    /// Pagination window metadata. Present only when the request supplied
    /// `limit` and/or `offset`; absent on legacy (parameterless) calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<super::PaginationInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueEnqueueResponse {
    pub status: String,
    pub job: QueueJob,
}
