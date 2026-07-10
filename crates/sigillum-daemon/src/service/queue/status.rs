//! Queue status summaries and deposit-facing status labels.

use super::state::normalize_queue_state;
use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL,
    QUEUE_STATE_LEGACY_DEFERRED, QUEUE_STATE_LEGACY_FAILED, QUEUE_STATE_OPERATOR_ACTION_REQUIRED,
    QUEUE_STATE_PREPARED, QUEUE_STATE_QUEUED, QUEUE_STATE_RETRYING, QUEUE_STATE_SENT,
    QUEUE_STATE_SUBMITTED_UNKNOWN,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::service) struct QueueStateCounts {
    pub blocked: usize,
    pub retrying: usize,
    pub failed: usize,
    pub operator_action_required: usize,
    pub deferred_legacy: usize,
}

pub(in crate::service) fn count_queue_states(
    queue_state: &crate::queue_store::QueueState,
) -> QueueStateCounts {
    let mut counts = QueueStateCounts::default();
    for job in &queue_state.jobs {
        match job.state.as_str() {
            QUEUE_STATE_BLOCKED => counts.blocked += 1,
            QUEUE_STATE_RETRYING => counts.retrying += 1,
            QUEUE_STATE_FAILED_TERMINAL | QUEUE_STATE_LEGACY_FAILED => counts.failed += 1,
            QUEUE_STATE_OPERATOR_ACTION_REQUIRED => counts.operator_action_required += 1,
            QUEUE_STATE_LEGACY_DEFERRED => counts.deferred_legacy += 1,
            _ => {}
        }
    }
    counts
}

pub(in crate::service) fn queue_status(state: &str) -> String {
    match normalize_queue_state(state) {
        QUEUE_STATE_SENT => "sweep_sent",
        QUEUE_STATE_CONFIRMED => "sweep_confirmed",
        QUEUE_STATE_FAILED_TERMINAL => "sweep_failed",
        QUEUE_STATE_OPERATOR_ACTION_REQUIRED => "sweep_operator_action_required",
        QUEUE_STATE_BLOCKED => "sweep_blocked",
        QUEUE_STATE_RETRYING => "sweep_retrying",
        QUEUE_STATE_PREPARED => "sweep_prepared",
        QUEUE_STATE_SUBMITTED_UNKNOWN => "sweep_submitted_unknown",
        QUEUE_STATE_QUEUED => "sweep_queued",
        _ => "funded",
    }
    .into()
}
