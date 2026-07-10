use sigillum_api::QueueJob;

/// Remove replay-capable transaction material once a job reaches a state that
/// must never resubmit it automatically.
pub(super) fn clear_replay_bytes(job: &mut QueueJob) {
    job.receipt.signed_raw_transaction_hex = None;
}
