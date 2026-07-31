use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::operation_registry::{OperationCancelRequest, OperationHandle};

use super::{AppState, LockState};

impl AppState {
    pub async fn operation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.operation_lock.lock().await
    }

    // ── Background operation registry ─────────────────────────────
    //
    // Cancel signaling deliberately bypasses `operation_lock` so a cancel
    // lands while a worker (e.g. a discovery scan) holds the mutation guard.

    /// Register a new `running` background operation and return the worker's
    /// handle (id + shared cancel flag).
    pub fn start_operation(&self, kind: &str, related_ids: Vec<String>) -> OperationHandle {
        self.operations.lock().start(kind, related_ids)
    }

    /// List tracked operations, most recent first, capped at `limit`.
    pub fn list_operations(&self, limit: usize) -> Vec<sigillum_api::Operation> {
        self.operations.lock().list(limit)
    }

    pub fn get_operation(&self, id: &str) -> Option<sigillum_api::Operation> {
        self.operations.lock().get(id)
    }

    /// Link a domain record (e.g. a discovery job id) to an operation.
    pub fn operation_add_related(&self, id: &str, related_id: String) {
        self.operations.lock().add_related(id, related_id);
    }

    pub fn operation_set_progress(&self, id: &str, processed: u64) {
        self.operations.lock().set_progress(id, processed);
    }

    /// Set the total work extent for an operation (learned after
    /// registration, e.g. a drain's selected job count).
    pub fn operation_set_progress_total(&self, id: &str, total: u64) {
        self.operations.lock().set_total(id, total);
    }

    /// Mark an operation terminal (`canceled`, `completed`, or `failed`).
    pub fn finish_operation(&self, id: &str, state: &str, error: Option<String>) {
        self.operations.lock().finish(id, state, error);
    }

    /// Mark an operation completed only if no cancel request won the
    /// registry-mutex race. Returns false when cancellation must be honored.
    pub fn complete_operation_if_not_canceled<E>(
        &self,
        id: &str,
        persist_completion: impl FnOnce() -> Result<(), E>,
    ) -> Result<bool, E> {
        self.operations
            .lock()
            .complete_if_not_canceled(id, persist_completion)
    }

    /// Request cancellation of a single operation by id.
    pub fn request_operation_cancel(&self, id: &str) -> OperationCancelRequest {
        self.operations.lock().request_cancel(id)
    }

    /// Signal cancellation for the newest live operation linked to a domain
    /// record id (e.g. a discovery job id), if any.
    pub fn request_operation_cancel_for_related(
        &self,
        related_id: &str,
    ) -> Option<sigillum_api::Operation> {
        self.operations
            .lock()
            .request_cancel_for_related(related_id)
    }

    /// The newest live (non-terminal) operation linked to a domain record id.
    pub fn running_operation_for_related(
        &self,
        related_id: &str,
    ) -> Option<sigillum_api::Operation> {
        self.operations.lock().running_for_related(related_id)
    }

    // ── Background scheduler status ───────────────────────────────

    /// Snapshot of the background scheduler's status for diagnostics.
    #[must_use]
    pub fn scheduler_status(&self) -> sigillum_api::SchedulerStatusResponse {
        self.scheduler_status.lock().clone()
    }

    /// Record a scheduler cycle's outcome (`advanced`, `idle`,
    /// `skipped_locked`, `skipped_guard_busy`, or `failed`). A failed cycle
    /// increments the consecutive-failure counter the scheduler loop uses
    /// for exponential backoff; any other outcome resets it.
    pub(crate) fn scheduler_note_cycle(&self, outcome: &'static str, failed: bool) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut status = self.scheduler_status.lock();
        status.last_tick_at_unix = Some(now);
        status.last_cycle_outcome = Some(outcome.into());
        if failed {
            status.consecutive_failures += 1;
        } else {
            status.consecutive_failures = 0;
        }
    }

    // ── Event bus (SSE fan-out) ───────────────────────────────────
    //
    // `operation` events are emitted inside the operation registry itself
    // (it holds a sender clone), which keeps emission exactly at the
    // mutation points. Queue and status transitions publish through the
    // helpers below.

    /// Register a new subscriber on the daemon event bus.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<sigillum_api::DaemonEvent> {
        self.events.subscribe()
    }

    /// Publish a queue-job state transition. `previous_state` is the job's
    /// state before the mutation (`None` for a newly created job); no event
    /// is emitted when the state did not actually change.
    pub(crate) fn publish_queue_job_transition(
        &self,
        job: &sigillum_api::QueueJob,
        previous_state: Option<&str>,
    ) {
        if previous_state == Some(job.state.as_str()) {
            return;
        }
        self.events.publish(sigillum_api::DaemonEvent::Queue(
            sigillum_api::QueueJobEvent {
                v: sigillum_api::EVENTS_PROTOCOL_VERSION,
                job_id: job.id.clone(),
                state: job.state.clone(),
                last_error: job.last_error.clone(),
            },
        ));
    }

    /// Publish a `status` event (`locked`, `unlocked`,
    /// `compartment_switched`), stamped with the current default active
    /// compartment.
    pub(super) fn publish_status_event(&self, kind: &'static str) {
        self.events.publish(sigillum_api::DaemonEvent::Status(
            sigillum_api::StatusEvent {
                v: sigillum_api::EVENTS_PROTOCOL_VERSION,
                kind: kind.into(),
                active_compartment_id: self.default_active_compartment_id(),
            },
        ));
    }

    /// Build the `snapshot` event payload for a new SSE subscriber: the
    /// current lock status plus the live (non-terminal) operations, so the
    /// client can sync without a second request.
    pub fn events_snapshot(&self) -> sigillum_api::EventsSnapshot {
        let operations = self
            .operations
            .lock()
            .list(crate::operation_registry::MAX_TRACKED_OPERATIONS)
            .into_iter()
            .filter(|operation| !crate::operation_registry::is_terminal_state(&operation.state))
            .collect();
        sigillum_api::EventsSnapshot {
            v: sigillum_api::EVENTS_PROTOCOL_VERSION,
            locked: !self.is_unlocked(),
            active_compartment_id: self.default_active_compartment_id(),
            operations,
        }
    }

    #[must_use]
    pub fn queue_execution_pause_latched(&self) -> bool {
        self.queue_execution_paused.load(Ordering::Acquire)
    }

    pub fn set_queue_execution_pause_latch(&self, paused: bool) {
        self.queue_execution_paused.store(paused, Ordering::Release);
    }

    #[must_use]
    pub fn is_locking(&self) -> bool {
        *self.lock_state.lock() == LockState::Locking
    }

    #[must_use]
    pub fn begin_locking(&self) -> bool {
        if !self.is_unlocked() {
            return false;
        }
        let mut state = self.lock_state.lock();
        if *state == LockState::Locking {
            return false;
        }
        *state = LockState::Locking;
        true
    }

    /// Linearize provider submission against [`Self::begin_locking`].
    /// The caller retains `operation_lock` through dispatch, so the lock
    /// latch and the submission admission point have one stable ordering.
    #[must_use]
    pub(crate) fn admit_broadcast_if_ready(&self) -> bool {
        *self.lock_state.lock() == LockState::Ready
    }

    pub fn finish_locking(&self) {
        *self.lock_state.lock() = LockState::Ready;
    }

    /// Lock and zeroize all unlocked compartments while leaving the daemon running.
    pub async fn lock_now(&self) -> bool {
        if !self.begin_locking() {
            return false;
        }
        let _guard = self.operation_guard().await;
        self.lock_all();
        true
    }
}
