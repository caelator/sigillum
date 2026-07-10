//! Crash-safe submission of queue transactions prepared by any queue family.
//!
//! The drain writes `signed_raw_transaction_hex`, the local transaction hash,
//! and state=`prepared` to durable queue storage before entering this module.
//! It then writes state=`submitted_unknown` before the RPC call. Consequently
//! a restart may query the stored hash or submit the exact same bytes, but it
//! never needs (or is allowed) to derive a signing key again.

use sha3::{Digest, Keccak256};
use sigillum_api::{EvmProviderProfile, QueueJob, QueueJobPayload, WalletPlanStepAction};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::{now_unix, session_fingerprint_hex};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::gates::plan_action_execution_family;
use super::plan_steps::receipts::{BroadcastErrorClass, ReceiptPoll, classify_broadcast_error};
use super::state::normalize_queue_state;
use super::{QUEUE_STATE_SUBMITTED_UNKNOWN, QueueExecution};

impl SigillumService {
    pub(super) fn persist_queue_submission_marker(
        &self,
        queue: &mut crate::queue_store::QueueState,
        job_index: usize,
        now: u64,
    ) -> ServiceResult<()> {
        if queue.jobs[job_index].state != super::QUEUE_STATE_SUBMITTED_UNKNOWN {
            queue.jobs[job_index].state = super::QUEUE_STATE_SUBMITTED_UNKNOWN.into();
            queue.jobs[job_index].updated_at_unix = now;
            queue.jobs[job_index]
                .receipt
                .broadcast_at_unix
                .get_or_insert(now);
            persist_queue(&self.state.base_dir, queue)?;
            super::failpoints::hit(super::failpoints::AFTER_SUBMITTED_UNKNOWN_PERSIST);
        }
        Ok(())
    }

    pub(super) async fn broadcast_prepared_queue_job(
        &self,
        token: &str,
        job: &QueueJob,
        resume_existing_submission: bool,
        allow_submission: bool,
    ) -> ServiceResult<QueueExecution> {
        let raw = job
            .receipt
            .signed_raw_transaction_hex
            .as_deref()
            .ok_or_else(|| {
                ServiceError::internal("Prepared queue job is missing signed_raw_transaction_hex.")
            })?;
        let transaction_hash_hex = job.transaction_hash_hex.as_deref().ok_or_else(|| {
            ServiceError::internal("Prepared queue job is missing transaction_hash_hex.")
        })?;
        if let Some(reason) = prepared_integrity_error(job) {
            return Ok(QueueExecution::OperatorActionRequired(reason));
        }
        let (provider, wallet_compartment_id) = self.queue_provider_for_payload(&job.payload)?;

        if resume_existing_submission
            && normalize_queue_state(&job.state) == QUEUE_STATE_SUBMITTED_UNKNOWN
        {
            if let Some(outcome) = self
                .resume_submitted_queue_job(
                    job,
                    &provider,
                    transaction_hash_hex,
                    job.receipt.broadcast_at_unix.unwrap_or_else(now_unix),
                )
                .await?
            {
                return Ok(outcome);
            }
        }

        // This is the final instruction before external network I/O. The
        // latch is lock-free so a concurrent pause request can trip it while
        // the drain still owns the operation mutex.
        if !allow_submission || self.state.queue_execution_pause_latched() {
            return Ok(if resume_existing_submission {
                QueueExecution::SubmittedUnknown(
                    "submission_held: preserving submitted_unknown until execution is permitted"
                        .into(),
                )
            } else {
                QueueExecution::Prepared {
                    signed_raw_transaction_hex: raw.to_string(),
                    transaction_hash_hex: transaction_hash_hex.to_string(),
                }
            });
        }

        match self
            .evm_broadcast_raw_transaction_for_provider(provider.compartment_id, &provider, raw)
            .await
        {
            Ok(broadcast_transaction_hash_hex) => {
                if !transaction_hashes_equal(&broadcast_transaction_hash_hex, transaction_hash_hex)
                {
                    let reason = format!(
                        "provider_transaction_hash_mismatch: provider returned {broadcast_transaction_hash_hex} for prepared transaction {transaction_hash_hex}; submission outcome is unknown"
                    );
                    self.record_queue_broadcast_failure(
                        token,
                        job,
                        wallet_compartment_id,
                        transaction_hash_hex,
                        &reason,
                    )?;
                    return Ok(QueueExecution::SubmittedUnknown(reason));
                }
                self.record_queue_broadcast_success(
                    token,
                    job,
                    wallet_compartment_id,
                    transaction_hash_hex,
                    &broadcast_transaction_hash_hex,
                )?;
                Ok(QueueExecution::Broadcasted {
                    broadcast_transaction_hash_hex,
                })
            }
            Err(error) => {
                self.record_queue_broadcast_failure(
                    token,
                    job,
                    wallet_compartment_id,
                    transaction_hash_hex,
                    error.message(),
                )?;
                Ok(classify_prepared_broadcast_failure(job, error.message()))
            }
        }
    }

    async fn resume_submitted_queue_job(
        &self,
        job: &QueueJob,
        provider: &EvmProviderProfile,
        transaction_hash_hex: &str,
        broadcast_at_unix: u64,
    ) -> ServiceResult<Option<QueueExecution>> {
        if let QueueJobPayload::PlanStepExecution(payload) = &job.payload {
            return Ok(
                match self
                    .poll_plan_step_receipt(
                        provider,
                        payload.chain_id,
                        transaction_hash_hex,
                        broadcast_at_unix,
                    )
                    .await
                {
                    Ok(ReceiptPoll::Pending) | Err(_) => None,
                    Ok(ReceiptPoll::PartiallyConfirmed {
                        block_number,
                        gas_used_hex,
                        confirmations,
                    }) => Some(QueueExecution::AwaitingConfirmation {
                        block_number: Some(block_number),
                        gas_used_hex: Some(gas_used_hex),
                        confirmations: Some(confirmations),
                    }),
                    Ok(ReceiptPoll::Confirmed {
                        block_number,
                        gas_used_hex,
                        confirmations,
                    }) => Some(QueueExecution::Confirmed {
                        block_number,
                        gas_used_hex,
                        confirmations,
                    }),
                    Ok(ReceiptPoll::Reverted {
                        block_number,
                        gas_used_hex,
                    }) => Some(QueueExecution::RevertedOnChain {
                        reason: format!(
                            "on_chain_revert: transaction {transaction_hash_hex} mined in block {block_number} with a failure status"
                        ),
                        block_number,
                        gas_used_hex,
                    }),
                    // `submitted_unknown` is persisted BEFORE the RPC call.
                    // A crash may therefore mean no submission happened at
                    // all; timeout cannot park the job until exact-byte
                    // resubmission receives an affirmative provider result.
                    Ok(ReceiptPoll::TimedOut) => None,
                },
            );
        }

        match self
            .evm_transaction_receipt_for_provider(
                provider.compartment_id,
                provider,
                transaction_hash_hex,
            )
            .await
        {
            Ok(Some(receipt)) if receipt.status_success => Ok(Some(QueueExecution::Broadcasted {
                broadcast_transaction_hash_hex: transaction_hash_hex.to_string(),
            })),
            Ok(Some(receipt)) => Ok(Some(QueueExecution::RevertedOnChain {
                reason: format!(
                    "on_chain_revert: transaction {transaction_hash_hex} mined in block {} with a failure status",
                    receipt.block_number
                ),
                block_number: receipt.block_number,
                gas_used_hex: receipt.gas_used_hex,
            })),
            // As above, a pre-RPC crash is indistinguishable here. Always
            // allow the caller to resubmit the exact prepared bytes.
            Ok(None) | Err(_) => Ok(None),
        }
    }

    fn queue_provider_for_payload(
        &self,
        payload: &QueueJobPayload,
    ) -> ServiceResult<(EvmProviderProfile, usize)> {
        let wallet_profile = queue_wallet_profile(payload);
        match payload {
            QueueJobPayload::EthStealthTransfer { .. }
            | QueueJobPayload::EthStealthErc20Transfer { .. }
            | QueueJobPayload::EthStealthNativeSweep { .. }
            | QueueJobPayload::EthStealthErc20Sweep { .. } => {
                let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
                Ok((provider, wallet.compartment_id))
            }
            QueueJobPayload::EthSeedTransfer { .. }
            | QueueJobPayload::EthSeedNativeSweep { .. }
            | QueueJobPayload::EthSeedErc20Sweep { .. }
            | QueueJobPayload::PlanStepExecution(_) => {
                let (provider, wallet) = self.resolve_eth_seed_wallet_profile(wallet_profile)?;
                Ok((provider, wallet.compartment_id))
            }
        }
    }

    fn record_queue_broadcast_success(
        &self,
        token: &str,
        job: &QueueJob,
        wallet_compartment_id: usize,
        transaction_hash_hex: &str,
        broadcast_transaction_hash_hex: &str,
    ) -> ServiceResult<()> {
        if let QueueJobPayload::PlanStepExecution(payload) = &job.payload {
            let action_family = plan_action_execution_family(&payload.action)
                .map(|family| family.as_str())
                .unwrap_or("unknown");
            self.record_audit(
                Some(wallet_compartment_id),
                AuditEventSpec::WalletConsolidationPlanStepBroadcast {
                    plan_id: payload.plan_id.clone(),
                    step_id: payload.step_id.clone(),
                    job_id: job.id.clone(),
                    action_family: action_family.into(),
                    transaction_hash_hex: transaction_hash_hex.into(),
                    broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.into(),
                    session_fingerprint_hex: session_fingerprint_hex(token),
                },
            )
        } else {
            self.record_audit(
                Some(wallet_compartment_id),
                AuditEventSpec::EvmBroadcast {
                    transaction_hash_hex: broadcast_transaction_hash_hex.into(),
                },
            )
        }
    }

    fn record_queue_broadcast_failure(
        &self,
        token: &str,
        job: &QueueJob,
        wallet_compartment_id: usize,
        transaction_hash_hex: &str,
        reason: &str,
    ) -> ServiceResult<()> {
        let QueueJobPayload::PlanStepExecution(payload) = &job.payload else {
            return Ok(());
        };
        let action_family = plan_action_execution_family(&payload.action)
            .map(|family| family.as_str())
            .unwrap_or("unknown");
        self.record_audit(
            Some(wallet_compartment_id),
            AuditEventSpec::WalletConsolidationPlanStepBroadcastFailed {
                plan_id: payload.plan_id.clone(),
                step_id: payload.step_id.clone(),
                job_id: job.id.clone(),
                action_family: action_family.into(),
                transaction_hash_hex: transaction_hash_hex.into(),
                reason: reason.into(),
                session_fingerprint_hex: session_fingerprint_hex(token),
            },
        )
    }
}

pub(super) fn persist_queue(
    base_dir: &std::path::Path,
    queue: &crate::queue_store::QueueState,
) -> ServiceResult<()> {
    crate::queue_store::save_queue(base_dir, queue)
        .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))
}

pub(super) fn signed_raw_transaction_hash_hex(raw: &str) -> Result<String, String> {
    let raw = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    let bytes = hex::decode(raw).map_err(|error| error.to_string())?;
    if bytes.is_empty() {
        return Err("signed transaction is empty".into());
    }
    Ok(hex::encode(Keccak256::digest(bytes)))
}

pub(super) fn transaction_hashes_equal(left: &str, right: &str) -> bool {
    fn normalized(value: &str) -> &str {
        value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value)
    }
    normalized(left).eq_ignore_ascii_case(normalized(right))
}

pub(super) fn queue_payload_hash_hex(payload: &QueueJobPayload) -> Result<String, String> {
    let encoded = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    Ok(hex::encode(Keccak256::digest(encoded)))
}

pub(super) fn prepared_binding_hash_hex(
    payload: &QueueJobPayload,
    signed_raw_transaction_hex: &str,
) -> Result<String, String> {
    let payload = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let raw = signed_raw_transaction_hex
        .strip_prefix("0x")
        .or_else(|| signed_raw_transaction_hex.strip_prefix("0X"))
        .unwrap_or(signed_raw_transaction_hex);
    let raw = hex::decode(raw).map_err(|error| error.to_string())?;
    let mut hasher = Keccak256::new();
    hasher.update(b"sigillum.queue.prepared.v1\0");
    hasher.update((payload.len() as u64).to_be_bytes());
    hasher.update(&payload);
    hasher.update((raw.len() as u64).to_be_bytes());
    hasher.update(&raw);
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn prepared_integrity_error(job: &QueueJob) -> Option<String> {
    let Some(raw) = job.receipt.signed_raw_transaction_hex.as_deref() else {
        return Some("prepared_transaction_invalid: signed transaction bytes are missing".into());
    };
    let stored_hash = job.transaction_hash_hex.as_deref().unwrap_or_default();
    let computed_hash = match signed_raw_transaction_hash_hex(raw) {
        Ok(hash) => hash,
        Err(reason) => {
            return Some(format!(
                "prepared_transaction_invalid: signed transaction bytes are invalid: {reason}"
            ));
        }
    };
    if !transaction_hashes_equal(&computed_hash, stored_hash) {
        return Some(format!(
            "prepared_transaction_hash_mismatch: stored hash {stored_hash} does not match the signed bytes ({computed_hash}); refusing submission"
        ));
    }

    let stored_payload_hash = job
        .receipt
        .prepared_payload_hash_hex
        .as_deref()
        .unwrap_or_default();
    let current_payload_hash = match queue_payload_hash_hex(&job.payload) {
        Ok(hash) => hash,
        Err(reason) => {
            return Some(format!(
                "prepared_payload_invalid: failed to hash queue payload: {reason}"
            ));
        }
    };
    if !transaction_hashes_equal(&current_payload_hash, stored_payload_hash) {
        return Some(format!(
            "prepared_payload_hash_mismatch: queue payload changed after transaction preparation; refusing submission for job {}",
            job.id
        ));
    }

    let stored_binding_hash = job
        .receipt
        .prepared_binding_hash_hex
        .as_deref()
        .unwrap_or_default();
    let current_binding_hash = match prepared_binding_hash_hex(&job.payload, raw) {
        Ok(hash) => hash,
        Err(reason) => {
            return Some(format!(
                "prepared_binding_invalid: failed to bind payload to signed bytes: {reason}"
            ));
        }
    };
    if !transaction_hashes_equal(&current_binding_hash, stored_binding_hash) {
        return Some(format!(
            "prepared_binding_hash_mismatch: signed bytes are not bound to the prepared payload for job {}",
            job.id
        ));
    }
    None
}

fn queue_wallet_profile(payload: &QueueJobPayload) -> &str {
    match payload {
        QueueJobPayload::EthStealthTransfer { wallet_profile, .. }
        | QueueJobPayload::EthStealthErc20Transfer { wallet_profile, .. }
        | QueueJobPayload::EthStealthNativeSweep { wallet_profile, .. }
        | QueueJobPayload::EthStealthErc20Sweep { wallet_profile, .. }
        | QueueJobPayload::EthSeedTransfer { wallet_profile, .. }
        | QueueJobPayload::EthSeedNativeSweep { wallet_profile, .. }
        | QueueJobPayload::EthSeedErc20Sweep { wallet_profile, .. } => wallet_profile,
        QueueJobPayload::PlanStepExecution(payload) => &payload.wallet_profile,
    }
}

fn classify_prepared_broadcast_failure(job: &QueueJob, message: &str) -> QueueExecution {
    let is_claim = matches!(
        &job.payload,
        QueueJobPayload::PlanStepExecution(payload)
            if payload.action == WalletPlanStepAction::ClaimReward
    );

    match classify_broadcast_error(message) {
        Some(BroadcastErrorClass::Revert) if is_claim => QueueExecution::OperatorActionRequired(
            format!("claim_execution_failed: deterministic broadcast rejection: {message}"),
        ),
        Some(BroadcastErrorClass::Revert) => QueueExecution::OperatorActionRequired(format!(
            "on_chain_revert: rejected at broadcast; exact prepared transaction was not retried: {message}"
        )),
        Some(BroadcastErrorClass::NonceTooLow) => QueueExecution::OperatorActionRequired(format!(
            "broadcast_rejected: prepared transaction nonce is no longer available and its hash was not observed; refusing to re-sign: {message}"
        )),
        Some(BroadcastErrorClass::Underpriced) => QueueExecution::OperatorActionRequired(format!(
            "broadcast_rejected: prepared transaction is underpriced; an explicit replacement job is required because this job will not be re-signed: {message}"
        )),
        None => QueueExecution::SubmittedUnknown(format!(
            "broadcast_outcome_unknown: {message}; recovery will query {} or resubmit the exact prepared bytes",
            job.transaction_hash_hex
                .as_deref()
                .unwrap_or("the stored hash")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_job(action: WalletPlanStepAction) -> QueueJob {
        QueueJob {
            id: "job-1".into(),
            state: "submitted_unknown".into(),
            attempts: 1,
            created_at_unix: 1,
            updated_at_unix: 1,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::PlanStepExecution(Box::new(
                sigillum_api::PlanStepExecutionPayload {
                    plan_id: "plan-1".into(),
                    step_id: "step-1".into(),
                    chain_id: 1,
                    source_address: "0x1111111111111111111111111111111111111111".into(),
                    derivation_path: "m/44'/60'/0'/0/0".into(),
                    wallet_family: "eth-seed".into(),
                    wallet_profile: "seed".into(),
                    provider_profile: "mainnet".into(),
                    action,
                    asset_kind: "native".into(),
                    asset_address: None,
                    amount_hex: "0x1".into(),
                    destination_address: None,
                    call_label: "native.transfer(value)".into(),
                    call_target_address: "0x2222222222222222222222222222222222222222".into(),
                    call_data_hex: "0x".into(),
                    call_value_wei_hex: Some("0x1".into()),
                    simulation_evidence_hash_hex: "ab".repeat(32),
                    fee_basis: None,
                    max_priority_fee_per_gas_hex: Some("0x1".into()),
                    max_fee_per_gas_hex: Some("0x2".into()),
                    prerequisite_job_ids: Vec::new(),
                },
            )),
            last_error: None,
            transaction_hash_hex: Some("0xabc".into()),
            broadcast_transaction_hash_hex: None,
            receipt: sigillum_api::QueueJobReceipt {
                signed_raw_transaction_hex: Some("0x02dead".into()),
                prepared_at_unix: Some(1),
                ..Default::default()
            },
        }
    }

    #[test]
    fn deterministic_rejections_never_request_resigning() {
        let job = plan_job(WalletPlanStepAction::SweepNative);
        for message in ["nonce too low", "replacement transaction underpriced"] {
            let QueueExecution::OperatorActionRequired(reason) =
                classify_prepared_broadcast_failure(&job, message)
            else {
                panic!("expected operator action for {message}");
            };
            assert!(
                reason.contains("refusing to re-sign") || reason.contains("will not be re-signed")
            );
        }
    }

    #[test]
    fn ambiguous_transport_failure_stays_submitted_unknown() {
        let job = plan_job(WalletPlanStepAction::SweepNative);
        let QueueExecution::SubmittedUnknown(reason) =
            classify_prepared_broadcast_failure(&job, "connection reset by peer")
        else {
            panic!("expected submitted_unknown");
        };
        assert!(reason.contains("exact prepared bytes"));
    }

    #[test]
    fn ambiguous_claim_submission_stays_unknown_for_receipt_lookup() {
        let job = plan_job(WalletPlanStepAction::ClaimReward);
        assert!(matches!(
            classify_prepared_broadcast_failure(&job, "connection reset by peer"),
            QueueExecution::SubmittedUnknown(_)
        ));
    }
}
