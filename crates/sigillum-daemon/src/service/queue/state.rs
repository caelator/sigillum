//! Queue state normalization, recovery, and retry classification.

use axum::http::StatusCode;
use sigillum_api::QueueJob;

use crate::service::ServiceError;
use crate::service::helpers::now_unix;

use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_LEGACY_DEFERRED,
    QUEUE_STATE_LEGACY_FAILED, QUEUE_STATE_QUEUED, QUEUE_STATE_RETRYING, QUEUE_STATE_SENT,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::service) struct QueueStateCounts {
    pub blocked: usize,
    pub retrying: usize,
    pub failed: usize,
    pub deferred_legacy: usize,
}

pub(super) enum QueueFailureDisposition {
    Retryable {
        reason: String,
        retry_after_unix: u64,
    },
    FailedTerminal {
        reason: String,
    },
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
            QUEUE_STATE_LEGACY_DEFERRED => counts.deferred_legacy += 1,
            _ => {}
        }
    }
    counts
}

pub(in crate::service) fn is_active_queue_state(state: &str) -> bool {
    matches!(
        normalize_queue_state(state),
        QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED | QUEUE_STATE_RETRYING
    )
}

pub(in crate::service) fn is_active_or_completed_queue_state(state: &str) -> bool {
    is_active_queue_state(state) || normalize_queue_state(state) == QUEUE_STATE_SENT
}

pub(in crate::service) fn queue_status(state: &str) -> String {
    match normalize_queue_state(state) {
        QUEUE_STATE_SENT => "sweep_sent",
        QUEUE_STATE_FAILED_TERMINAL => "sweep_failed",
        QUEUE_STATE_BLOCKED => "sweep_blocked",
        QUEUE_STATE_RETRYING => "sweep_retrying",
        QUEUE_STATE_QUEUED => "sweep_queued",
        _ => "funded",
    }
    .into()
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
    let normalized_state = normalize_queue_state(&job.state);
    if normalized_state != job.state {
        job.state = normalized_state.into();
        changed = true;
    }

    if job.state == QUEUE_STATE_RETRYING {
        if job.next_attempt_after_unix.is_none() {
            job.next_attempt_after_unix = Some(now_unix());
            changed = true;
        }
    } else if job.next_attempt_after_unix.take().is_some() {
        changed = true;
    }

    changed
}

pub(super) fn queue_job_is_runnable(job: &QueueJob, force_target: bool, now: u64) -> bool {
    match normalize_queue_state(&job.state) {
        QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED => true,
        QUEUE_STATE_RETRYING => {
            force_target || job.next_attempt_after_unix.unwrap_or_default() <= now
        }
        _ => false,
    }
}

pub(super) fn classify_queue_error(
    error: ServiceError,
    attempts: u32,
    now: u64,
    policy: crate::policy::RuntimePolicy,
) -> QueueFailureDisposition {
    match error.status() {
        StatusCode::INTERNAL_SERVER_ERROR | StatusCode::TOO_MANY_REQUESTS => {
            QueueFailureDisposition::Retryable {
                reason: error.message().to_string(),
                retry_after_unix: now + queue_retry_delay_secs(attempts, policy),
            }
        }
        _ => QueueFailureDisposition::FailedTerminal {
            reason: error.message().to_string(),
        },
    }
}

fn queue_retry_delay_secs(attempts: u32, policy: crate::policy::RuntimePolicy) -> u64 {
    policy.queue_retry_delay_secs(attempts)
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
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        }
    }

    #[test]
    fn queue_status_normalizes_legacy_states() {
        assert_eq!(queue_status("deferred"), "sweep_blocked");
        assert_eq!(queue_status("failed"), "sweep_failed");
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
                sample_job("deferred", None),
            ],
        };
        let counts = count_queue_states(&queue_state);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.retrying, 1);
        assert_eq!(counts.failed, 2);
        assert_eq!(counts.deferred_legacy, 1);
    }

    #[test]
    fn recover_queue_job_normalizes_legacy_states_and_retry_schedule() {
        let mut deferred = sample_job("deferred", None);
        assert!(recover_queue_job(&mut deferred));
        assert_eq!(deferred.state, "blocked");

        let mut retrying = sample_job("retrying", None);
        assert!(recover_queue_job(&mut retrying));
        assert!(retrying.next_attempt_after_unix.is_some());

        let mut queued = sample_job("queued", Some(10));
        assert!(recover_queue_job(&mut queued));
        assert!(queued.next_attempt_after_unix.is_none());
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
        assert!(!queue_job_is_runnable(&sample_job("sent", None), false, 10));
    }

    #[test]
    fn retry_delay_uses_bounded_backoff() {
        let policy = crate::policy::RuntimePolicy::default();
        assert_eq!(queue_retry_delay_secs(0, policy), 5);
        assert_eq!(queue_retry_delay_secs(1, policy), 5);
        assert_eq!(queue_retry_delay_secs(2, policy), 10);
        assert_eq!(queue_retry_delay_secs(10, policy), 300);
    }

    #[test]
    fn queue_error_classification_distinguishes_retryable_failures() {
        let now = 100;
        let policy = crate::policy::RuntimePolicy::default();
        match classify_queue_error(ServiceError::internal("rpc down"), 0, now, policy) {
            QueueFailureDisposition::Retryable {
                reason,
                retry_after_unix,
            } => {
                assert!(reason.contains("rpc down"));
                assert_eq!(retry_after_unix, 105);
            }
            QueueFailureDisposition::FailedTerminal { .. } => panic!("expected retryable"),
        }

        match classify_queue_error(ServiceError::bad_request("bad payload"), 0, now, policy) {
            QueueFailureDisposition::FailedTerminal { reason } => {
                assert!(reason.contains("bad payload"));
            }
            QueueFailureDisposition::Retryable { .. } => panic!("expected terminal"),
        }
    }
}
