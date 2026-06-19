//! Asynchronous job queue for transaction and sweep operations.
//!
//! Queues, processes, and tracks Ethereum stealth transfers and deposit
//! sweeps with deferred execution and retry logic.

mod authorization;
mod payloads;
mod processing;
mod state;
mod sweeps;

use sigillum_api::{
    QueueEnqueueResponse, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJobListResponse,
    QueueJobPayload,
};

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};

pub(super) use state::{
    count_queue_states, is_active_or_completed_queue_state, is_active_queue_state, queue_status,
    recover_queue_job,
};

use super::helpers::{now_unix, random_id};
use super::{ServiceError, ServiceResult, SigillumService};

// ── Queue State Constants ──────────────────────────────────────────────────

const QUEUE_STATE_QUEUED: &str = "queued";
const QUEUE_STATE_BLOCKED: &str = "blocked";
const QUEUE_STATE_RETRYING: &str = "retrying";
const QUEUE_STATE_SENT: &str = "sent";
const QUEUE_STATE_FAILED_TERMINAL: &str = "failed_terminal";
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
        Ok(QueueJobListResponse { jobs: queue.jobs })
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
        authorization::require_queue_execution_enabled(self, &payload)?;
        authorization::authorize_queue_payload_policy(self, &payload)?;
        let now = now_unix();
        let job = payloads::queued_job(random_id(), now, payload);

        let _guard = self.state.operation_guard().await;
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

// ── Execution State Types ──────────────────────────────────────────────────

#[allow(clippy::large_enum_variant)]
pub(super) enum QueueExecution {
    Sent(sigillum_api::EthStealthSendResponse),
    Blocked(String),
}
