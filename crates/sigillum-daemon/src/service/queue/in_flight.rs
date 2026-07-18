//! State-preserving dispatch for queue jobs that already carry transaction
//! authority or have crossed the broadcast boundary.

use std::collections::HashMap;

use sigillum_api::{QueueJob, QueueJobPayload};

use crate::service::{ServiceResult, SigillumService};

use super::state::normalize_queue_state;
use super::{
    QUEUE_STATE_PREPARED, QUEUE_STATE_SENT, QUEUE_STATE_SUBMITTED_UNKNOWN, QueueExecution,
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

        let dependency_block_reason = match &job.payload {
            QueueJobPayload::PlanStepExecution(payload) => {
                super::plan_steps::dependency_block_reason(payload, job_states)
            }
            _ => None,
        };

        // A durable marker means the RPC submission boundary may have been
        // crossed regardless of the job state subsequently recorded. For
        // example, provider acceptance followed by an audit-write failure is
        // reduced to `retrying` while the marker and replay bytes remain.
        // Always reconcile chain truth before any exact-byte resubmission.
        let resume_existing_submission = normalized_state == QUEUE_STATE_SUBMITTED_UNKNOWN
            || job.receipt.broadcast_at_unix.is_some();
        if let Some(reason) = dependency_block_reason.as_deref()
            && !resume_existing_submission
        {
            // No submission marker exists, so there is no chain truth to
            // reconcile. Preserve the signed authority and its source
            // occupancy without resolving a provider or touching the network.
            return Ok(Some(Ok(QueueExecution::PreparedHeld(reason.into()))));
        }
        let allow_submission = allow_submission && dependency_block_reason.is_none();
        if allow_submission || resume_existing_submission {
            self.persist_queue_submission_marker(queue, job_index, now)?;
        }
        let submitted_job = queue.jobs[job_index].clone();
        let result = self
            .broadcast_prepared_queue_job(
                token,
                &submitted_job,
                resume_existing_submission,
                allow_submission,
            )
            .await
            .map(
                |outcome| match (dependency_block_reason.as_deref(), outcome) {
                    (Some(reason), QueueExecution::SubmittedUnknown(_)) => {
                        QueueExecution::SubmittedUnknown(reason.into())
                    }
                    (_, outcome) => outcome,
                },
            );
        Ok(Some(result))
    }
}
