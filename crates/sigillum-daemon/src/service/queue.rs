//! Asynchronous job queue for transaction and sweep operations.
//!
//! Queues, processes, and tracks Ethereum stealth transfers and deposit
//! sweeps with deferred execution and retry logic.

mod state;

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, EvmProviderRef,
    QueueEnqueueResponse, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJob,
    QueueJobListResponse, QueueJobPayload, QueueProcessRequest, QueueProcessResponse,
    StealthPaymentRef,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};

use state::{QueueFailureDisposition, classify_queue_error, queue_job_is_runnable};
pub(super) use state::{
    count_queue_states, is_active_or_completed_queue_state, is_active_queue_state, queue_status,
    recover_queue_job,
};

use super::helpers::{
    compare_u256, is_zero_u256, map_wallet_error, multiply_u256_u64, now_unix, random_id,
    subtract_u256,
};
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
            QueueJobPayload::EthStealthTransfer {
                wallet_profile: body.wallet_profile,
                stealth_address: body.stealth.stealth_address,
                ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
                value_wei_hex: body.value_wei_hex,
                destination_address: body.destination_address,
                nonce: body.nonce,
                gas_limit: body.gas_limit,
                view_tag_hex: body.stealth.view_tag_hex,
            },
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
            QueueJobPayload::EthStealthErc20Transfer {
                wallet_profile: body.wallet_profile,
                stealth_address: body.stealth.stealth_address,
                ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
                token_address: body.token_address,
                recipient_address: body.recipient_address,
                amount_hex: body.amount_hex,
                nonce: body.nonce,
                gas_limit: body.gas_limit,
                view_tag_hex: body.stealth.view_tag_hex,
            },
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
            QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: body.wallet_profile,
                stealth_address: body.stealth.stealth_address,
                ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
                destination_address: body.destination_address,
                min_value_wei_hex: body.min_value_wei_hex,
                gas_limit: body.gas_limit,
                view_tag_hex: body.stealth.view_tag_hex,
            },
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
            QueueJobPayload::EthStealthErc20Sweep {
                wallet_profile: body.wallet_profile,
                stealth_address: body.stealth.stealth_address,
                ephemeral_public_key_hex: body.stealth.ephemeral_public_key_hex,
                token_address: body.token_address,
                recipient_address: body.recipient_address,
                min_amount_hex: body.min_amount_hex,
                gas_limit: body.gas_limit,
                view_tag_hex: body.stealth.view_tag_hex,
            },
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
        let now = now_unix();
        let job = QueueJob {
            id: random_id(),
            state: QUEUE_STATE_QUEUED.into(),
            attempts: 0,
            created_at_unix: now,
            updated_at_unix: now,
            next_attempt_after_unix: None,
            payload,
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        };

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

    // ── Queue Processing ───────────────────────────────────────────────────

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

    pub(super) async fn process_queue_state(
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

// ── Execution State Types ──────────────────────────────────────────────────

#[allow(clippy::large_enum_variant)]
enum QueueExecution {
    Sent(sigillum_api::EthStealthSendResponse),
    Blocked(String),
}

impl SigillumService {
    // ── Sweep Processing Implementation ────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    async fn process_eth_stealth_native_sweep(
        &self,
        token: &str,
        wallet_profile: &str,
        stealth_address: &str,
        ephemeral_public_key_hex: &str,
        destination_address: Option<String>,
        min_value_wei_hex: Option<&str>,
        gas_limit_override: Option<u64>,
        view_tag_hex: Option<String>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
        let destination_address = destination_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "Native sweep requires destination_address or wallet default destination.",
                )
            })?;
        provider
            .max_priority_fee_per_gas_hex
            .as_ref()
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "provider profile is missing max_priority_fee_per_gas_hex",
                )
            })?;
        let max_fee_per_gas_hex = provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?;
        let gas_limit = gas_limit_override
            .or(provider.native_gas_limit)
            .unwrap_or(21_000);
        let balance = self
            .evm_balance(
                Some(token),
                sigillum_api::EvmRpcBalanceRequest {
                    provider: EvmProviderRef {
                        rpc_url: provider.rpc_url.clone(),
                        auth_token_key: provider.auth_token_key.clone(),
                        compartment_id: Some(provider.compartment_id),
                    },
                    address: stealth_address.to_string(),
                    block_tag: Some("latest".into()),
                },
            )
            .await?;
        let balance_raw =
            decode_quantity_hex(&balance.balance_wei_hex).map_err(map_wallet_error)?;
        let gas_cost = multiply_u256_u64(
            &decode_quantity_hex(&max_fee_per_gas_hex).map_err(map_wallet_error)?,
            gas_limit,
        );
        if compare_u256(&balance_raw, &gas_cost).is_le() {
            return Ok(QueueExecution::Blocked(
                "deposit has insufficient native balance after gas".into(),
            ));
        }
        let spendable = subtract_u256(&balance_raw, &gas_cost);
        if let Some(minimum) = min_value_wei_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&spendable, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "deposit balance has not reached the sweep threshold".into(),
                ));
            }
        }
        let sent = self
            .eth_stealth_send_with_profile(
                Some(token),
                EthStealthSendWithProfileRequest {
                    wallet_profile: wallet_profile.into(),
                    stealth: StealthPaymentRef {
                        stealth_address: stealth_address.into(),
                        ephemeral_public_key_hex: ephemeral_public_key_hex.into(),
                        view_tag_hex,
                    },
                    value_wei_hex: super::evm::encode_quantity_u256(&spendable),
                    destination_address: Some(destination_address),
                    nonce: None,
                    gas_limit: Some(gas_limit),
                    broadcast: Some(true),
                },
            )
            .await?;
        Ok(QueueExecution::Sent(sent))
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_eth_stealth_erc20_sweep(
        &self,
        token: &str,
        wallet_profile: &str,
        stealth_address: &str,
        ephemeral_public_key_hex: &str,
        token_address: &str,
        recipient_address: Option<String>,
        min_amount_hex: Option<&str>,
        gas_limit_override: Option<u64>,
        view_tag_hex: Option<String>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
        let recipient_address = recipient_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "ERC-20 sweep requires recipient_address or wallet default destination.",
                )
            })?;
        let gas_limit = gas_limit_override
            .or(provider.erc20_gas_limit)
            .unwrap_or(65_000);
        let max_fee = provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?;
        let (native_balance_raw, token_balance_raw) = self
            .evm_native_and_erc20_balance_for_provider(
                provider.compartment_id,
                &provider,
                stealth_address,
                token_address,
                "latest",
            )
            .await?;
        let gas_cost = multiply_u256_u64(
            &decode_quantity_hex(&max_fee).map_err(map_wallet_error)?,
            gas_limit,
        );
        if compare_u256(&native_balance_raw, &gas_cost).is_lt() {
            return Ok(QueueExecution::Blocked(
                "deposit lacks native gas for ERC-20 sweep".into(),
            ));
        }

        let amount_hex = super::evm::encode_quantity_u256(&token_balance_raw);
        let amount = decode_quantity_hex(&amount_hex).map_err(map_wallet_error)?;
        if is_zero_u256(&amount) {
            return Ok(QueueExecution::Blocked(
                "deposit has no ERC-20 balance to sweep".into(),
            ));
        }
        if let Some(minimum) = min_amount_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&amount, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "deposit token balance has not reached the sweep threshold".into(),
                ));
            }
        }

        let sent = self
            .eth_stealth_send_erc20_with_profile(
                Some(token),
                EthStealthSendErc20WithProfileRequest {
                    wallet_profile: wallet_profile.into(),
                    stealth: StealthPaymentRef {
                        stealth_address: stealth_address.into(),
                        ephemeral_public_key_hex: ephemeral_public_key_hex.into(),
                        view_tag_hex,
                    },
                    token_address: token_address.into(),
                    recipient_address,
                    amount_hex,
                    nonce: None,
                    gas_limit: Some(gas_limit),
                    broadcast: Some(true),
                },
            )
            .await?;
        Ok(QueueExecution::Sent(sent))
    }
}
