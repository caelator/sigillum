//! In-memory registry of long-running daemon operations.
//!
//! [`OperationRegistry`] tracks the daemon's background work (EVM discovery
//! scans today) as [`Operation`] records with cooperative cancellation. The
//! registry is deliberately process-lifetime state: durable progress for
//! scans already lives in the persisted inventory checkpoints and discovery
//! job records, so a restart loses only the live-progress view, never the
//! ability to resume.
//!
//! Cancellation is cooperative: `request_cancel` flips an [`AtomicBool`]
//! shared with the worker via [`OperationHandle`]; the worker polls the flag
//! at its checkpoints (at least once per scanned address index for
//! discovery scans) and transitions the record to `canceled` when it honors
//! the signal. Cancel requests never block on the daemon's operation mutex,
//! so a cancel lands even while a scan holds the mutation guard.
//!
//! The registry is bounded: terminal records are evicted oldest-first once
//! more than [`MAX_TRACKED_OPERATIONS`] are retained; running records are
//! never evicted.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sigillum_api::{
    DaemonEvent, EVENTS_PROTOCOL_VERSION, OPERATION_STATE_CANCEL_REQUESTED,
    OPERATION_STATE_CANCELED, OPERATION_STATE_COMPLETED, OPERATION_STATE_FAILED,
    OPERATION_STATE_RUNNING, Operation, OperationEvent, OperationProgress,
};
use tokio::sync::broadcast;

/// Maximum retained operations. Terminal records beyond this bound are
/// evicted oldest-first; running records are never evicted.
pub(crate) const MAX_TRACKED_OPERATIONS: usize = 50;

/// Worker-side view of a tracked operation: its id plus the shared cancel
/// flag the worker polls at its checkpoints.
#[derive(Clone)]
pub struct OperationHandle {
    id: String,
    cancel_requested: Arc<AtomicBool>,
}

impl OperationHandle {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// True once a cancel request has been accepted for this operation.
    pub(crate) fn cancellation_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Acquire)
    }
}

/// Outcome of a cancel request against the registry.
#[derive(Debug)]
pub enum OperationCancelRequest {
    /// The operation was running; its cancel flag is now set.
    Signaled(Operation),
    /// The operation was already awaiting cancellation; no state change.
    AlreadyRequested(Operation),
    /// The operation is in a terminal state; canceling is a conflict.
    Terminal(Operation),
    /// No operation with this id is tracked.
    NotFound,
}

struct OperationRecord {
    operation: Operation,
    cancel_requested: Arc<AtomicBool>,
}

pub struct OperationRegistry {
    /// Records in insertion order (oldest front, newest back).
    records: VecDeque<OperationRecord>,
    /// Optional fan-out for `operation` events (the daemon's SSE bus).
    /// Emitted inside each mutation, so subscribers observe create/state/
    /// progress transitions in registry-mutation order.
    events: Option<broadcast::Sender<DaemonEvent>>,
}

impl OperationRegistry {
    pub(crate) fn new() -> Self {
        Self {
            records: VecDeque::new(),
            events: None,
        }
    }

    /// Attach the daemon event bus; every subsequent mutation emits an
    /// `operation` event carrying the post-mutation record.
    pub(crate) fn set_event_sender(&mut self, sender: broadcast::Sender<DaemonEvent>) {
        self.events = Some(sender);
    }

    fn emit(events: &Option<broadcast::Sender<DaemonEvent>>, operation: &Operation) {
        if let Some(events) = events {
            let _ = events.send(DaemonEvent::Operation(OperationEvent {
                v: EVENTS_PROTOCOL_VERSION,
                operation: operation.clone(),
            }));
        }
    }

    /// Register a new `running` operation and return the worker's handle.
    pub(crate) fn start(&mut self, kind: &str, related_ids: Vec<String>) -> OperationHandle {
        let id = random_operation_id();
        let now = now_unix();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.records.push_back(OperationRecord {
            operation: Operation {
                id: id.clone(),
                kind: kind.to_string(),
                state: OPERATION_STATE_RUNNING.to_string(),
                progress: OperationProgress {
                    processed: 0,
                    total: None,
                },
                related_ids,
                created_at_unix: now,
                updated_at_unix: now,
                completed_at_unix: None,
                error: None,
            },
            cancel_requested: cancel_requested.clone(),
        });
        if let Some(record) = self.records.back() {
            Self::emit(&self.events, &record.operation);
        }
        self.prune();
        OperationHandle {
            id,
            cancel_requested,
        }
    }

    pub(crate) fn get(&self, id: &str) -> Option<Operation> {
        self.records
            .iter()
            .find(|record| record.operation.id == id)
            .map(|record| record.operation.clone())
    }

    /// Most recent first, capped at `limit`.
    pub(crate) fn list(&self, limit: usize) -> Vec<Operation> {
        self.records
            .iter()
            .rev()
            .take(limit)
            .map(|record| record.operation.clone())
            .collect()
    }

    /// Link a domain record (for example a discovery job id) to an
    /// operation after the domain record exists.
    pub(crate) fn add_related(&mut self, id: &str, related_id: String) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.operation.id == id)
        {
            if !record.operation.related_ids.contains(&related_id) {
                record.operation.related_ids.push(related_id);
            }
            record.operation.updated_at_unix = now_unix();
            Self::emit(&self.events, &record.operation);
        }
    }

    pub(crate) fn set_progress(&mut self, id: &str, processed: u64) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.operation.id == id)
        {
            record.operation.progress.processed = processed;
            record.operation.updated_at_unix = now_unix();
            Self::emit(&self.events, &record.operation);
        }
    }

    /// Set the total work extent for an operation that only learns it after
    /// registration (for example a queue drain's selected job count, known
    /// once the queue is loaded under the guard).
    pub(crate) fn set_total(&mut self, id: &str, total: u64) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.operation.id == id)
        {
            record.operation.progress.total = Some(total);
            record.operation.updated_at_unix = now_unix();
            Self::emit(&self.events, &record.operation);
        }
    }

    /// Mark an operation terminal (`canceled`, `completed`, or `failed`).
    pub(crate) fn finish(&mut self, id: &str, state: &str, error: Option<String>) {
        debug_assert!(is_terminal_state(state));
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.operation.id == id)
        {
            let now = now_unix();
            record.operation.state = state.to_string();
            record.operation.error = error;
            record.operation.updated_at_unix = now;
            record.operation.completed_at_unix = Some(now);
            Self::emit(&self.events, &record.operation);
        }
        self.prune();
    }

    /// Request cancellation of a single operation by id.
    pub(crate) fn request_cancel(&mut self, id: &str) -> OperationCancelRequest {
        let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.operation.id == id)
        else {
            return OperationCancelRequest::NotFound;
        };
        if is_terminal_state(&record.operation.state) {
            return OperationCancelRequest::Terminal(record.operation.clone());
        }
        if record.operation.state == OPERATION_STATE_CANCEL_REQUESTED {
            return OperationCancelRequest::AlreadyRequested(record.operation.clone());
        }
        record.cancel_requested.store(true, Ordering::Release);
        record.operation.state = OPERATION_STATE_CANCEL_REQUESTED.to_string();
        record.operation.updated_at_unix = now_unix();
        Self::emit(&self.events, &record.operation);
        OperationCancelRequest::Signaled(record.operation.clone())
    }

    /// Signal cancellation for the newest live (running or already
    /// cancel-requested) operation linked to `related_id`, if any. Terminal
    /// operations are ignored so callers can fall back to durable state.
    pub(crate) fn request_cancel_for_related(&mut self, related_id: &str) -> Option<Operation> {
        let record = self.records.iter_mut().rev().find(|record| {
            record
                .operation
                .related_ids
                .iter()
                .any(|id| id == related_id)
                && !is_terminal_state(&record.operation.state)
        })?;
        if record.operation.state == OPERATION_STATE_RUNNING {
            record.cancel_requested.store(true, Ordering::Release);
            record.operation.state = OPERATION_STATE_CANCEL_REQUESTED.to_string();
            record.operation.updated_at_unix = now_unix();
            Self::emit(&self.events, &record.operation);
        }
        Some(record.operation.clone())
    }

    /// The newest live operation linked to `related_id`, if any.
    pub(crate) fn running_for_related(&self, related_id: &str) -> Option<Operation> {
        self.records
            .iter()
            .rev()
            .find(|record| {
                record
                    .operation
                    .related_ids
                    .iter()
                    .any(|id| id == related_id)
                    && !is_terminal_state(&record.operation.state)
            })
            .map(|record| record.operation.clone())
    }

    /// Evict terminal records oldest-first beyond the retention bound.
    fn prune(&mut self) {
        while self.records.len() > MAX_TRACKED_OPERATIONS
            && self
                .records
                .front()
                .is_some_and(|record| is_terminal_state(&record.operation.state))
        {
            self.records.pop_front();
        }
    }
}

pub(crate) fn is_terminal_state(state: &str) -> bool {
    matches!(
        state,
        OPERATION_STATE_CANCELED | OPERATION_STATE_COMPLETED | OPERATION_STATE_FAILED
    )
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_operation_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_running_to_completed() {
        let mut registry = OperationRegistry::new();
        let handle = registry.start("inventory_scan_evm", vec![]);
        let id = handle.id().to_string();
        assert!(!handle.cancellation_requested());

        let listed = registry.list(10);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].state, OPERATION_STATE_RUNNING);
        assert_eq!(listed[0].progress.processed, 0);

        registry.set_progress(&id, 5);
        registry.add_related(&id, "job-1".to_string());
        let op = registry.get(&id).unwrap();
        assert_eq!(op.progress.processed, 5);
        assert_eq!(op.related_ids, vec!["job-1".to_string()]);

        registry.finish(&id, OPERATION_STATE_COMPLETED, None);
        let op = registry.get(&id).unwrap();
        assert_eq!(op.state, OPERATION_STATE_COMPLETED);
        assert!(op.completed_at_unix.is_some());
        assert!(op.error.is_none());
    }

    #[test]
    fn cancel_transitions_and_terminal_conflict() {
        let mut registry = OperationRegistry::new();
        let handle = registry.start("inventory_scan_evm", vec![]);
        let id = handle.id().to_string();

        match registry.request_cancel(&id) {
            OperationCancelRequest::Signaled(op) => {
                assert_eq!(op.state, OPERATION_STATE_CANCEL_REQUESTED)
            }
            other => panic!("expected signaled, got {other:?}"),
        }
        assert!(handle.cancellation_requested());

        match registry.request_cancel(&id) {
            OperationCancelRequest::AlreadyRequested(_) => {}
            other => panic!("expected already requested, got {other:?}"),
        }

        registry.finish(&id, OPERATION_STATE_CANCELED, None);
        match registry.request_cancel(&id) {
            OperationCancelRequest::Terminal(op) => {
                assert_eq!(op.state, OPERATION_STATE_CANCELED)
            }
            other => panic!("expected terminal, got {other:?}"),
        }

        assert!(matches!(
            registry.request_cancel("op-missing"),
            OperationCancelRequest::NotFound
        ));
    }

    #[test]
    fn related_lookup_signals_only_live_operations() {
        let mut registry = OperationRegistry::new();
        let first = registry.start("inventory_scan_evm", vec!["job-1".into()]);
        registry.finish(first.id(), OPERATION_STATE_CANCELED, None);
        let _second = registry.start("inventory_scan_evm", vec!["job-2".into()]);

        // A terminal operation is not a live target for its related id.
        assert!(registry.running_for_related("job-1").is_none());
        assert!(registry.request_cancel_for_related("job-1").is_none());
        assert!(!first.cancellation_requested());

        let signaled = registry.request_cancel_for_related("job-2").unwrap();
        assert_eq!(signaled.id, _second.id());
        assert_eq!(signaled.state, OPERATION_STATE_CANCEL_REQUESTED);
        assert!(registry.running_for_related("job-2").is_some());
    }

    #[test]
    fn list_is_recent_first_and_prunes_terminal_records() {
        let mut registry = OperationRegistry::new();
        let mut finished_ids = Vec::new();
        for _ in 0..(MAX_TRACKED_OPERATIONS + 5) {
            let handle = registry.start("inventory_scan_evm", vec![]);
            finished_ids.push(handle.id().to_string());
            registry.finish(handle.id(), OPERATION_STATE_COMPLETED, None);
        }
        // One still-running record is never evicted even beyond the bound.
        let handle = registry.start("inventory_scan_evm", vec![]);
        let running_id = handle.id().to_string();
        registry.set_progress(&running_id, 1);

        let listed = registry.list(MAX_TRACKED_OPERATIONS);
        assert_eq!(listed.len(), MAX_TRACKED_OPERATIONS);
        assert_eq!(listed[0].id, running_id);
        assert!(listed.iter().any(|op| op.id == running_id));
        assert!(listed.iter().all(|op| op.id != finished_ids[0]));
    }
}
