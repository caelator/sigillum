//! Queue processing loop and execution-result state transitions.

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, QueueJobPayload,
    QueueProcessRequest, QueueProcessResponse, StealthPaymentRef,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::state::{QueueFailureDisposition, classify_queue_error, queue_job_is_runnable};
use super::{
    QUEUE_STATE_BLOCKED, QUEUE_STATE_FAILED_TERMINAL, QUEUE_STATE_RETRYING, QUEUE_STATE_SENT,
    QueueExecution,
};

impl SigillumService {
    pub(crate) async fn process_queue(
        &self,
        token: Option<&str>,
        body: QueueProcessRequest,
    ) -> ServiceResult<QueueProcessResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let processed = self.process_queue_state(token, &mut queue, body).await?;

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::QueueProcess {
                processed: processed.processed,
                succeeded: processed.succeeded,
                blocked: processed.blocked,
                retrying: processed.retrying,
                failed: processed.failed,
            },
        )?;

        Ok(processed)
    }

    pub(in crate::service) async fn process_queue_state(
        &self,
        token: &str,
        queue: &mut crate::queue_store::QueueState,
        body: QueueProcessRequest,
    ) -> ServiceResult<QueueProcessResponse> {
        let policy = self.state.runtime_policy();
        let limit = policy.queue_process_limit(body.limit);
        let force_target = body.id.is_some();
        let mut processed = Vec::new();
        let mut succeeded = 0usize;
        let mut blocked = 0usize;
        let mut retrying = 0usize;
        let mut failed = 0usize;

        for job in queue.jobs.iter_mut() {
            if processed.len() >= limit {
                break;
            }
            if let Some(target_id) = body.id.as_deref() {
                if job.id != target_id {
                    continue;
                }
            }

            let now = now_unix();
            if !queue_job_is_runnable(job, force_target, now) {
                if body.id.is_some() {
                    break;
                }
                continue;
            }

            job.attempts += 1;
            job.updated_at_unix = now;
            job.next_attempt_after_unix = None;

            let result = match &job.payload {
                QueueJobPayload::EthStealthTransfer {
                    wallet_profile,
                    stealth_address,
                    ephemeral_public_key_hex,
                    value_wei_hex,
                    destination_address,
                    nonce,
                    gas_limit,
                    view_tag_hex,
                } => self
                    .eth_stealth_send_with_profile(
                        Some(token),
                        EthStealthSendWithProfileRequest {
                            wallet_profile: wallet_profile.clone(),
                            stealth: StealthPaymentRef {
                                stealth_address: stealth_address.clone(),
                                ephemeral_public_key_hex: ephemeral_public_key_hex.clone(),
                                view_tag_hex: view_tag_hex.clone(),
                            },
                            value_wei_hex: value_wei_hex.clone(),
                            destination_address: destination_address.clone(),
                            nonce: *nonce,
                            gas_limit: *gas_limit,
                            broadcast: Some(true),
                        },
                    )
                    .await
                    .map(QueueExecution::Sent),
                QueueJobPayload::EthStealthErc20Transfer {
                    wallet_profile,
                    stealth_address,
                    ephemeral_public_key_hex,
                    token_address,
                    recipient_address,
                    amount_hex,
                    nonce,
                    gas_limit,
                    view_tag_hex,
                } => self
                    .eth_stealth_send_erc20_with_profile(
                        Some(token),
                        EthStealthSendErc20WithProfileRequest {
                            wallet_profile: wallet_profile.clone(),
                            stealth: StealthPaymentRef {
                                stealth_address: stealth_address.clone(),
                                ephemeral_public_key_hex: ephemeral_public_key_hex.clone(),
                                view_tag_hex: view_tag_hex.clone(),
                            },
                            token_address: token_address.clone(),
                            recipient_address: recipient_address.clone(),
                            amount_hex: amount_hex.clone(),
                            nonce: *nonce,
                            gas_limit: *gas_limit,
                            broadcast: Some(true),
                        },
                    )
                    .await
                    .map(QueueExecution::Sent),
                QueueJobPayload::EthStealthNativeSweep {
                    wallet_profile,
                    stealth_address,
                    ephemeral_public_key_hex,
                    destination_address,
                    min_value_wei_hex,
                    gas_limit,
                    view_tag_hex,
                } => {
                    self.process_eth_stealth_native_sweep(
                        token,
                        wallet_profile,
                        stealth_address,
                        ephemeral_public_key_hex,
                        destination_address.clone(),
                        min_value_wei_hex.as_deref(),
                        *gas_limit,
                        view_tag_hex.clone(),
                    )
                    .await
                }
                QueueJobPayload::EthStealthErc20Sweep {
                    wallet_profile,
                    stealth_address,
                    ephemeral_public_key_hex,
                    token_address,
                    recipient_address,
                    min_amount_hex,
                    gas_limit,
                    view_tag_hex,
                } => {
                    self.process_eth_stealth_erc20_sweep(
                        token,
                        wallet_profile,
                        stealth_address,
                        ephemeral_public_key_hex,
                        token_address,
                        recipient_address.clone(),
                        min_amount_hex.as_deref(),
                        *gas_limit,
                        view_tag_hex.clone(),
                    )
                    .await
                }
                QueueJobPayload::EthSeedTransfer { .. }
                | QueueJobPayload::EthSeedNativeSweep { .. }
                | QueueJobPayload::EthSeedErc20Sweep { .. } => Ok(QueueExecution::Blocked(
                    "seed-wallet queue execution is not enabled yet".into(),
                )),
            };

            match result {
                Ok(QueueExecution::Sent(sent)) => {
                    job.state = QUEUE_STATE_SENT.into();
                    job.last_error = None;
                    job.transaction_hash_hex = Some(sent.transaction_hash_hex);
                    job.broadcast_transaction_hash_hex = sent.broadcast_transaction_hash_hex;
                    succeeded += 1;
                }
                Ok(QueueExecution::Blocked(reason)) => {
                    job.state = QUEUE_STATE_BLOCKED.into();
                    job.last_error = Some(reason);
                    blocked += 1;
                }
                Err(error) => match classify_queue_error(error, job.attempts, now, policy) {
                    QueueFailureDisposition::Retryable {
                        reason,
                        retry_after_unix,
                    } => {
                        job.state = QUEUE_STATE_RETRYING.into();
                        job.last_error = Some(reason);
                        job.next_attempt_after_unix = Some(retry_after_unix);
                        retrying += 1;
                    }
                    QueueFailureDisposition::FailedTerminal { reason } => {
                        job.state = QUEUE_STATE_FAILED_TERMINAL.into();
                        job.last_error = Some(reason);
                        failed += 1;
                    }
                },
            }

            processed.push(job.clone());

            if body.id.is_some() {
                break;
            }
        }

        Ok(QueueProcessResponse {
            processed: processed.len(),
            succeeded,
            blocked,
            retrying,
            failed,
            jobs: processed,
        })
    }
}
