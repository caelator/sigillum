//! Discovery job mutation verbs: real, cooperative cancel and resume.
//!
//! Cancel and resume are precise, distinct operations:
//!
//! - **Cancel** stops in-flight work. For a job driven by a live background
//!   operation (any scan started in this process, sync or `run_async`) it
//!   signals the operation's cooperative cancel flag — never blocking on the
//!   operation mutex the scan holds — and the scan honors the signal at the
//!   next address index, persists its progress exactly like the per-index
//!   saves, and marks the job `canceled` durably. For an orphaned `running`
//!   job (daemon restarted mid-scan, no live operation) the job is marked
//!   `canceled` directly. Canceling a terminal job (`completed`, `canceled`,
//!   `failed`) is a 409 conflict.
//! - **Resume** starts a NEW background operation and discovery job that
//!   continues from the interrupted job's persisted per-wallet/provider
//!   checkpoints (the same `resume_from_latest_checkpoint` machinery a manual
//!   rescan uses). Per-index persistence plus observation upserts keyed on
//!   wallet/provider/chain/address make resume idempotent: already-processed
//!   indices are never re-observed, so no duplicate observations result.

use sigillum_api::{
    DiscoveryJobListResponse, DiscoveryJobMutationRequest, DiscoveryJobMutationResponse,
    WalletDiscoveryJob, WalletInventoryScanRequest,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;
use crate::service::list_query::{
    self, CreatedUpdatedSort, DISCOVERY_JOB_STATES, SortOrder, effective_order, paginate,
    validated_value,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::DISCOVERY_SOURCE_LOCAL_RPC;
use super::support::{load_inventory_state, save_inventory_state};

const STATUS_CANCEL_REQUESTED: &str = "cancel_requested";
const STATUS_CANCELED: &str = "canceled";
const STATUS_RESUMED: &str = "resumed";

impl SigillumService {
    pub(crate) fn list_discovery_jobs(
        &self,
        token: Option<&str>,
        query: list_query::DiscoveryJobListQuery,
    ) -> ServiceResult<DiscoveryJobListResponse> {
        let _ = self.require_session(token)?;
        let state_filter = query
            .state
            .map(|value| validated_value("state", value, &DISCOVERY_JOB_STATES))
            .transpose()?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let mut jobs = state.jobs;
        if let Some(status) = state_filter.as_deref() {
            jobs.retain(|job| job.status == status);
        }
        if let Some(sort) = query.sort {
            let order = effective_order(query.sort.as_ref(), query.order);
            // `updated` has no dedicated field: completion time for terminal
            // jobs, start time for still-running ones.
            let key = |job: &WalletDiscoveryJob| match sort {
                CreatedUpdatedSort::Created => job.started_at_unix,
                CreatedUpdatedSort::Updated => job.completed_at_unix.unwrap_or(job.started_at_unix),
            };
            match order {
                SortOrder::Asc => jobs.sort_by_key(|job| key(job)),
                SortOrder::Desc => jobs.sort_by(|a, b| key(b).cmp(&key(a))),
            }
        }
        let (jobs, pagination) = paginate(jobs, query.page);
        Ok(DiscoveryJobListResponse { jobs, pagination })
    }

    pub(crate) async fn cancel_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        // Signal the live operation first: this must never block on the
        // operation mutex while a scan holds it.
        let signaled = self.state.request_operation_cancel_for_related(&body.id);
        let inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory.jobs.iter().find(|job| job.id == body.id).cloned();

        let Some(job) = job else {
            // The job may not be persisted yet: an accepted async scan only
            // persists its job record once the runner acquires the operation
            // guard. The signal is already latched on the operation, and the
            // runner will persist the job as `canceled` before any provider
            // call; the returned job is a wire-level acknowledgment of that
            // pending cancel, never a persisted record.
            if let Some(operation) = signaled {
                self.record_audit(
                    self.state.active_compartment_id_for(token),
                    AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                        id: body.id.clone(),
                        status: STATUS_CANCEL_REQUESTED.into(),
                    },
                )?;
                return Ok(DiscoveryJobMutationResponse {
                    status: STATUS_CANCEL_REQUESTED.into(),
                    job: pending_cancel_acknowledgment(&body.id),
                    operation: Some(operation),
                });
            }
            return Err(ServiceError::not_found("Discovery job not found."));
        };

        match job.status.as_str() {
            "completed" | "canceled" | "failed" => {
                return Err(ServiceError::conflict(format!(
                    "Discovery job is already {} and cannot be canceled.",
                    job.status
                )));
            }
            _ => {}
        }

        if let Some(operation) = signaled {
            // The running scan persists the durable `canceled` status itself
            // when it honors the signal; writing it here would be clobbered
            // by the scan's next per-index save.
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                    id: body.id.clone(),
                    status: STATUS_CANCEL_REQUESTED.into(),
                },
            )?;
            return Ok(DiscoveryJobMutationResponse {
                status: STATUS_CANCEL_REQUESTED.into(),
                job,
                operation: Some(operation),
            });
        }

        // No live operation: orphaned `running` job (the daemon restarted
        // mid-scan) or a record written before operations existed. Mark it
        // canceled durably so a future resume treats it as interrupted.
        let _guard = self.state.operation_guard().await;
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory
            .jobs
            .iter_mut()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        job.status = STATUS_CANCELED.into();
        job.completed_at_unix = Some(now_unix());
        let job = job.clone();
        save_inventory_state(&self.state.base_dir, &inventory)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                id: job.id.clone(),
                status: STATUS_CANCELED.into(),
            },
        )?;

        Ok(DiscoveryJobMutationResponse {
            status: job.status.clone(),
            job,
            operation: None,
        })
    }

    pub(crate) async fn resume_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        if self.state.running_operation_for_related(&body.id).is_some() {
            return Err(ServiceError::conflict(
                "Discovery job is still running and cannot be resumed.",
            ));
        }
        let inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory
            .jobs
            .iter()
            .find(|job| job.id == body.id)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        match job.status.as_str() {
            // `resume_requested` records predate real resume; a `running`
            // job with no live operation was interrupted by a restart.
            "canceled" | "failed" | "resume_requested" | "running" => {}
            _ => {
                return Err(ServiceError::conflict(format!(
                    "Discovery job is {} and cannot be resumed.",
                    job.status
                )));
            }
        }

        // Rebuild the scan from the interrupted job's scope. Filters are
        // only reapplied when the job covered exactly one family/profile/
        // provider; broader jobs resume across all configured wallets and
        // providers (checkpoints still prevent re-observation).
        let request = WalletInventoryScanRequest {
            wallet_family: single_value(&job.wallet_families),
            wallet_profile: single_value(&job.wallet_profiles),
            provider_profile: single_value(&job.provider_profiles),
            gap_limit: Some(job.gap_limit),
            max_index: Some(job.max_index),
            resume_from_latest_checkpoint: Some(true),
            run_async: Some(true),
            ..Default::default()
        };
        let prepared = self.prepare_evm_scan(token, request)?;
        let (resumed_job, operation) = self.spawn_async_evm_scan(token, prepared);

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                id: job.id.clone(),
                status: STATUS_RESUMED.into(),
            },
        )?;

        Ok(DiscoveryJobMutationResponse {
            status: resumed_job.status.clone(),
            job: resumed_job,
            operation: Some(operation),
        })
    }
}

fn single_value(values: &[String]) -> Option<String> {
    if values.len() == 1 {
        values.first().cloned()
    } else {
        None
    }
}

/// Wire-level acknowledgment returned when a cancel lands before the scan
/// runner persisted the job record. Never stored; the runner persists the
/// real record (as `canceled`) when it starts.
fn pending_cancel_acknowledgment(id: &str) -> WalletDiscoveryJob {
    WalletDiscoveryJob {
        id: id.to_string(),
        status: STATUS_CANCEL_REQUESTED.into(),
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        wallet_families: Vec::new(),
        wallet_profiles: Vec::new(),
        provider_profiles: Vec::new(),
        chain_ids: Vec::new(),
        gap_limit: 0,
        max_index: 0,
        addresses_scanned: 0,
        active_addresses: 0,
        holdings_detected: 0,
        checkpoints: Vec::new(),
        block_cursors: Vec::new(),
        started_at_unix: now_unix(),
        completed_at_unix: None,
        last_error: None,
    }
}
