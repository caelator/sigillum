//! Queue state normalization, recovery, and retry classification.

use sigillum_api::{QueueJob, QueueJobPayload};

use crate::service::helpers::now_unix;

use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL,
    QUEUE_STATE_LEGACY_DEFERRED, QUEUE_STATE_LEGACY_FAILED, QUEUE_STATE_OPERATOR_ACTION_REQUIRED,
    QUEUE_STATE_PREPARED, QUEUE_STATE_QUEUED, QUEUE_STATE_RETRYING, QUEUE_STATE_SENT,
    QUEUE_STATE_SUBMITTED_UNKNOWN,
};

pub(in crate::service) fn is_active_queue_state(state: &str) -> bool {
    matches!(
        normalize_queue_state(state),
        QUEUE_STATE_QUEUED
            | QUEUE_STATE_BLOCKED
            | QUEUE_STATE_RETRYING
            | QUEUE_STATE_PREPARED
            | QUEUE_STATE_SUBMITTED_UNKNOWN
    )
}

pub(in crate::service) fn is_active_or_completed_queue_state(state: &str) -> bool {
    is_active_queue_state(state)
        || matches!(
            normalize_queue_state(state),
            QUEUE_STATE_SENT | QUEUE_STATE_CONFIRMED
        )
}

/// Terminal failure (including the legacy `failed` alias).
pub(in crate::service) fn queue_job_failed_state(state: &str) -> bool {
    normalize_queue_state(state) == QUEUE_STATE_FAILED_TERMINAL
}

/// Terminal SUCCESS for a sweep job: `confirmed` (W7.4 finality, reached by
/// `PlanStepExecution` jobs) or `sent` — the legacy `EthSeed*`/`EthStealth*`
/// families' own terminal state ("broadcast, done"; those families never
/// poll receipts, see `plan_steps/receipts.rs`). The one-time receive
/// lifecycle (plan task 3.3) retires its allocation when its sweep job
/// reaches either.
pub(in crate::service) fn queue_job_sweep_settled_state(state: &str) -> bool {
    matches!(
        normalize_queue_state(state),
        QUEUE_STATE_SENT | QUEUE_STATE_CONFIRMED
    )
}

pub(in crate::service) fn queue_job_operator_action_required(state: &str) -> bool {
    normalize_queue_state(state) == QUEUE_STATE_OPERATOR_ACTION_REQUIRED
}

/// Park a job for operator inspection (E1 semantics): it is no longer
/// runnable until the operator resolves it out-of-band.
pub(in crate::service) fn mark_job_operator_action_required(
    job: &mut QueueJob,
    reason: String,
    now: u64,
) {
    job.state = QUEUE_STATE_OPERATOR_ACTION_REQUIRED.into();
    job.last_error = Some(reason);
    job.updated_at_unix = now;
    job.next_attempt_after_unix = None;
}

pub(super) fn normalize_queue_state(state: &str) -> &str {
    match state {
        QUEUE_STATE_LEGACY_DEFERRED => QUEUE_STATE_BLOCKED,
        QUEUE_STATE_LEGACY_FAILED => QUEUE_STATE_FAILED_TERMINAL,
        other => other,
    }
}

pub(in crate::service) fn recover_queue_job(job: &mut QueueJob) -> bool {
    let mut changed = false;
    let legacy_deferred = job.state == QUEUE_STATE_LEGACY_DEFERRED;
    let normalized_state = normalize_queue_state(&job.state);
    if normalized_state != job.state {
        job.state = normalized_state.into();
        changed = true;
    }
    if legacy_deferred && job.last_error.is_none() {
        job.last_error = Some("legacy deferred queue job normalized to blocked".into());
        changed = true;
    }

    if matches!(
        job.state.as_str(),
        QUEUE_STATE_RETRYING | QUEUE_STATE_SUBMITTED_UNKNOWN
    ) {
        if job.next_attempt_after_unix.is_none() {
            job.next_attempt_after_unix = Some(now_unix());
            changed = true;
        }
    } else if job.next_attempt_after_unix.take().is_some() {
        changed = true;
    }

    let integrity_required = matches!(
        job.state.as_str(),
        QUEUE_STATE_PREPARED | QUEUE_STATE_SUBMITTED_UNKNOWN
    ) || (job.receipt.signed_raw_transaction_hex.is_some()
        && matches!(
            job.state.as_str(),
            QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED | QUEUE_STATE_RETRYING | QUEUE_STATE_SENT
        ));
    if integrity_required {
        if let Some(reason) = super::broadcast::prepared_integrity_error(job) {
            mark_job_operator_action_required(job, reason, now_unix());
            changed = true;
        }
    }

    changed
}

pub(super) fn queue_job_is_runnable(job: &QueueJob, force_target: bool, now: u64) -> bool {
    match normalize_queue_state(&job.state) {
        QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED => true,
        QUEUE_STATE_RETRYING => {
            force_target || job.next_attempt_after_unix.unwrap_or_default() <= now
        }
        QUEUE_STATE_PREPARED => true,
        QUEUE_STATE_SUBMITTED_UNKNOWN => {
            force_target || job.next_attempt_after_unix.unwrap_or_default() <= now
        }
        // W7.4: a `PlanStepExecution` job in `sent` only AWAITS
        // confirmation — revisit it to keep polling (incl. after a
        // restart: E2). Other kinds keep `sent` as their pre-W7.4 terminal
        // meaning — never re-driven, byte-identical.
        QUEUE_STATE_SENT => matches!(job.payload, QueueJobPayload::PlanStepExecution(_)),
        QUEUE_STATE_OPERATOR_ACTION_REQUIRED => false,
        _ => false,
    }
}

/// Due-work statistics for the background scheduler's pre-check and for
/// diagnostics: `due_now` counts the jobs a drain would attempt immediately
/// (runnable states whose backoff, if any, has elapsed), and
/// `next_retry_at_unix` is the earliest backoff deadline among the jobs
/// still waiting. Computed without the operation guard — a cheap read-only
/// estimate; the drain re-decides authoritatively under the guard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::service) struct QueueDueStats {
    pub due_now: usize,
    pub next_retry_at_unix: Option<u64>,
}

pub(in crate::service) fn queue_due_stats(
    queue: &crate::queue_store::QueueState,
    now: u64,
) -> QueueDueStats {
    let mut stats = QueueDueStats::default();
    for job in &queue.jobs {
        if queue_job_is_runnable(job, false, now) {
            stats.due_now += 1;
            continue;
        }
        if matches!(
            normalize_queue_state(&job.state),
            QUEUE_STATE_RETRYING | QUEUE_STATE_SUBMITTED_UNKNOWN
        ) {
            if let Some(next_attempt) = job.next_attempt_after_unix {
                stats.next_retry_at_unix = Some(
                    stats
                        .next_retry_at_unix
                        .map_or(next_attempt, |earliest| earliest.min(next_attempt)),
                );
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests;
