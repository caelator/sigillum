//! Queue state normalization, recovery, and retry classification.

use sigillum_api::{QueueJob, QueueJobPayload};

use crate::service::helpers::now_unix;

#[cfg(test)]
use super::status::{count_queue_states, queue_status};

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
mod tests {
    use sigillum_api::{QueueJob, QueueJobPayload};

    use super::*;

    fn sample_job(state: &str, next_attempt_after_unix: Option<u64>) -> QueueJob {
        QueueJob {
            id: "job-1".into(),
            state: state.into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "profile".into(),
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
                stealth_hash_convention: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }
    }

    #[test]
    fn queue_status_normalizes_legacy_states() {
        assert_eq!(queue_status("deferred"), "sweep_blocked");
        assert_eq!(queue_status("failed"), "sweep_failed");
        assert_eq!(
            queue_status("operator_action_required"),
            "sweep_operator_action_required"
        );
        assert_eq!(queue_status("sent"), "sweep_sent");
    }

    #[test]
    fn queue_counts_track_new_and_legacy_states() {
        let queue_state = crate::queue_store::QueueState {
            jobs: vec![
                sample_job("blocked", None),
                sample_job("retrying", Some(10)),
                sample_job("failed_terminal", None),
                sample_job("failed", None),
                sample_job("operator_action_required", None),
                sample_job("deferred", None),
            ],
        };
        let counts = count_queue_states(&queue_state);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.retrying, 1);
        assert_eq!(counts.failed, 2);
        assert_eq!(counts.operator_action_required, 1);
        assert_eq!(counts.deferred_legacy, 1);
    }

    #[test]
    fn recover_queue_job_normalizes_legacy_states_and_retry_schedule() {
        let mut deferred = sample_job("deferred", None);
        assert!(recover_queue_job(&mut deferred));
        assert_eq!(deferred.state, "blocked");
        assert_eq!(
            deferred.last_error.as_deref(),
            Some("legacy deferred queue job normalized to blocked")
        );

        let mut deferred_with_reason = sample_job("deferred", None);
        deferred_with_reason.last_error = Some("waiting for gas".into());
        assert!(recover_queue_job(&mut deferred_with_reason));
        assert_eq!(deferred_with_reason.state, "blocked");
        assert_eq!(
            deferred_with_reason.last_error.as_deref(),
            Some("waiting for gas")
        );

        let mut retrying = sample_job("retrying", None);
        assert!(recover_queue_job(&mut retrying));
        assert!(retrying.next_attempt_after_unix.is_some());

        let mut queued = sample_job("queued", Some(10));
        assert!(recover_queue_job(&mut queued));
        assert!(queued.next_attempt_after_unix.is_none());

        let mut operator_action_required = sample_job("operator_action_required", None);
        assert!(!recover_queue_job(&mut operator_action_required));
        assert_eq!(operator_action_required.state, "operator_action_required");
    }

    #[test]
    fn queue_runnable_rules_respect_retry_deadlines() {
        assert!(queue_job_is_runnable(
            &sample_job("queued", None),
            false,
            10
        ));
        assert!(queue_job_is_runnable(
            &sample_job("blocked", None),
            false,
            10
        ));
        assert!(!queue_job_is_runnable(
            &sample_job("retrying", Some(20)),
            false,
            10
        ));
        assert!(queue_job_is_runnable(
            &sample_job("retrying", Some(20)),
            true,
            10
        ));
        assert!(queue_job_is_runnable(
            &sample_job("retrying", Some(5)),
            false,
            10
        ));
        assert!(!queue_job_is_runnable(
            &sample_job("operator_action_required", None),
            false,
            10
        ));
        assert!(!queue_job_is_runnable(
            &sample_job("operator_action_required", None),
            true,
            10
        ));
        assert!(!queue_job_is_runnable(&sample_job("sent", None), false, 10));
    }

    fn plan_step_job(state: &str) -> QueueJob {
        let mut job = sample_job(state, None);
        job.payload =
            QueueJobPayload::PlanStepExecution(Box::new(sigillum_api::PlanStepExecutionPayload {
                plan_id: "plan_1".into(),
                step_id: "step_1".into(),
                chain_id: 1,
                source_address: "0x0000000000000000000000000000000000000001".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                wallet_family: "eth-seed".into(),
                wallet_profile: "seed-a".into(),
                provider_profile: "mainnet".into(),
                action: "sweep_native".into(),
                asset_kind: "native".into(),
                asset_address: None,
                amount_hex: "0x1".into(),
                destination_address: None,
                call_label: "native.transfer(value)".into(),
                call_target_address: "0x0000000000000000000000000000000000000002".into(),
                call_data_hex: "0x".into(),
                call_value_wei_hex: None,
                simulation_evidence_hash_hex: "ab".repeat(32),
                fee_basis: None,
                max_priority_fee_per_gas_hex: None,
                max_fee_per_gas_hex: None,
                prerequisite_job_ids: Vec::new(),
            }));
        job
    }

    // W7.4: `sent` for a `PlanStepExecution` job means "broadcast, awaiting
    // confirmation" — the drain loop keeps visiting it (E2 crash-resumption
    // relies on this) until `confirmed` or `operator_action_required`.
    #[test]
    fn w7_4_confirmed_state_semantics() {
        assert!(queue_job_is_runnable(&plan_step_job("sent"), false, 10));
        assert!(!queue_job_is_runnable(
            &plan_step_job("confirmed"),
            false,
            10
        ));
        assert_eq!(queue_status("confirmed"), "sweep_confirmed");
        assert!(is_active_or_completed_queue_state("confirmed"));
        assert!(is_active_or_completed_queue_state("sent"));
    }
}
