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
                SortOrder::Asc => jobs.sort_by_key(&key),
                SortOrder::Desc => jobs.sort_by_key(|job| std::cmp::Reverse(key(job))),
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
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.cancel_discovery_job_with_context(session_context, body)
            .await
    }

    async fn cancel_discovery_job_with_context(
        &self,
        session_context: super::super::SessionOperationContext,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = session_context.token.as_str();
        let inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory.jobs.iter().find(|job| job.id == body.id).cloned();

        let Some(job) = job else {
            // The job may not be persisted yet: an accepted async scan only
            // persists its job record once the runner acquires the operation
            // guard. Signal only in this missing-record case; when a durable
            // record exists, terminal-state validation must happen first.
            if let Some(operation) = self.state.request_operation_cancel_for_related(&body.id) {
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
            "completed" | "canceled" | "failed" | "interrupted" => {
                return Err(ServiceError::conflict(format!(
                    "Discovery job is already {} and cannot be canceled.",
                    job.status
                )));
            }
            _ => {}
        }

        // The durable record is non-terminal, so a live operation can now be
        // signaled without turning a terminal job into a misleading
        // cancel-requested operation.
        if let Some(operation) = self.state.request_operation_cancel_for_related(&body.id) {
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
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory
            .jobs
            .iter_mut()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        if is_terminal_discovery_status(&job.status) {
            return Err(ServiceError::conflict(format!(
                "Discovery job is already {} and cannot be canceled.",
                job.status
            )));
        }
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
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.resume_discovery_job_with_context(session_context, body)
            .await
    }

    async fn resume_discovery_job_with_context(
        &self,
        session_context: super::super::SessionOperationContext,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = session_context.token.as_str();
        // Fast conflict path for a currently active scan. Recheck under the
        // mutation guard below to close the admission race with a new runner.
        if self.state.running_operation_for_related(&body.id).is_some() {
            return Err(ServiceError::conflict(
                "Discovery job is still running and cannot be resumed.",
            ));
        }
        let _guard = self.acquire_session_operation(&session_context).await?;
        if self.state.running_operation_for_related(&body.id).is_some() {
            return Err(ServiceError::conflict(
                "Discovery job is still running and cannot be resumed.",
            ));
        }
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let job = inventory
            .jobs
            .iter_mut()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        // A legacy store can still contain an orphaned `running` record even
        // after startup recovery was introduced. Normalize it durably under
        // the same guard that admits the replacement scan.
        if job.status == "running" {
            job.status = "interrupted".into();
            job.completed_at_unix = Some(now_unix());
            job.last_error = Some("No live operation owned this running discovery job.".into());
            save_inventory_state(&self.state.base_dir, &inventory)?;
        }
        let job = inventory
            .jobs
            .iter()
            .find(|job| job.id == body.id)
            .cloned()
            .expect("job was resolved above");
        match job.status.as_str() {
            // `resume_requested` records predate real resume. Startup
            // recovery and the guarded normalization above turn every
            // orphaned `running` record into `interrupted`.
            "canceled" | "failed" | "interrupted" | "resume_requested" => {}
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
            // Replay the interrupted job's partitioning so the resumed scan
            // keeps the same stable assignment: every remaining address is
            // probed by the same provider that would have served it
            // originally, preserving disjoint per-provider coverage.
            partition_providers: job.partition_providers,
            ..Default::default()
        };
        let prepared = self.prepare_evm_scan(&session_context, request)?;
        let (resumed_job, operation) = self.spawn_async_evm_scan(session_context.clone(), prepared);

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

/// Startup recovery must never expose a stale `running` discovery job when no
/// request or background operation can still own it. Preserve its durable
/// checkpoints and make the record explicitly resumable.
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

fn single_value(values: &[String]) -> Option<String> {
    if values.len() == 1 {
        values.first().cloned()
    } else {
        None
    }
}

fn is_terminal_discovery_status(status: &str) -> bool {
    matches!(status, "completed" | "canceled" | "failed" | "interrupted")
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
        partition_providers: None,
        provider_partition_observations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{DiscoveryJobMutationRequest, WalletDiscoveryJob};
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;

    use super::{
        SigillumService, load_inventory_state, recover_interrupted_discovery_jobs,
        save_inventory_state,
    };

    fn compartment(id: usize, label: &str) -> CompartmentMeta {
        CompartmentMeta {
            id,
            label: label.into(),
            threshold: 1,
            passphrase_mode: None,
        }
    }

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

    #[tokio::test(flavor = "current_thread")]
    async fn orphan_cancel_wait_rejects_revoked_session_without_mutation() {
        let dir = TempDir::new().unwrap();
        let app_state = Arc::new(
            crate::AppState::new(dir.path().to_path_buf()).expect("app state should initialize"),
        );
        app_state.unlock_compartment(0, [1u8; 32], compartment(0, "daily"));
        let session = app_state.create_session(Some(0));
        let service = SigillumService::new(app_state.clone());
        let inventory = crate::inventory::WalletInventoryState {
            jobs: vec![sample_job("running")],
            ..Default::default()
        };
        save_inventory_state(&app_state.base_dir, &inventory).unwrap();
        let session_context = service
            .capture_session_operation_context(Some(&session))
            .unwrap();

        let held_operation = app_state.operation_guard().await;
        let queued_service = service.clone();
        let queued = tokio::spawn(async move {
            queued_service
                .cancel_discovery_job_with_context(
                    session_context,
                    DiscoveryJobMutationRequest {
                        id: "job-running".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        app_state.revoke_session(&session);
        drop(held_operation);

        let error = queued.await.unwrap().unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        let unchanged = load_inventory_state(&app_state.base_dir).unwrap();
        assert_eq!(unchanged.jobs.len(), 1);
        assert_eq!(unchanged.jobs[0].status, "running");
        assert!(unchanged.jobs[0].completed_at_unix.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resume_wait_rejects_lock_latch_without_starting_replacement() {
        let dir = TempDir::new().unwrap();
        let app_state = Arc::new(
            crate::AppState::new(dir.path().to_path_buf()).expect("app state should initialize"),
        );
        app_state.unlock_compartment(0, [1u8; 32], compartment(0, "daily"));
        let session = app_state.create_session(Some(0));
        let service = SigillumService::new(app_state.clone());
        let inventory = crate::inventory::WalletInventoryState {
            jobs: vec![sample_job("canceled")],
            ..Default::default()
        };
        save_inventory_state(&app_state.base_dir, &inventory).unwrap();
        let session_context = service
            .capture_session_operation_context(Some(&session))
            .unwrap();

        let held_operation = app_state.operation_guard().await;
        let queued_service = service.clone();
        let queued = tokio::spawn(async move {
            queued_service
                .resume_discovery_job_with_context(
                    session_context,
                    DiscoveryJobMutationRequest {
                        id: "job-canceled".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(app_state.begin_locking());
        drop(held_operation);

        let error = queued.await.unwrap().unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::LOCKED);
        let unchanged = load_inventory_state(&app_state.base_dir).unwrap();
        assert_eq!(unchanged.jobs.len(), 1);
        assert_eq!(unchanged.jobs[0].status, "canceled");
        assert!(app_state.list_operations(10).is_empty());
        app_state.lock_all();
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
            partition_providers: None,
            provider_partition_observations: Vec::new(),
        }
    }
}
