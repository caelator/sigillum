//! State-preserving dispatch for queue jobs that already carry transaction
//! authority or have crossed the broadcast boundary.

use std::collections::HashMap;

use sigillum_api::{QueueJob, QueueJobPayload};

use crate::service::{ServiceResult, SigillumService};

use super::state::normalize_queue_state;
use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_PREPARED, QUEUE_STATE_SENT, QUEUE_STATE_SUBMITTED_UNKNOWN,
    QueueExecution,
};

pub(super) fn is_sent_plan_step(job: &QueueJob) -> bool {
    normalize_queue_state(&job.state) == QUEUE_STATE_SENT
        && matches!(&job.payload, QueueJobPayload::PlanStepExecution(_))
}

impl SigillumService {
    /// Return `Some` when the job must stay on an integrity/recovery path.
    /// The outer result is reserved for durable-marker failures; the inner
    /// result keeps normal queue errors flowing through `outcomes::apply`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn process_in_flight_queue_job(
        &self,
        token: &str,
        queue: &mut crate::queue_store::QueueState,
        job_index: usize,
        job: &QueueJob,
        job_states: &HashMap<String, String>,
        now: u64,
        allow_submission: bool,
    ) -> ServiceResult<Option<ServiceResult<QueueExecution>>> {
        let normalized_state = normalize_queue_state(&job.state);
        let has_replay_bytes = job.receipt.signed_raw_transaction_hex.is_some();
        let integrity_required = has_replay_bytes
            || matches!(
                normalized_state,
                QUEUE_STATE_PREPARED | QUEUE_STATE_SUBMITTED_UNKNOWN
            );
        if integrity_required {
            if let Some(reason) = super::broadcast::prepared_integrity_error(job) {
                return Ok(Some(Ok(QueueExecution::OperatorActionRequired(reason))));
            }
        }

        if is_sent_plan_step(job) {
            let QueueJobPayload::PlanStepExecution(step_payload) = &job.payload else {
                return Ok(Some(Ok(QueueExecution::OperatorActionRequired(
                    "sent_state_invalid: only plan-step jobs may be receipt-polled".into(),
                ))));
            };
            return Ok(Some(
                self.process_plan_step_execution(
                    token,
                    &job.id,
                    &job.state,
                    job.transaction_hash_hex.as_deref(),
                    job.receipt.broadcast_at_unix,
                    step_payload,
                    job_states,
                )
                .await,
            ));
        }

        if !has_replay_bytes || job.transaction_hash_hex.is_none() {
            return Ok(None);
        }

        // `blocked` + replay bytes + a broadcast marker can exist from the
        // pre-v5 gate-order bug. Reconcile it as submitted-unknown before any
        // exact-byte resubmission.
        let resume_existing_submission = normalized_state == QUEUE_STATE_SUBMITTED_UNKNOWN
            || (normalized_state == QUEUE_STATE_BLOCKED && job.receipt.broadcast_at_unix.is_some());
        if allow_submission || resume_existing_submission {
            self.persist_queue_submission_marker(queue, job_index, now)?;
        }
        let submitted_job = queue.jobs[job_index].clone();
        Ok(Some(
            self.broadcast_prepared_queue_job(
                token,
                &submitted_job,
                resume_existing_submission,
                allow_submission,
            )
            .await,
        ))
    }
}
