//! Apply one queue execution outcome or classified error to persisted fields and the drain tally.

use sigillum_api::QueueJob;

use crate::policy::RuntimePolicy;
use crate::service::ServiceError;

use super::failure::{
    QueueFailureCause, QueueFailureDisposition, classify_blocked_queue_reason,
    classify_operator_action_reason, classify_queue_error,
};
use super::replay::{clear_replay_bytes, hold_prepared};
use super::state::mark_job_operator_action_required;
use super::tally::QueueDrainTally;
use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_CONFIRMED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_PREPARED,
    QUEUE_STATE_RETRYING, QUEUE_STATE_SENT, QUEUE_STATE_SUBMITTED_UNKNOWN, QueueExecution,
};

/// Apply the E1/W7.4 state-transition rules and update the drain tally.
pub(super) fn apply(
    job: &mut QueueJob,
    result: Result<QueueExecution, ServiceError>,
    now: u64,
    policy: RuntimePolicy,
    tally: &mut QueueDrainTally,
) {
    match result {
        Ok(QueueExecution::Prepared {
            signed_raw_transaction_hex,
            transaction_hash_hex,
        }) => {
            let payload_hash = match super::broadcast::queue_payload_hash_hex(&job.payload) {
                Ok(hash) => hash,
                Err(reason) => {
                    tally.record_cause(QueueFailureCause::Validation);
                    mark_job_operator_action_required(
                        job,
                        format!("prepared_payload_hash_failed: {reason}"),
                        now,
                    );
                    tally.operator_action_required += 1;
                    return;
                }
            };
            let binding_hash = match super::broadcast::prepared_binding_hash_hex(
                &job.payload,
                &signed_raw_transaction_hex,
            ) {
                Ok(hash) => hash,
                Err(reason) => {
                    tally.record_cause(QueueFailureCause::Validation);
                    mark_job_operator_action_required(
                        job,
                        format!("prepared_binding_hash_failed: {reason}"),
                        now,
                    );
                    tally.operator_action_required += 1;
                    return;
                }
            };
            job.state = QUEUE_STATE_PREPARED.into();
            job.last_error = None;
            job.next_attempt_after_unix = None;
            job.transaction_hash_hex = Some(transaction_hash_hex);
            job.receipt.signed_raw_transaction_hex = Some(signed_raw_transaction_hex);
            job.receipt.prepared_at_unix = Some(now);
            job.receipt.prepared_payload_hash_hex = Some(payload_hash);
            job.receipt.prepared_binding_hash_hex = Some(binding_hash);
        }
        Ok(QueueExecution::PreparedHeld(reason)) => hold_prepared(job, reason, tally),
        Ok(QueueExecution::Broadcasted {
            broadcast_transaction_hash_hex,
        }) => {
            job.state = QUEUE_STATE_SENT.into();
            job.last_error = None;
            job.next_attempt_after_unix = None;
            job.broadcast_transaction_hash_hex = Some(broadcast_transaction_hash_hex);
            clear_replay_bytes(job);
            // Start confirmation timeout from an affirmative provider
            // acceptance, not from the earlier pre-I/O crash marker.
            job.receipt.broadcast_at_unix = Some(now);
            tally.succeeded += 1;
        }
        Ok(QueueExecution::SubmittedUnknown(reason)) => {
            tally.record_cause(QueueFailureCause::ProviderError);
            job.state = QUEUE_STATE_SUBMITTED_UNKNOWN.into();
            job.last_error = Some(reason);
            job.next_attempt_after_unix = Some(now + policy.queue_retry_delay_secs(job.attempts));
            tally.retrying += 1;
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
            clear_replay_bytes(job);
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
            clear_replay_bytes(job);
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
            clear_replay_bytes(job);
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
                clear_replay_bytes(job);
                tally.failed += 1;
            }
        },
    }
}
