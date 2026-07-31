//! Async operation resource types.
//!
//! Long-running daemon work (EVM discovery scans, queue drains, and
//! maintenance cycles today) is tracked as an `Operation`: a
//! process-lifetime record with cooperative cancellation and progress.
//! Operations are the operator surface for work the daemon performs outside
//! the request/response cycle; durable domain state (discovery jobs,
//! checkpoints, inventory, the queue itself) stays in the existing persisted
//! records and is linked via [`Operation::related_ids`].
//!
//! State machine:
//! - `running` — the operation is active (possibly queued behind the
//!   daemon's mutation mutex).
//! - `cancel_requested` — a cancel was accepted; the worker honors it at its
//!   next checkpoint and transitions to `canceled`.
//! - `canceled` / `completed` / `failed` — terminal states.
//!
//! States and kinds are free-form `snake_case` strings (same convention as
//! [`crate::error_codes`]) so newer daemons can introduce new kinds without
//! breaking older clients; consumers must treat unrecognized values as
//! opaque.

use serde::{Deserialize, Serialize};

/// [`Operation::state`] — the operation is actively running (or queued
/// behind the daemon's mutation mutex).
pub const OPERATION_STATE_RUNNING: &str = "running";
/// [`Operation::state`] — cancellation was requested but the worker has not
/// honored it yet.
pub const OPERATION_STATE_CANCEL_REQUESTED: &str = "cancel_requested";
/// [`Operation::state`] — terminal: the worker stopped early because a
/// cancellation was honored.
pub const OPERATION_STATE_CANCELED: &str = "canceled";
/// [`Operation::state`] — terminal: the work finished successfully.
pub const OPERATION_STATE_COMPLETED: &str = "completed";
/// [`Operation::state`] — terminal: the work failed; `error` carries the
/// human-readable cause.
pub const OPERATION_STATE_FAILED: &str = "failed";

/// [`Operation::kind`] — an EVM wallet-inventory discovery scan
/// (`POST /api/inventory/scan/evm` with `run_async`, or a discovery-job
/// resume). `related_ids` carries the associated discovery job id.
pub const OPERATION_KIND_INVENTORY_SCAN_EVM: &str = "inventory_scan_evm";

/// [`Operation::kind`] — a queue drain (`POST /api/queue/process`, sync or
/// `run_async`). `progress.processed` counts jobs attempted so far and
/// `progress.total` the jobs selected for the run (see
/// `service/queue/processing.rs` for the exact selection semantics);
/// `related_ids` stays empty — the queue store itself is the durable domain
/// record. Cancellation is honored between jobs only — an in-flight job
/// always finishes its current attempt (including its broadcast, bracketed
/// by the prepared/submitted barriers), so a canceled drain reports the
/// processed vs remaining counts in `progress`.
pub const OPERATION_KIND_QUEUE_PROCESS: &str = "queue_process";

/// [`Operation::kind`] — a maintenance cycle (`POST /api/maintenance/run`,
/// sync or `run_async`). The cycle runs three stages in order —
/// `treasury_automation`, `deposit_refresh`, `queue_drain` — encoded in
/// `related_ids` as `stage:<name>` markers in execution order;
/// `progress.processed` counts completed stages and `progress.total` the
/// stage count. Cancellation is honored between stages: a canceled cycle
/// stops before the next stage with the completed stages' effects durably
/// persisted.
pub const OPERATION_KIND_MAINTENANCE_RUN: &str = "maintenance_run";

/// [`Operation::kind`] — a background-scheduler cycle that advanced work
/// (plan task 1.6). Scheduler ticks are deliberately NOT registered one by
/// one (the registry retains 50 records); a record appears only for a cycle
/// that actually advanced work (processed > 0 queue jobs or refreshed > 0
/// deposits), registered already-`completed` as a summary of what ran.
/// `related_ids` carries the `stage:<name>` markers for the stages that ran
/// (`treasury_automation`, `deposit_refresh`, `queue_drain`), and
/// `progress.processed`/`progress.total` count the advanced units (jobs
/// attempted plus deposits refreshed).
pub const OPERATION_KIND_SCHEDULER_CYCLE: &str = "scheduler_cycle";

/// Progress counters for a running or finished [`Operation`].
///
/// `total` is `None` when the work cannot know its extent up front (for
/// example discovery scans stop early at the gap limit).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationProgress {
    /// Units of work completed so far. For discovery scans this is the
    /// number of per-provider address observations recorded; for queue
    /// drains the jobs attempted; for maintenance cycles the stages
    /// completed.
    pub processed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// A long-running daemon operation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Operation {
    pub id: String,
    /// Machine-readable work kind (`inventory_scan_evm`, …).
    pub kind: String,
    /// Lifecycle state (`running`, `cancel_requested`, `canceled`,
    /// `completed`, `failed`).
    pub state: String,
    pub progress: OperationProgress,
    /// Domain records this operation drives — for discovery scans the
    /// persisted discovery job id from `GET /api/discovery/jobs`, for
    /// maintenance cycles the `stage:<name>` markers in execution order (see
    /// [`OPERATION_KIND_MAINTENANCE_RUN`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_ids: Vec<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of `GET /api/operations` (most recent first, bounded).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationListResponse {
    pub operations: Vec<Operation>,
}

/// Result of `GET /api/operations/{id}`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationResponse {
    pub operation: Operation,
}

/// Result of `POST /api/operations/{id}/cancel`.
///
/// `status` mirrors the operation state after the request: `cancel_requested`
/// when the cancel was accepted, or the unchanged terminal state with a 409
/// error envelope when the operation already finished.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationMutationResponse {
    pub status: String,
    pub operation: Operation,
}
