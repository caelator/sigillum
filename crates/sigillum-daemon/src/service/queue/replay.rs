use sigillum_api::QueueJob;

use super::QUEUE_STATE_PREPARED;
use super::failure::classify_blocked_queue_reason;
use super::tally::QueueDrainTally;

/// Remove replay-capable transaction material once a job reaches a state that
/// must never resubmit it automatically.
pub(super) fn clear_replay_bytes(job: &mut QueueJob) {
    job.receipt.signed_raw_transaction_hex = None;
}

/// Retain signed authority and its source occupancy while a prerequisite is
/// unresolved. Ordinary `blocked` state would let a sibling consume its nonce.
pub(super) fn hold_prepared(job: &mut QueueJob, reason: String, tally: &mut QueueDrainTally) {
    tally.record_cause(classify_blocked_queue_reason(&reason));
    job.state = QUEUE_STATE_PREPARED.into();
    job.last_error = Some(reason);
    job.next_attempt_after_unix = None;
    tally.blocked += 1;
}
