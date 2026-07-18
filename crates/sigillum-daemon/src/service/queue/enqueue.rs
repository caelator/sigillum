//! Queue job enqueue operations.

use sigillum_api::{
    QueueEnqueueResponse, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJobPayload,
};

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};
use crate::service::helpers::{now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::{authorization, gates, payloads};

impl SigillumService {
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
        if let Some(family) = gates::queue_payload_execution_family(&payload) {
            self.require_execution_family_allowed(family)?;
        }
        let now = now_unix();
        let job = payloads::queued_job(random_id(), now, payload);

        let _guard = self.state.operation_guard().await;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        queue.jobs.push(job.clone());
        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        self.state.publish_queue_job_transition(&job, None);

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
