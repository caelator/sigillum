//! Asynchronous job queue for transaction and sweep operations.
//!
//! Queues, processes, and tracks Ethereum stealth transfers and deposit
//! sweeps with deferred execution and retry logic.

use axum::http::StatusCode;
use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, EvmProviderRef,
    QueueEnqueueResponse, QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueJob,
    QueueJobListResponse, QueueJobPayload, QueueProcessRequest, QueueProcessResponse,
    StealthPaymentRef,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};

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

// ── Queue State Types ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct QueueStateCounts {
    pub blocked: usize,
    pub retrying: usize,
    pub failed: usize,
    pub deferred_legacy: usize,
}

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

enum QueueFailureDisposition {
    Retryable {
        reason: String,
        retry_after_unix: u64,
    },
    FailedTerminal {
        reason: String,
    },
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

// ── Queue State Utilities ──────────────────────────────────────────────────

pub(super) fn count_queue_states(queue: &crate::queue_store::QueueState) -> QueueStateCounts {
    let mut counts = QueueStateCounts::default();
    for job in &queue.jobs {
        match job.state.as_str() {
            QUEUE_STATE_BLOCKED => counts.blocked += 1,
            QUEUE_STATE_RETRYING => counts.retrying += 1,
            QUEUE_STATE_FAILED_TERMINAL | QUEUE_STATE_LEGACY_FAILED => counts.failed += 1,
            QUEUE_STATE_LEGACY_DEFERRED => counts.deferred_legacy += 1,
            _ => {}
        }
    }
    counts
}

pub(super) fn is_active_queue_state(state: &str) -> bool {
    matches!(
        normalize_queue_state(state),
        QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED | QUEUE_STATE_RETRYING
    )
}

pub(super) fn is_active_or_completed_queue_state(state: &str) -> bool {
    is_active_queue_state(state) || normalize_queue_state(state) == QUEUE_STATE_SENT
}

pub(super) fn queue_status(state: &str) -> String {
    match normalize_queue_state(state) {
        QUEUE_STATE_SENT => "sweep_sent",
        QUEUE_STATE_FAILED_TERMINAL => "sweep_failed",
        QUEUE_STATE_BLOCKED => "sweep_blocked",
        QUEUE_STATE_RETRYING => "sweep_retrying",
        QUEUE_STATE_QUEUED => "sweep_queued",
        _ => "funded",
    }
    .into()
}

pub(super) fn normalize_queue_state(state: &str) -> &str {
    match state {
        QUEUE_STATE_LEGACY_DEFERRED => QUEUE_STATE_BLOCKED,
        QUEUE_STATE_LEGACY_FAILED => QUEUE_STATE_FAILED_TERMINAL,
        other => other,
    }
}

pub(super) fn recover_queue_job(job: &mut QueueJob) -> bool {
    let mut changed = false;
    let normalized_state = normalize_queue_state(&job.state);
    if normalized_state != job.state {
        job.state = normalized_state.into();
        changed = true;
    }

    if job.state == QUEUE_STATE_RETRYING {
        if job.next_attempt_after_unix.is_none() {
            job.next_attempt_after_unix = Some(now_unix());
            changed = true;
        }
    } else if job.next_attempt_after_unix.take().is_some() {
        changed = true;
    }

    changed
}

// ── Job Runnable Checks & Error Classification ─────────────────────────────

fn queue_job_is_runnable(job: &QueueJob, force_target: bool, now: u64) -> bool {
    match normalize_queue_state(&job.state) {
        QUEUE_STATE_QUEUED | QUEUE_STATE_BLOCKED => true,
        QUEUE_STATE_RETRYING => {
            force_target || job.next_attempt_after_unix.unwrap_or_default() <= now
        }
        _ => false,
    }
}

fn classify_queue_error(
    error: ServiceError,
    attempts: u32,
    now: u64,
    policy: crate::policy::RuntimePolicy,
) -> QueueFailureDisposition {
    match error.status() {
        StatusCode::INTERNAL_SERVER_ERROR | StatusCode::TOO_MANY_REQUESTS => {
            QueueFailureDisposition::Retryable {
                reason: error.message().to_string(),
                retry_after_unix: now + queue_retry_delay_secs(attempts, policy),
            }
        }
        _ => QueueFailureDisposition::FailedTerminal {
            reason: error.message().to_string(),
        },
    }
}

fn queue_retry_delay_secs(attempts: u32, policy: crate::policy::RuntimePolicy) -> u64 {
    policy.queue_retry_delay_secs(attempts)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sigillum_api::{QueueJob, QueueJobPayload};

    use super::*;

    fn sample_job(state: &str, next_attempt_after_unix: Option<u64>) -> QueueJob {
        QueueJob {
            id: "job-1".into(),
            state: state.into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "profile".into(),
                stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        }
    }

    #[test]
    fn queue_counts_track_new_and_legacy_states() {
        let queue = crate::queue_store::QueueState {
            jobs: vec![
                sample_job(QUEUE_STATE_BLOCKED, None),
                sample_job(QUEUE_STATE_RETRYING, Some(30)),
                sample_job(QUEUE_STATE_FAILED_TERMINAL, None),
                sample_job(QUEUE_STATE_LEGACY_FAILED, None),
                sample_job(QUEUE_STATE_LEGACY_DEFERRED, None),
            ],
        };

        let counts = count_queue_states(&queue);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.retrying, 1);
        assert_eq!(counts.failed, 2);
        assert_eq!(counts.deferred_legacy, 1);
    }

    #[test]
    fn queue_status_normalizes_legacy_states() {
        assert_eq!(queue_status(QUEUE_STATE_QUEUED), "sweep_queued");
        assert_eq!(queue_status(QUEUE_STATE_BLOCKED), "sweep_blocked");
        assert_eq!(queue_status(QUEUE_STATE_RETRYING), "sweep_retrying");
        assert_eq!(queue_status(QUEUE_STATE_SENT), "sweep_sent");
        assert_eq!(queue_status(QUEUE_STATE_FAILED_TERMINAL), "sweep_failed");
        assert_eq!(queue_status(QUEUE_STATE_LEGACY_DEFERRED), "sweep_blocked");
        assert_eq!(queue_status(QUEUE_STATE_LEGACY_FAILED), "sweep_failed");
    }

    #[test]
    fn queue_runnable_rules_respect_retry_deadlines() {
        let queued = sample_job(QUEUE_STATE_QUEUED, None);
        let blocked = sample_job(QUEUE_STATE_BLOCKED, None);
        let retry_due = sample_job(QUEUE_STATE_RETRYING, Some(10));
        let retry_later = sample_job(QUEUE_STATE_RETRYING, Some(20));
        let sent = sample_job(QUEUE_STATE_SENT, None);

        assert!(queue_job_is_runnable(&queued, false, 15));
        assert!(queue_job_is_runnable(&blocked, false, 15));
        assert!(queue_job_is_runnable(&retry_due, false, 15));
        assert!(!queue_job_is_runnable(&retry_later, false, 15));
        assert!(queue_job_is_runnable(&retry_later, true, 15));
        assert!(!queue_job_is_runnable(&sent, false, 15));
    }

    #[test]
    fn retry_delay_uses_bounded_backoff() {
        let policy = crate::policy::RuntimePolicy::default();
        assert_eq!(queue_retry_delay_secs(0, policy), 5);
        assert_eq!(queue_retry_delay_secs(1, policy), 5);
        assert_eq!(queue_retry_delay_secs(2, policy), 10);
        assert_eq!(queue_retry_delay_secs(3, policy), 20);
        assert_eq!(queue_retry_delay_secs(7, policy), 300);
        assert_eq!(queue_retry_delay_secs(100, policy), 300);
    }

    #[test]
    fn queue_error_classification_distinguishes_retryable_failures() {
        let policy = crate::policy::RuntimePolicy::default();
        let retryable = classify_queue_error(ServiceError::internal("temporary"), 2, 100, policy);
        assert!(matches!(
            retryable,
            QueueFailureDisposition::Retryable {
                retry_after_unix: 110,
                ..
            }
        ));

        let throttled =
            classify_queue_error(ServiceError::too_many_requests("slow down"), 1, 50, policy);
        assert!(matches!(
            throttled,
            QueueFailureDisposition::Retryable {
                retry_after_unix: 55,
                ..
            }
        ));

        let terminal = classify_queue_error(ServiceError::bad_request("bad input"), 1, 50, policy);
        assert!(matches!(
            terminal,
            QueueFailureDisposition::FailedTerminal { .. }
        ));
    }

    #[test]
    fn recover_queue_job_normalizes_legacy_states_and_retry_schedule() {
        let mut blocked = sample_job(QUEUE_STATE_LEGACY_DEFERRED, Some(99));
        assert!(recover_queue_job(&mut blocked));
        assert_eq!(blocked.state, QUEUE_STATE_BLOCKED);
        assert_eq!(blocked.next_attempt_after_unix, None);

        let mut retrying = sample_job(QUEUE_STATE_RETRYING, None);
        assert!(recover_queue_job(&mut retrying));
        assert_eq!(retrying.state, QUEUE_STATE_RETRYING);
        assert!(retrying.next_attempt_after_unix.is_some());
    }
}
