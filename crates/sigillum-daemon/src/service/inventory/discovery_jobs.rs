use std::sync::Arc;

use sigillum_api::{
    DiscoveryJobListResponse, DiscoveryJobMutationRequest, DiscoveryJobMutationResponse,
    WalletDiscoveryJob,
};

use crate::AppState;
use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::support::{load_inventory_state, save_inventory_state};

const RESUMABLE_JOB_STATUSES: &[&str] = &["canceled", "failed", "interrupted"];

impl SigillumService {
    pub(crate) fn list_discovery_jobs(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<DiscoveryJobListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(DiscoveryJobListResponse { jobs: state.jobs })
    }

    pub(crate) async fn cancel_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        self.state.request_discovery_cancel(&body.id);

        // If no operation currently owns the mutation boundary, this is either
        // a pre-scan/crashed job or a job between requests. Finalize it now.
        if let Some(_guard) = self.state.try_operation_guard() {
            self.require_authenticated_session(Some(token))?;
            let job = finalize_canceled_job(&self.state, &body.id)?;
            self.record_discovery_job_update(token, &job, "canceled")?;
            return Ok(DiscoveryJobMutationResponse {
                status: job.status.clone(),
                job,
            });
        }

        // A live scan owns operation_lock. Confirm the durable job is actually
        // running, return immediately, and let both the scan and a detached
        // fallback race to finalize it at the serialized boundary. The fallback
        // covers handler cancellation/panic: if the scan disappears, it wins
        // the mutex and closes the durable `running` state.
        let state = load_inventory_state(&self.state.base_dir)?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == body.id)
            .ok_or_else(|| {
                self.state.clear_discovery_cancel_request(&body.id);
                ServiceError::not_found("Discovery job not found.")
            })?
            .clone();
        if job.status != "running" {
            self.state.clear_discovery_cancel_request(&body.id);
            return Err(ServiceError::conflict(
                "Only a running discovery job can be canceled.",
            ));
        }

        self.record_discovery_job_update(token, &job, "cancel_requested")?;
        let state = self.state.clone();
        let job_id = body.id;
        tokio::spawn(async move {
            if let Err(error) = finalize_cancel_after_scan(state, job_id.clone()).await {
                tracing::error!(
                    job_id = %job_id,
                    error = %error,
                    "failed to finalize discovery cancellation"
                );
            }
        });

        Ok(DiscoveryJobMutationResponse {
            status: "cancel_requested".into(),
            job,
        })
    }

    pub(crate) async fn resume_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let guard = self.acquire_session_operation(&session_context).await?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        if !RESUMABLE_JOB_STATUSES.contains(&job.status.as_str()) {
            return Err(ServiceError::conflict(
                "Only a canceled, failed, or interrupted discovery job can be resumed.",
            ));
        }
        let mut request = job.scan_request.as_deref().cloned().ok_or_else(|| {
            ServiceError::bad_request(
                "This job predates resumable scan parameters; start a new scan with \
                     resume_from_latest_checkpoint enabled.",
            )
        })?;
        request.resume_from_latest_checkpoint = Some(true);
        let resumed_from_job_id = job.id.clone();
        drop(guard);

        let response = self
            .scan_wallet_inventory_evm_with_origin(Some(token), request, Some(resumed_from_job_id))
            .await?;
        Ok(DiscoveryJobMutationResponse {
            status: response.job.status.clone(),
            job: response.job,
        })
    }

    fn record_discovery_job_update(
        &self,
        token: &str,
        job: &WalletDiscoveryJob,
        status: &str,
    ) -> ServiceResult<()> {
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                id: job.id.clone(),
                status: status.to_string(),
            },
        )
    }
}

fn finalize_canceled_job(state: &AppState, job_id: &str) -> ServiceResult<WalletDiscoveryJob> {
    let mut inventory = load_inventory_state(&state.base_dir)?;
    let job = inventory
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .ok_or_else(|| {
            state.clear_discovery_cancel_request(job_id);
            ServiceError::not_found("Discovery job not found.")
        })?;
    if job.status != "running" {
        state.clear_discovery_cancel_request(job_id);
        return Err(ServiceError::conflict(
            "Only a running discovery job can be canceled.",
        ));
    }
    job.status = "canceled".into();
    job.completed_at_unix = Some(now_unix());
    job.last_error = None;
    let job = job.clone();
    save_inventory_state(&state.base_dir, &inventory)?;
    state.clear_discovery_cancel_request(job_id);
    Ok(job)
}

async fn finalize_cancel_after_scan(state: Arc<AppState>, job_id: String) -> ServiceResult<()> {
    let _guard = state.operation_guard().await;
    if !state.is_discovery_cancel_requested(&job_id) {
        return Ok(());
    }
    match finalize_canceled_job(&state, &job_id) {
        Ok(_) => Ok(()),
        // A scan may have reached another terminal state while the fallback
        // waited. The latch is no longer actionable in that case.
        Err(error) if error.status() == axum::http::StatusCode::CONFLICT => {
            state.clear_discovery_cancel_request(&job_id);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Startup recovery must never expose a stale `running` job when no request can
/// still own it. Preserve its durable checkpoints and make it explicitly
/// resumable.
pub(in crate::service) fn recover_interrupted_discovery_jobs(
    inventory: &mut crate::inventory::WalletInventoryState,
) -> usize {
    let mut recovered = 0usize;
    for job in &mut inventory.jobs {
        if job.status == "running" {
            job.status = "interrupted".into();
            job.completed_at_unix = Some(now_unix());
            job.last_error =
                Some("Daemon restarted before the scan reached a terminal state.".into());
            recovered += 1;
        }
    }
    recovered
}

#[cfg(test)]
mod tests {
    use sigillum_api::WalletDiscoveryJob;

    use super::recover_interrupted_discovery_jobs;

    #[test]
    fn startup_recovery_terminalizes_only_running_discovery_jobs() {
        let mut inventory = crate::inventory::WalletInventoryState {
            jobs: vec![sample_job("running"), sample_job("completed")],
            ..Default::default()
        };

        assert_eq!(recover_interrupted_discovery_jobs(&mut inventory), 1);
        assert_eq!(inventory.jobs[0].status, "interrupted");
        assert!(inventory.jobs[0].completed_at_unix.is_some());
        assert!(inventory.jobs[0].last_error.is_some());
        assert_eq!(inventory.jobs[1].status, "completed");
    }

    fn sample_job(status: &str) -> WalletDiscoveryJob {
        WalletDiscoveryJob {
            id: format!("job-{status}"),
            status: status.into(),
            source: "local-rpc".into(),
            wallet_families: Vec::new(),
            wallet_profiles: Vec::new(),
            provider_profiles: Vec::new(),
            chain_ids: Vec::new(),
            gap_limit: 20,
            max_index: 100,
            addresses_scanned: 0,
            active_addresses: 0,
            holdings_detected: 0,
            checkpoints: Vec::new(),
            block_cursors: Vec::new(),
            started_at_unix: 1,
            completed_at_unix: None,
            last_error: None,
            scan_request: None,
            resumed_from_job_id: None,
        }
    }
}
