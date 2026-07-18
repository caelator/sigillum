//! Queue payload execution dispatch.

use std::collections::HashMap;

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, QueueJob,
    QueueJobPayload, StealthPaymentRef,
};

use crate::service::{ServiceResult, SigillumService};

use super::QueueExecution;

impl SigillumService {
    #[rustfmt::skip]
    pub(super) async fn dispatch_queue_job(
        &self,
        token: &str,
        job: &QueueJob,
        job_states: &HashMap<String, String>,
    ) -> ServiceResult<QueueExecution> {
        match &job.payload {
            QueueJobPayload::EthStealthTransfer {
                wallet_profile,
                stealth_address,
                ephemeral_public_key_hex,
                value_wei_hex,
                destination_address,
                nonce,
                gas_limit,
                view_tag_hex,
                stealth_hash_convention,
            } => self
                .eth_stealth_send_with_profile(
                    Some(token),
                    EthStealthSendWithProfileRequest {
                        wallet_profile: wallet_profile.clone(),
                        stealth: StealthPaymentRef {
                            stealth_address: stealth_address.clone(),
                            ephemeral_public_key_hex: ephemeral_public_key_hex.clone(),
                            view_tag_hex: view_tag_hex.clone(),
                            stealth_hash_convention: *stealth_hash_convention,
                        },
                        value_wei_hex: value_wei_hex.clone(),
                        destination_address: destination_address.clone(),
                        nonce: *nonce,
                        gas_limit: *gas_limit,
                        estimate_fees: None,
                        broadcast: Some(false),
                    },
                )
                .await
                .map(QueueExecution::prepared_from_send),
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
                stealth_hash_convention,
            } => self
                .eth_stealth_send_erc20_with_profile(
                    Some(token),
                    EthStealthSendErc20WithProfileRequest {
                        wallet_profile: wallet_profile.clone(),
                        stealth: StealthPaymentRef {
                            stealth_address: stealth_address.clone(),
                            ephemeral_public_key_hex: ephemeral_public_key_hex.clone(),
                            view_tag_hex: view_tag_hex.clone(),
                            stealth_hash_convention: *stealth_hash_convention,
                        },
                        token_address: token_address.clone(),
                        recipient_address: recipient_address.clone(),
                        amount_hex: amount_hex.clone(),
                        nonce: *nonce,
                        gas_limit: *gas_limit,
                        estimate_fees: None,
                        broadcast: Some(false),
                    },
                )
                .await
                .map(QueueExecution::prepared_from_send),
            QueueJobPayload::EthStealthNativeSweep {
                wallet_profile,
                stealth_address,
                ephemeral_public_key_hex,
                destination_address,
                min_value_wei_hex,
                gas_limit,
                view_tag_hex,
                stealth_hash_convention,
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
                    *stealth_hash_convention,
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
                stealth_hash_convention,
                prerequisite_job_ids,
            } => {
                // W6.4-style dependency ordering (mirrors the
                // `PlanStepExecution` prerequisite semantics): a sponsor
                // gas top-up must have broadcast before this sweep
                // executes. The sweep's own on-chain gas balance check
                // remains the authoritative gate until the top-up
                // confirms, so the sweep stays `blocked` until gas is
                // actually there.
                if let Some(reason) = super::stealth_gas_topup::sweep_dependency_block_reason(
                    prerequisite_job_ids,
                    job_states,
                ) {
                    Ok(QueueExecution::Blocked(reason))
                } else {
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
                        *stealth_hash_convention,
                    )
                    .await
                }
            }
            // Sponsor gas top-up for a gas-starved stealth deposit: a
            // native transfer from the wallet's derived gas sponsor to
            // the stealth address (see `stealth_gas_topup.rs`).
            QueueJobPayload::EthStealthGasTopup {
                wallet_profile,
                sponsor_address,
                destination_address,
                value_wei_hex,
                gas_limit,
            } => {
                self.process_eth_stealth_gas_topup(
                    wallet_profile,
                    sponsor_address,
                    destination_address,
                    value_wei_hex,
                    *gas_limit,
                )
                .await
            }
            // W7.3: EthSeed* jobs execute once their Sweep-family gate
            // (checked by the processing caller before dispatch) passes. With
            // gates off the caller's `execution_gate_block_reason` check
            // short-circuits before this arm is ever reached, so the
            // byte-identical "not enabled yet" wording no longer applies
            // here — it is superseded by the gate's own denial reason.
            QueueJobPayload::EthSeedTransfer {
                wallet_profile,
                address,
                derivation_path,
                value_wei_hex,
                destination_address,
                nonce,
                gas_limit,
            } => self
                .process_eth_seed_transfer(
                    wallet_profile,
                    address,
                    derivation_path,
                    value_wei_hex,
                    destination_address,
                    *nonce,
                    *gas_limit,
                )
                .await,
            QueueJobPayload::EthSeedNativeSweep {
                wallet_profile,
                address,
                derivation_path,
                destination_address,
                min_value_wei_hex,
                gas_limit,
            } => self
                .process_eth_seed_native_sweep(
                    wallet_profile,
                    address,
                    derivation_path,
                    destination_address.clone(),
                    min_value_wei_hex.as_deref(),
                    *gas_limit,
                )
                .await,
            QueueJobPayload::EthSeedErc20Sweep {
                wallet_profile,
                address,
                derivation_path,
                token_address,
                recipient_address,
                min_amount_hex,
                gas_limit,
            } => self
                .process_eth_seed_erc20_sweep(
                    wallet_profile,
                    address,
                    derivation_path,
                    token_address,
                    recipient_address.clone(),
                    min_amount_hex.as_deref(),
                    *gas_limit,
                )
                .await,
            // W7.3/W7.4: plan-step jobs execute once their action-family gate
            // (checked by the processing caller) passes; see `plan_steps.rs` for the
            // pre-signing guard chain (dependency ordering, evidence-hash
            // re-verification, signer resolution, fee cap) that runs
            // before any key material is touched, and the resume path
            // (no re-sign/re-broadcast) for a job already `sent`.
            QueueJobPayload::PlanStepExecution(step_payload) => {
                self.process_plan_step_execution(
                    token,
                    &job.id,
                    &job.state,
                    job.transaction_hash_hex.as_deref(),
                    job.receipt.broadcast_at_unix,
                    step_payload,
                    job_states,
                )
                .await
            }
        }
    }
}
