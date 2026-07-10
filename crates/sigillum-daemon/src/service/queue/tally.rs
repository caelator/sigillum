use sigillum_api::MaintenanceFailureBreakdown;

use super::failure::QueueFailureCause;

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
    pub(super) fn record_cause(&mut self, cause: QueueFailureCause) {
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
