//! Apply one job's `QueueExecution` outcome (or classified error) to its
//! persisted fields and the drain-cycle tally. Split out of
//! `processing.rs` to keep the drain loop itself readable (house
//! architecture cap) — this is where E1's state-transition rules and W7.4's
//! receipt/failure-cause bookkeeping actually live.

use sigillum_api::{MaintenanceFailureBreakdown, QueueJob};

use crate::policy::RuntimePolicy;
use crate::service::ServiceError;

use super::failure::{
    QueueFailureCause, QueueFailureDisposition, classify_blocked_queue_reason,
    classify_operator_action_reason, classify_queue_error,
};
use super::state::mark_job_operator_action_required;
use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_RETRYING,
    QUEUE_STATE_SENT, QueueExecution,
};

/// Per-drain-call counters, mirroring `QueueProcessResponse`'s fields.
#[derive(Default)]
pub(super) struct QueueDrainTally {
    pub(super) succeeded: usize,
    pub(super) blocked: usize,
    pub(super) retrying: usize,
    pub(super) operator_action_required: usize,
    pub(super) failed: usize,
    pub(super) confirmed: usize,
    pub(super) failures_by_cause: MaintenanceFailureBreakdown,
}

impl QueueDrainTally {
    fn record_cause(&mut self, cause: QueueFailureCause) {
        match cause {
            QueueFailureCause::ProviderError => self.failures_by_cause.provider_error += 1,
            QueueFailureCause::PolicyBlock => self.failures_by_cause.policy_block += 1,
            QueueFailureCause::InsufficientGas => self.failures_by_cause.insufficient_gas += 1,
            QueueFailureCause::Validation => self.failures_by_cause.validation += 1,
            QueueFailureCause::Unknown => self.failures_by_cause.unknown += 1,
            QueueFailureCause::OnChainRevert => self.failures_by_cause.on_chain_revert += 1,
            QueueFailureCause::BroadcastRejected => self.failures_by_cause.broadcast_rejected += 1,
            QueueFailureCause::ReceiptTimeout => self.failures_by_cause.receipt_timeout += 1,
        }
    }
}

/// Mutates `job` in place per the E1/W7.4 state-transition rules and
/// updates `tally` accordingly.
pub(super) fn apply(
    job: &mut QueueJob,
    result: Result<QueueExecution, ServiceError>,
    now: u64,
    policy: RuntimePolicy,
    tally: &mut QueueDrainTally,
) {
    match result {
        Ok(QueueExecution::Sent(sent)) => {
            job.state = QUEUE_STATE_SENT.into();
            job.last_error = None;
            job.transaction_hash_hex = Some(sent.transaction_hash_hex);
            job.broadcast_transaction_hash_hex = sent.broadcast_transaction_hash_hex;
            // W7.4: stamp the wall-clock broadcast time so receipt
            // confirmation (and its timeout budget) can resume correctly,
            // including across a restart (E2). Harmless and unused for
            // non-`PlanStepExecution` job kinds.
            job.receipt.broadcast_at_unix = Some(now);
            tally.succeeded += 1;
        }
        Ok(QueueExecution::Blocked(reason)) => {
            tally.record_cause(classify_blocked_queue_reason(&reason));
            job.state = QUEUE_STATE_BLOCKED.into();
            job.last_error = Some(reason);
            tally.blocked += 1;
        }
        Ok(QueueExecution::OperatorActionRequired(reason)) => {
            // E1: terminal-until-human, never auto-retried (W7.3
            // evidence-hash tamper detection and claim-revert rule; W7.4
            // nonce/fee-bump exhaustion and receipt timeout).
            tally.record_cause(classify_operator_action_reason(&reason));
            mark_job_operator_action_required(job, reason, now);
            tally.operator_action_required += 1;
        }
        Ok(QueueExecution::RevertedOnChain {
            reason,
            block_number,
            gas_used_hex,
        }) => {
            // W7.4: on-chain revert, discovered via receipt polling — never
            // auto-retried, gas/block evidence recorded.
            tally.record_cause(QueueFailureCause::OnChainRevert);
            mark_job_operator_action_required(job, reason, now);
            job.receipt.receipt_block_number = Some(block_number);
            job.receipt.receipt_gas_used_hex = Some(gas_used_hex);
            job.receipt.receipt_status = Some("reverted".into());
            tally.operator_action_required += 1;
        }
        Ok(QueueExecution::Confirmed {
            block_number,
            gas_used_hex,
            confirmations,
        }) => {
            // W7.4: receipt confirmed at the chain's configured finality
            // depth — genuinely terminal, distinct from `sent`
            // (broadcast-only).
            job.state = QUEUE_STATE_CONFIRMED.into();
            job.last_error = None;
            job.receipt.receipt_block_number = Some(block_number);
            job.receipt.receipt_gas_used_hex = Some(gas_used_hex);
            job.receipt.receipt_status = Some("success".into());
            job.receipt.confirmations = Some(confirmations);
            tally.confirmed += 1;
        }
        Ok(QueueExecution::AwaitingConfirmation {
            block_number,
            gas_used_hex,
            confirmations,
        }) => {
            // W7.4: still awaiting confirmation — state stays `sent`
            // (unchanged); record any partial receipt observation for
            // operator visibility. Not a failure: touches no counter.
            job.state = QUEUE_STATE_SENT.into();
            job.last_error = None;
            if let Some(block_number) = block_number {
                job.receipt.receipt_block_number = Some(block_number);
            }
            if let Some(gas_used_hex) = gas_used_hex {
                job.receipt.receipt_gas_used_hex = Some(gas_used_hex);
            }
            if let Some(confirmations) = confirmations {
                job.receipt.confirmations = Some(confirmations);
            }
        }
        Err(error) => match classify_queue_error(error, job.attempts, now, policy) {
            QueueFailureDisposition::Retryable {
                reason,
                retry_after_unix,
                cause,
            } => {
                tally.record_cause(cause);
                job.state = QUEUE_STATE_RETRYING.into();
                job.last_error = Some(reason);
                job.next_attempt_after_unix = Some(retry_after_unix);
                tally.retrying += 1;
            }
            QueueFailureDisposition::FailedTerminal { reason, cause } => {
                tally.record_cause(cause);
                job.state = QUEUE_STATE_FAILED_TERMINAL.into();
                job.last_error = Some(reason);
                tally.failed += 1;
            }
        },
    }
}
