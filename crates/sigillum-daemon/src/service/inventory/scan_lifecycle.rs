//! Lifecycle checkpoints and fail-closed terminalization for discovery scans.

use super::{load_inventory_state, save_inventory_state};
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    /// Revalidate the live scan after each provider await and before every
    /// checkpoint write. The scan holds `operation_lock`, so revocation and
    /// compartment switches cannot overtake it; the explicit lock-latch check
    /// closes the one preemptive path that intentionally lives outside that
    /// mutex.
    pub(super) fn discovery_scan_checkpoint(
        &self,
        _token: &str,
        job_id: &str,
    ) -> ServiceResult<bool> {
        if self.state.is_locking() {
            return Err(ServiceError::locked(
                "Daemon began locking while the discovery scan was running.",
            ));
        }
        Ok(self.state.is_discovery_cancel_requested(job_id))
    }

    /// Terminalize from the last authorized durable snapshot. In-memory
    /// observations produced by the failing/inadmissible provider step are
    /// deliberately discarded.
    pub(super) fn finalize_failed_discovery_scan(
        &self,
        job_id: &str,
        message: &str,
    ) -> ServiceResult<()> {
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory
            .jobs
            .iter_mut()
            .find(|job| job.id == job_id)
            .ok_or_else(|| ServiceError::internal("Running discovery job disappeared."))?;
        if job.status == "running" {
            job.status = "failed".into();
            job.completed_at_unix = Some(now_unix());
            job.last_error = Some(message.chars().take(512).collect());
            save_inventory_state(&self.state.base_dir, &inventory)?;
        }
        self.state.clear_discovery_cancel_request(job_id);
        Ok(())
    }
}
