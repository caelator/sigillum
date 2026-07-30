//! Maintenance and cleanup tasks for deposits and queue processing.
//!
//! Provides batch operations for refreshing deposit balances and processing
//! queued jobs as a single atomic maintenance transaction.

mod treasury_automation;

use sigillum_api::{MaintenanceRunRequest, MaintenanceRunResponse, QueueProcessRequest};

use crate::audit_log::AuditEventSpec;

use super::{ServiceResult, SigillumService};
use treasury_automation::merge_failure_breakdowns;

impl SigillumService {
    pub(crate) async fn run_maintenance(
        &self,
        token: Option<&str>,
        body: MaintenanceRunRequest,
    ) -> ServiceResult<MaintenanceRunResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let automation = self.run_treasury_automation(token).await?;
        let operation_guard = self.acquire_session_operation(&session_context).await?;
        let mut deposits =
            crate::deposits::load_deposits(&self.state.base_dir).map_err(|error| {
                super::ServiceError::internal(format!("Failed to load deposits: {error}"))
            })?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir).map_err(|error| {
            super::ServiceError::internal(format!("Failed to load queue: {error}"))
        })?;

        let refresh = self
            .refresh_eth_stealth_deposits_state(
                token,
                &mut deposits,
                &mut queue,
                sigillum_api::EthStealthDepositRefreshRequest {
                    id: None,
                    limit: body.deposit_refresh_limit,
                    auto_enqueue: body.auto_enqueue,
                },
            )
            .await?;
        let processed = self
            .process_queue_state(
                token,
                &mut queue,
                QueueProcessRequest {
                    id: None,
                    limit: body.queue_process_limit,
                },
                &operation_guard,
            )
            .await?;
        let _ = super::deposits::sync_eth_stealth_deposits_with_queue(&mut deposits, &queue);

        crate::queue_store::save_queue(&self.state.base_dir, &queue).map_err(|error| {
            super::ServiceError::internal(format!("Failed to save queue: {error}"))
        })?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits).map_err(|error| {
            super::ServiceError::internal(format!("Failed to save deposits: {error}"))
        })?;

        self.record_audit(
            None,
            AuditEventSpec::MaintenanceRun {
                refreshed: refresh.processed,
                detected: refresh.detected,
                queued: refresh.queued,
                processed: processed.processed,
                succeeded: processed.succeeded,
                blocked: processed.blocked,
                retrying: processed.retrying,
                failed: processed.failed,
            },
        )?;

        let mut failures_by_cause = processed.failures_by_cause;
        if let Some(automation) = automation.as_ref() {
            merge_failure_breakdowns(&mut failures_by_cause, &automation.failures);
        }

        Ok(MaintenanceRunResponse {
            status: "ok".into(),
            refreshed: refresh.processed,
            detected: refresh.detected,
            queued: refresh.queued,
            processed: processed.processed,
            succeeded: processed.succeeded,
            blocked: processed.blocked,
            retrying: processed.retrying,
            operator_action_required: processed.operator_action_required,
            failed: processed.failed,
            confirmed: processed.confirmed,
            failures_by_cause,
            treasury_automation: automation.as_ref().map(|outcome| outcome.summary.clone()),
            deposits: deposits.eth_stealth,
            jobs: processed.jobs,
        })
    }
}
