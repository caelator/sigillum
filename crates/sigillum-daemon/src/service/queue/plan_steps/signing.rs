//! Sign + broadcast the crypto boundary for a `PlanStepExecution` job that
//! has already passed every pre-signing guard in `plan_steps.rs`. The
//! prepared call (target, calldata, value) is taken from the job's payload
//! VERBATIM — never rebuilt here; only gas limit, fee basis, and nonce are
//! resolved at execution time.
//!
//! W7.4 failure classes (all EXCEPT `ClaimReward`, which keeps W7.3's
//! "never retry ANY broadcast failure" rule — a Merkle proof may be
//! consumed by a single attempt): `nonce too low` re-fetches the nonce
//! ONCE and retries; underpriced/replacement-underpriced bumps the fee
//! ONCE within the policy cap (or a conservative ceiling when uncapped)
//! and retries; either repeat, or a revert rejected at broadcast time
//! (never retried, even once), parks to `operator_action_required` —
//! generalizing the W7.3 claim-only revert rule to every family. Any
//! other error bubbles up for the drain loop's existing provider-error
//! retry-with-backoff, exactly as before W7.4. At most two sign+broadcast
//! attempts ever happen; the signing key is dropped (zeroized) the
//! instant no further signing can occur on any exit path.

use k256::ecdsa::SigningKey;
use sigillum_api::{
    ConsolidationPlanStep, EthStealthSendResponse, EvmProviderProfile, PlanStepExecutionPayload,
    WalletPlanStepAction,
};
use sigillum_core::{EthereumEip1559Call, decode_quantity_hex, sign_ethereum_call};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::map_wallet_error;
use crate::service::inventory::zero_value_transaction_gas_limit;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::receipts::{BroadcastErrorClass, bump_fees_within_cap, classify_broadcast_error};

impl SigillumService {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn sign_and_broadcast_plan_step(
        &self,
        job_id: &str,
        payload: &PlanStepExecutionPayload,
        action_family: &str,
        provider: &EvmProviderProfile,
        wallet_compartment_id: usize,
        step: &ConsolidationPlanStep,
        signing_key: SigningKey,
        session_fingerprint_hex: &str,
        fee_cap: Option<[u8; 32]>,
    ) -> ServiceResult<super::QueueExecution> {
        let gas_limit = plan_step_gas_limit(payload, provider, step);
        let mut max_priority_fee_per_gas = resolve_fee_component(
            payload.max_priority_fee_per_gas_hex.as_deref(),
            provider.max_priority_fee_per_gas_hex.as_deref(),
            "max_priority_fee_per_gas_hex",
        )?;
        let mut max_fee_per_gas = resolve_fee_component(
            payload.max_fee_per_gas_hex.as_deref(),
            provider.max_fee_per_gas_hex.as_deref(),
            "max_fee_per_gas_hex",
        )?;
        let value = match payload.call_value_wei_hex.as_deref() {
            Some(value_hex) => decode_quantity_hex(value_hex).map_err(map_wallet_error)?,
            None => [0u8; 32],
        };
        let data = decode_prefixed_hex(&payload.call_data_hex, "call_data_hex")?;
        let mut nonce = self
            .evm_transaction_count_for_provider(
                provider.compartment_id,
                provider,
                &payload.source_address,
                "pending",
            )
            .await?;

        // Claims never retry (W7.3 rule, preserved exactly): a Merkle proof
        // may already be consumed by a single broadcast attempt. Every
        // other family gets the W7.4 bounded retry semantics below.
        let retry_eligible = payload.action != WalletPlanStepAction::ClaimReward;
        let mut nonce_retried = false;
        let mut fee_bumped = false;

        loop {
            let call = EthereumEip1559Call {
                chain_id: payload.chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit,
                to_address: payload.call_target_address.clone(),
                value,
                data: data.clone(),
            };

            let signed = sign_ethereum_call(&signing_key, &call).map_err(map_wallet_error)?;

            self.record_audit(
                Some(wallet_compartment_id),
                AuditEventSpec::WalletConsolidationPlanStepSign {
                    plan_id: payload.plan_id.clone(),
                    step_id: payload.step_id.clone(),
                    job_id: job_id.to_string(),
                    action_family: action_family.to_string(),
                    source_address: payload.source_address.clone(),
                    transaction_hash_hex: signed.transaction_hash_hex.clone(),
                    session_fingerprint_hex: session_fingerprint_hex.to_string(),
                },
            )?;

            match self
                .evm_broadcast_raw_transaction_for_provider(
                    provider.compartment_id,
                    provider,
                    &signed.raw_transaction_hex,
                )
                .await
            {
                Ok(broadcast_transaction_hash_hex) => {
                    // No further signing on this path — zeroize now.
                    drop(signing_key);
                    self.record_audit(
                        Some(wallet_compartment_id),
                        AuditEventSpec::WalletConsolidationPlanStepBroadcast {
                            plan_id: payload.plan_id.clone(),
                            step_id: payload.step_id.clone(),
                            job_id: job_id.to_string(),
                            action_family: action_family.to_string(),
                            transaction_hash_hex: signed.transaction_hash_hex.clone(),
                            broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.clone(),
                            session_fingerprint_hex: session_fingerprint_hex.to_string(),
                        },
                    )?;
                    return Ok(super::QueueExecution::Sent(EthStealthSendResponse {
                        wallet: payload.wallet_profile.clone(),
                        kind: signed.kind,
                        chain_id: signed.chain_id,
                        nonce: signed.nonce,
                        from_address: signed.from_address,
                        to_address: signed.to_address,
                        value_hex: signed.value_hex,
                        data_hex: signed.data_hex,
                        raw_transaction_hex: signed.raw_transaction_hex,
                        transaction_hash_hex: signed.transaction_hash_hex,
                        broadcast: true,
                        broadcast_transaction_hash_hex: Some(broadcast_transaction_hash_hex),
                    }));
                }
                Err(error) => {
                    self.record_audit(
                        Some(wallet_compartment_id),
                        AuditEventSpec::WalletConsolidationPlanStepBroadcastFailed {
                            plan_id: payload.plan_id.clone(),
                            step_id: payload.step_id.clone(),
                            job_id: job_id.to_string(),
                            action_family: action_family.to_string(),
                            transaction_hash_hex: signed.transaction_hash_hex.clone(),
                            reason: error.message().to_string(),
                            session_fingerprint_hex: session_fingerprint_hex.to_string(),
                        },
                    )?;

                    if retry_eligible {
                        if let Some(class) = classify_broadcast_error(error.message()) {
                            match class {
                                BroadcastErrorClass::NonceTooLow if !nonce_retried => {
                                    nonce_retried = true;
                                    nonce = self
                                        .evm_transaction_count_for_provider(
                                            provider.compartment_id,
                                            provider,
                                            &payload.source_address,
                                            "pending",
                                        )
                                        .await?;
                                    continue;
                                }
                                BroadcastErrorClass::Underpriced if !fee_bumped => {
                                    if let Some((bumped_fee, bumped_priority)) =
                                        bump_fees_within_cap(
                                            max_fee_per_gas,
                                            max_priority_fee_per_gas,
                                            fee_cap,
                                        )
                                    {
                                        fee_bumped = true;
                                        max_fee_per_gas = bumped_fee;
                                        max_priority_fee_per_gas = bumped_priority;
                                        continue;
                                    }
                                    // No room left to bump: fall through and park.
                                }
                                _ => {}
                            }
                            drop(signing_key);
                            let message = error.message();
                            let reason = match class {
                                BroadcastErrorClass::Revert => {
                                    format!(
                                        "on_chain_revert: rejected at broadcast (never retried); {message}"
                                    )
                                }
                                BroadcastErrorClass::NonceTooLow => {
                                    format!(
                                        "broadcast_rejected: nonce too low again after one re-fetch retry; {message}"
                                    )
                                }
                                BroadcastErrorClass::Underpriced => {
                                    format!(
                                        "broadcast_rejected: underpriced again after the one allowed fee bump; {message}"
                                    )
                                }
                            };
                            return Ok(super::QueueExecution::OperatorActionRequired(reason));
                        }
                    }
                    drop(signing_key);
                    return Err(error);
                }
            }
        }
    }
}

/// The gas limit assumed at simulation time (W6.2), reused verbatim so
/// execution never diverges from what was validated against the wallet's
/// native balance when the step was simulated.
fn plan_step_gas_limit(
    payload: &PlanStepExecutionPayload,
    provider: &EvmProviderProfile,
    step: &ConsolidationPlanStep,
) -> u64 {
    match payload.action {
        WalletPlanStepAction::SweepNative | WalletPlanStepAction::FundGas => {
            provider.native_gas_limit.unwrap_or(21_000)
        }
        _ => zero_value_transaction_gas_limit(provider, step),
    }
}

/// Prefer the fee basis recorded on the job at enqueue time (W6.2 simulation
/// evidence); fall back to the provider profile's static fee fields only
/// when the job did not record one.
fn resolve_fee_component(
    payload_value: Option<&str>,
    provider_value: Option<&str>,
    field: &str,
) -> ServiceResult<[u8; 32]> {
    let hex = payload_value.or(provider_value).ok_or_else(|| {
        ServiceError::bad_request(format!(
            "plan step job carries no {field} and the provider profile has none configured"
        ))
    })?;
    decode_quantity_hex(hex).map_err(map_wallet_error)
}

fn decode_prefixed_hex(value: &str, label: &str) -> ServiceResult<Vec<u8>> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    hex::decode(raw)
        .map_err(|error| ServiceError::bad_request(format!("Invalid {label} encoding: {error}")))
}
