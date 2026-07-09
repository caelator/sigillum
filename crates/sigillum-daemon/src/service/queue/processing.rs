//! Queue processing loop and execution-result state transitions.
//!
//! The drain loop itself lives here; per-source serialization (W7.4) is
//! split into `serialization.rs` and applying a job's outcome to its
//! persisted fields + the drain tally is split into `outcomes.rs` (house
//! architecture cap).

use std::collections::HashMap;

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, QueueJobPayload,
    QueueProcessRequest, QueueProcessResponse, StealthPaymentRef,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::QueueExecution;
use super::gates::EXECUTION_PAUSED_REASON;
use super::outcomes::QueueDrainTally;
use super::serialization;
use super::state::queue_job_is_runnable;

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
        let mut tally = QueueDrainTally::default();
        let mut paused_reason: Option<String> = None;

        // Snapshot of every job's id -> state, refreshed after each job so
        // `PlanStepExecution` dependents resolve within the same drain batch
        // (W6.4 ordering) instead of waiting for the next `process_queue`
        // call. Built once up front because `queue.jobs.iter_mut()` below
        // holds a mutable borrow of the vector for the loop's duration.
        let mut job_states: HashMap<String, String> = queue
            .jobs
            .iter()
            .map(|job| (job.id.clone(), job.state.clone()))
            .collect();

        // W7.4 per-source serialization snapshot: (chain, source) -> the
        // job id currently occupying it (broadcast, awaiting confirmation).
        // Refreshed the same way as `job_states` (see `serialization.rs`).
        let mut in_flight_sources = serialization::build_in_flight_sources(&queue.jobs);

        for job in queue.jobs.iter_mut() {
            if processed.len() >= limit {
                break;
            }
            // Pause is immediate: no new job starts. Any in-flight job finishes
            // its current attempt; remaining jobs keep state and attempts intact.
            if self.queue_execution_paused()? {
                paused_reason = Some(EXECUTION_PAUSED_REASON.to_string());
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

            if let Some(reason) = serialization::skip_reason(job, &in_flight_sources) {
                job.last_error = Some(reason);
                if body.id.is_some() {
                    break;
                }
                continue;
            }

            job.attempts += 1;
            job.updated_at_unix = now;
            job.next_attempt_after_unix = None;

            #[rustfmt::skip]
            let result = if let Some(reason) = self.execution_gate_block_reason(&job.payload)? {
                Ok(QueueExecution::Blocked(reason))
            } else {
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
                            estimate_fees: None,
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
                            estimate_fees: None,
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
                // W7.3: EthSeed* jobs execute once their Sweep-family gate
                // (checked above, before this match) passes. With gates off
                // the outer `execution_gate_block_reason` check above
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
                // W7.3/W7.4: plan-step jobs execute once their action-family
                // gate (checked above) passes; see `plan_steps.rs` for the
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
                        &job_states,
                    )
                    .await
                }
            }
            };

            super::outcomes::apply(job, result, now, policy, &mut tally);

            // Refresh the dependency-lookup snapshot immediately so a
            // dependent `PlanStepExecution` job processed later in THIS
            // batch sees this job's just-computed outcome.
            job_states.insert(job.id.clone(), job.state.clone());
            serialization::refresh(&mut in_flight_sources, job);

            processed.push(job.clone());

            if body.id.is_some() {
                break;
            }
        }

        Ok(QueueProcessResponse {
            processed: processed.len(),
            succeeded: tally.succeeded,
            blocked: tally.blocked,
            retrying: tally.retrying,
            operator_action_required: tally.operator_action_required,
            failed: tally.failed,
            confirmed: tally.confirmed,
            failures_by_cause: tally.failures_by_cause,
            paused_reason,
            jobs: processed,
        })
    }
}
