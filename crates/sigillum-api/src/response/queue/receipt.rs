//! W7.4 receipt-confirmation fields, `#[serde(flatten)]`-ed into `QueueJob`
//! so the wire shape stays flat (identical to if these fields were declared
//! directly on `QueueJob`) while keeping `queue.rs` under the house
//! architecture line cap.

use serde::{Deserialize, Serialize};

/// All `None` until a receipt is observed. `broadcast_at_unix` drives the
/// confirmation timeout window and lets restart resume polling without
/// re-broadcasting; `confirmations` is the last observed depth;
/// `receipt_block_number`/`receipt_gas_used_hex`/`receipt_status`
/// (`"success"`/`"reverted"`) are recorded for both a confirmed success and
/// an on-chain revert.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueJobReceipt {
    /// Exact signed transaction bytes durably prepared before the first
    /// network submission. Once present, queue recovery must never sign this
    /// job again; it may only submit these exact bytes or poll their hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_raw_transaction_hex: Option<String>,
    /// Wall-clock time at which the signed bytes crossed the durable
    /// prepare barrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared_at_unix: Option<u64>,
    /// Hash of the canonical queue payload at preparation time. Recovery
    /// refuses to submit stored bytes if the job payload changed afterward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared_payload_hash_hex: Option<String>,
    /// Domain-separated hash over both the canonical queue payload and exact
    /// signed bytes. This detects cross-job record swaps/corruption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared_binding_hash_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmations: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_block_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_gas_used_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_status: Option<String>,
}
