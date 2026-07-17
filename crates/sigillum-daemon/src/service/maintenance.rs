//! Maintenance and cleanup tasks for deposits and queue processing.
//!
//! Provides batch operations for refreshing deposit balances and processing
//! queued jobs as a single atomic maintenance transaction.

mod treasury_automation;

use sigillum_api::{
    MaintenanceRunRequest, MaintenanceRunResponse, OPERATION_KIND_MAINTENANCE_RUN,
    OPERATION_STATE_CANCELED, OPERATION_STATE_COMPLETED, OPERATION_STATE_FAILED,
    QueueProcessRequest,
};

use crate::audit_log::AuditEventSpec;
use crate::operation_registry::OperationHandle;

use super::{ServiceResult, SigillumService};
use treasury_automation::merge_failure_breakdowns;

/// Maintenance cycle stages, in execution order. Encoded in the operation's
/// `related_ids` as `stage:<name>` markers (same order); the operation's
/// `progress.processed` counts completed stages and `progress.total` is
/// [`MAINTENANCE_STAGES`]`::len()`. Cancellation is honored BETWEEN stages:
/// a canceled cycle stops before the next stage with every completed
/// stage's effects durably persisted (automation persists its own plans;
/// the deposit refresh is saved before the cycle returns).
const MAINTENANCE_STAGES: [&str; 4] = [
    "treasury_automation",
    "deposit_refresh",
    "one_time_receive",
    "queue_drain",
];

/// `MaintenanceRunResponse::status` value when the cycle stopped early
/// because its tracking operation was canceled between stages. Previously
/// impossible (only `"ok"` existed), so existing clients are unaffected.
const MAINTENANCE_STATUS_CANCELED: &str = "canceled";

fn stage_related_ids() -> Vec<String> {
    MAINTENANCE_STAGES
        .iter()
        .map(|stage| format!("stage:{stage}"))
        .collect()
}

impl SigillumService {
    /// Register a `maintenance_run` operation. The stage encoding is static,
    /// so both the `stage:<name>` related ids and the progress total are
    /// complete from registration (unlike a queue drain, whose total is
    /// only knowable once the queue is loaded under the guard).
    fn start_maintenance_operation(&self) -> OperationHandle {
        let operation = self
            .state
            .start_operation(OPERATION_KIND_MAINTENANCE_RUN, stage_related_ids());
        self.state
            .operation_set_progress_total(operation.id(), MAINTENANCE_STAGES.len() as u64);
        operation
    }

    /// Run a batch maintenance cycle.
    ///
    /// Both the synchronous and `run_async` paths share one pipeline: the
    /// request is authenticated synchronously up front (so async
    /// submissions fail fast on bad input), and
    /// [`Self::execute_maintenance_run`] drives the stages under the
    /// operation guard with per-stage progress and cooperative cancellation
    /// between stages.
    pub(crate) async fn run_maintenance(
        &self,
        token: Option<&str>,
        body: MaintenanceRunRequest,
    ) -> ServiceResult<MaintenanceRunResponse> {
        let token = self.require_session(token)?;
        if body.run_async == Some(true) {
            let operation = self.spawn_async_maintenance_run(token, body);
            return Ok(MaintenanceRunResponse {
                status: "accepted".into(),
                refreshed: 0,
                detected: 0,
                queued: 0,
                processed: 0,
                succeeded: 0,
                blocked: 0,
                retrying: 0,
                operator_action_required: 0,
                failed: 0,
                confirmed: 0,
                failures_by_cause: Default::default(),
                treasury_automation: None,
                one_time_receive: None,
                deposits: Vec::new(),
                jobs: Vec::new(),
                operation: Some(operation),
            });
        }
        // Synchronous path: identical behavior to the historical endpoint,
        // including the response contract (no `operation` field). The cycle
        // is still registered as an operation so other clients can observe
        // or cancel it mid-run.
        let operation = self.start_maintenance_operation();
        self.execute_maintenance_run(token, body, operation).await
    }

    /// Spawn a maintenance cycle as a background daemon operation, returning
    /// the operation tracking it.
    fn spawn_async_maintenance_run(
        &self,
        token: &str,
        body: MaintenanceRunRequest,
    ) -> sigillum_api::Operation {
        let operation = self.start_maintenance_operation();
        let operation_id = operation.id().to_string();
        let service = self.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            if let Err(error) = service
                .execute_maintenance_run(&token, body, operation)
                .await
            {
                tracing::warn!(error = %error, "async maintenance run failed");
            }
        });
        self.state
            .get_operation(&operation_id)
            .expect("operation registered above")
    }

    /// Execute a maintenance cycle under the operation guard.
    ///
    /// The guard is held for the whole run exactly like the historical
    /// synchronous path, so mutation-serialization semantics are unchanged.
    /// Cancellation is cooperative and honored BETWEEN stages: the in-flight
    /// stage finishes; the cycle then stops with completed stages' effects
    /// persisted and the operation marked `canceled`. Every error exit marks
    /// the operation `failed` instead of leaking a permanently `running`
    /// record.
    async fn execute_maintenance_run(
        &self,
        token: &str,
        body: MaintenanceRunRequest,
        operation: OperationHandle,
    ) -> ServiceResult<MaintenanceRunResponse> {
        let result = self
            .execute_maintenance_run_inner(token, body, &operation)
            .await;
        if let Err(error) = &result {
            self.state.finish_operation(
                operation.id(),
                OPERATION_STATE_FAILED,
                Some(error.message().to_string()),
            );
        }
        result
    }

    async fn execute_maintenance_run_inner(
        &self,
        token: &str,
        body: MaintenanceRunRequest,
        operation: &OperationHandle,
    ) -> ServiceResult<MaintenanceRunResponse> {
        // Stage 1 — treasury automation (runs before the guard, exactly like
        // the historical synchronous path). Cancel checkpoint: the stage
        // boundary BEFORE any stage work.
        if operation.cancellation_requested() {
            self.state
                .finish_operation(operation.id(), OPERATION_STATE_CANCELED, None);
            return Ok(self.canceled_maintenance_response(None, None, None, None, Vec::new()));
        }
        let automation = self.run_treasury_automation(token).await?;
        self.state.operation_set_progress(operation.id(), 1);

        // Stage 2 — deposit refresh (+ optional auto-enqueue). Cancel
        // checkpoint: automation's effects are already durable; nothing of
        // this cycle's refresh/drain has started.
        if operation.cancellation_requested() {
            self.state
                .finish_operation(operation.id(), OPERATION_STATE_CANCELED, None);
            return Ok(self.canceled_maintenance_response(
                automation,
                None,
                None,
                None,
                Vec::new(),
            ));
        }

        let _guard = self.state.operation_guard().await;
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
        self.state.operation_set_progress(operation.id(), 2);

        // Stage 3 — one-time receive lifecycle (plan task 3.3). Cancel
        // checkpoint: the refresh's effects are durable; nothing of this
        // stage's settle/observe/enqueue or the drain has started.
        if operation.cancellation_requested() {
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
                    processed: 0,
                    succeeded: 0,
                    blocked: 0,
                    retrying: 0,
                    failed: 0,
                },
            )?;
            self.state
                .finish_operation(operation.id(), OPERATION_STATE_CANCELED, None);
            return Ok(self.canceled_maintenance_response(
                automation,
                Some((refresh.processed, refresh.detected, refresh.queued)),
                None,
                None,
                deposits.eth_stealth,
            ));
        }
        // Settle confirmed sweeps (retire + optional purge), observe balances
        // (a manual maintenance run always observes), enqueue due sweeps —
        // the drain below picks up whatever was enqueued, under the same
        // gates and durable barriers as operator-enqueued jobs.
        let one_time = self
            .advance_one_time_receive_allocations_state(token, &mut queue, true)
            .await?;
        self.state.operation_set_progress(operation.id(), 3);

        // Stage 4 — queue drain. Cancel checkpoint: the refresh's effects
        // (including auto-enqueued jobs) are durably saved before returning,
        // exactly like the completed path. The drain itself is NOT canceled
        // mid-run here (maintenance cancel is a between-stages boundary);
        // its own kill-switch checks still apply, and a standalone
        // `queue_process` operation honors cancel between jobs.
        if operation.cancellation_requested() {
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
                    processed: 0,
                    succeeded: 0,
                    blocked: 0,
                    retrying: 0,
                    failed: 0,
                },
            )?;
            self.state
                .finish_operation(operation.id(), OPERATION_STATE_CANCELED, None);
            return Ok(self.canceled_maintenance_response(
                automation,
                Some((refresh.processed, refresh.detected, refresh.queued)),
                one_time.tracked.then(|| one_time.summary()),
                None,
                deposits.eth_stealth,
            ));
        }

        let processed = self
            .process_queue_state(
                token,
                &mut queue,
                QueueProcessRequest {
                    id: None,
                    limit: body.queue_process_limit,
                    run_async: None,
                },
                None,
            )
            .await?;
        self.state.operation_set_progress(operation.id(), 4);
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

        self.state
            .finish_operation(operation.id(), OPERATION_STATE_COMPLETED, None);

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
            one_time_receive: one_time.tracked.then(|| one_time.summary()),
            deposits: deposits.eth_stealth,
            jobs: processed.jobs,
            operation: None,
        })
    }

    /// Build the partial-cycle response returned when a cancel is honored
    /// between stages: counts reflect only the stages that ran.
    fn canceled_maintenance_response(
        &self,
        automation: Option<treasury_automation::TreasuryAutomationOutcome>,
        refresh: Option<(usize, usize, usize)>,
        one_time: Option<sigillum_api::OneTimeReceiveRunSummary>,
        processed: Option<sigillum_api::QueueProcessResponse>,
        deposits: Vec<sigillum_api::EthStealthDeposit>,
    ) -> MaintenanceRunResponse {
        let (refreshed, detected, queued) = refresh.unwrap_or_default();
        let processed = processed.unwrap_or(sigillum_api::QueueProcessResponse {
            processed: 0,
            succeeded: 0,
            blocked: 0,
            retrying: 0,
            operator_action_required: 0,
            failed: 0,
            confirmed: 0,
            failures_by_cause: Default::default(),
            paused_reason: None,
            jobs: Vec::new(),
            operation: None,
        });
        let mut failures_by_cause = processed.failures_by_cause.clone();
        if let Some(automation) = automation.as_ref() {
            merge_failure_breakdowns(&mut failures_by_cause, &automation.failures);
        }
        MaintenanceRunResponse {
            status: MAINTENANCE_STATUS_CANCELED.into(),
            refreshed,
            detected,
            queued,
            processed: processed.processed,
            succeeded: processed.succeeded,
            blocked: processed.blocked,
            retrying: processed.retrying,
            operator_action_required: processed.operator_action_required,
            failed: processed.failed,
            confirmed: processed.confirmed,
            failures_by_cause,
            treasury_automation: automation.as_ref().map(|outcome| outcome.summary.clone()),
            one_time_receive: one_time,
            deposits,
            jobs: processed.jobs,
            operation: None,
        }
    }
}
