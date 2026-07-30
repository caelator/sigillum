//! Asynchronous job queue for transaction and sweep operations.
//!
//! Queues, processes, and tracks Ethereum stealth transfers and deposit
//! sweeps with deferred execution and retry logic.

mod authorization;
mod broadcast;
mod execution;
mod failpoints;
mod failure;
mod gates;
mod in_flight;
mod outcomes;
mod pause;
mod payloads;
mod plan_steps;
mod processing;
mod replay;
mod seed_sends;
mod serialization;
mod state;
mod status;
mod sweeps;
mod tally;

use sigillum_api::{
    QueueEnqueueResponse, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJobListResponse,
    QueueJobPayload,
};

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};

pub(super) use execution::QueueExecution;

pub(super) use state::{
    is_active_or_completed_queue_state, is_active_queue_state, mark_job_operator_action_required,
    queue_job_failed_state, queue_job_operator_action_required, recover_queue_job,
};
pub(super) use status::{count_queue_states, queue_status};

// W7.2 plan-step enqueue (service/inventory/plan_execution_enqueue.rs) reuses
// the queue domain's gate evaluation and job construction.
pub(in crate::service) use gates::{
    ExecutionFamily, execution_gate_denial, plan_action_execution_family,
};
pub(in crate::service) use payloads::queued_job;

use super::helpers::{now_unix, random_id};
use super::{ServiceError, ServiceResult, SigillumService};

// ── Queue State Constants ──────────────────────────────────────────────────

const QUEUE_STATE_QUEUED: &str = "queued";
const QUEUE_STATE_BLOCKED: &str = "blocked";
const QUEUE_STATE_RETRYING: &str = "retrying";
/// Signed bytes and their locally-derived hash are durably persisted. No
/// network submission has started yet, and this job must never be re-signed.
const QUEUE_STATE_PREPARED: &str = "prepared";
/// A pre-broadcast marker was durably persisted before the RPC call. The
/// network outcome may be unknown after a crash; recovery polls by hash or
/// resubmits the exact prepared bytes.
const QUEUE_STATE_SUBMITTED_UNKNOWN: &str = "submitted_unknown";
const QUEUE_STATE_SENT: &str = "sent";
/// W7.4: new first-class state (schema v4). `sent` means "broadcast,
/// awaiting confirmation"; `confirmed` means the receipt reached the
/// chain's configured `finality_blocks` depth with a SUCCESS status.
/// Genuinely terminal — never revisited by the drain loop.
const QUEUE_STATE_CONFIRMED: &str = "confirmed";
const QUEUE_STATE_FAILED_TERMINAL: &str = "failed_terminal";
const QUEUE_STATE_OPERATOR_ACTION_REQUIRED: &str = "operator_action_required";
const QUEUE_STATE_LEGACY_DEFERRED: &str = "deferred";
const QUEUE_STATE_LEGACY_FAILED: &str = "failed";

// ── Queue Operations: Listing & Enqueuing ─────────────────────────────────

impl SigillumService {
    pub(crate) fn list_queue_jobs(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<QueueJobListResponse> {
        let _ = self.require_session(token)?;
        let queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        Ok(QueueJobListResponse {
            jobs: queue.jobs.into_iter().map(queue_job_for_response).collect(),
        })
    }

    pub(crate) async fn enqueue_eth_stealth_transfer(
        &self,
        token: Option<&str>,
        body: QueueEthStealthTransferRequest,
    ) -> ServiceResult<QueueEnqueueResponse> {
        self.enqueue_job(
            token,
            payloads::eth_stealth_transfer_payload(body),
            AuditQueueJobKind::EthStealthTransfer,
        )
        .await
    }

    pub(crate) async fn enqueue_eth_stealth_erc20_transfer(
        &self,
        token: Option<&str>,
        body: QueueEthStealthErc20TransferRequest,
    ) -> ServiceResult<QueueEnqueueResponse> {
        self.enqueue_job(
            token,
            payloads::eth_stealth_erc20_transfer_payload(body),
            AuditQueueJobKind::EthStealthErc20Transfer,
        )
        .await
    }

    pub(crate) async fn enqueue_eth_stealth_native_sweep(
        &self,
        token: Option<&str>,
        body: QueueEthStealthNativeSweepRequest,
    ) -> ServiceResult<QueueEnqueueResponse> {
        self.enqueue_job(
            token,
            payloads::eth_stealth_native_sweep_payload(body),
            AuditQueueJobKind::EthStealthNativeSweep,
        )
        .await
    }

    pub(crate) async fn enqueue_eth_stealth_erc20_sweep(
        &self,
        token: Option<&str>,
        body: QueueEthStealthErc20SweepRequest,
    ) -> ServiceResult<QueueEnqueueResponse> {
        self.enqueue_job(
            token,
            payloads::eth_stealth_erc20_sweep_payload(body),
            AuditQueueJobKind::EthStealthErc20Sweep,
        )
        .await
    }

    /// Shared enqueue scaffolding: authenticate, build job, persist, and audit.
    ///
    /// All four enqueue endpoints construct their type-specific [`QueueJobPayload`]
    /// and delegate here for the load → push → save → audit cycle.
    async fn enqueue_job(
        &self,
        token: Option<&str>,
        payload: QueueJobPayload,
        audit_kind: AuditQueueJobKind,
    ) -> ServiceResult<QueueEnqueueResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let now = now_unix();
        let job = payloads::queued_job(random_id(), now, payload);

        let _guard = self.acquire_session_operation(&session_context).await?;
        authorization::require_queue_execution_enabled(self, &job.payload)?;
        authorization::authorize_queue_payload_policy(self, &job.payload)?;
        if let Some(family) = gates::queue_payload_execution_family(&job.payload) {
            self.require_execution_family_allowed(family)?;
        }
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        queue.jobs.push(job.clone());
        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::QueueEnqueue {
                id: job.id.clone(),
                job_kind: audit_kind,
            },
        )?;

        Ok(QueueEnqueueResponse {
            status: "queued".into(),
            job,
        })
    }
}

fn queue_job_for_response(mut job: sigillum_api::QueueJob) -> sigillum_api::QueueJob {
    // Signed bytes are intentionally persisted for crash-safe replay, but
    // returning them over list/process/maintenance APIs would grant any API
    // consumer an unnecessary transaction-submission capability.
    job.receipt.signed_raw_transaction_hex = None;
    job
}
