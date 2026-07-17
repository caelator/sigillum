use sigillum_api::{
    DiscoveryJobListResponse, DiscoveryJobMutationRequest, DiscoveryJobMutationResponse,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::support::{load_inventory_state, save_inventory_state};

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
        self.update_discovery_job_status(token, body, "canceled")
            .await
    }

    pub(crate) async fn resume_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        self.update_discovery_job_status(token, body, "resume_requested")
            .await
    }

    async fn update_discovery_job_status(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
        status: &str,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        job.status = status.to_string();
        job.completed_at_unix = Some(now_unix());
        let job = job.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                id: job.id.clone(),
                status: job.status.clone(),
            },
        )?;

        Ok(DiscoveryJobMutationResponse {
            status: job.status.clone(),
            job,
            operation: None,
        })
    }
}
