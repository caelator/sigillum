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
use super::gates::{DAEMON_LOCKING_REASON, EXECUTION_PAUSED_REASON};
use super::serialization;
use super::state::queue_job_is_runnable;
use super::tally::QueueDrainTally;

impl SigillumService {
    pub(crate) async fn process_queue(
        &self,
        token: Option<&str>,
        body: QueueProcessRequest,
    ) -> ServiceResult<QueueProcessResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
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

        // Snapshot of every job's id -> state, refreshed after each job so a
        // prerequisite polled to `confirmed` can unblock a later dependent
        // within this drain batch (W6.4 ordering). A freshly broadcast
        // prerequisite remains `sent`, so its dependent waits for a later
        // confirmation cycle. The drain uses indexes (rather than `iter_mut`) so it can
        // durably save the whole queue at the prepare/submission barriers.
        let mut job_states: HashMap<String, String> = queue
            .jobs
            .iter()
            .map(|job| (job.id.clone(), job.state.clone()))
            .collect();

        // W7.4 per-source serialization snapshot: (chain, source) -> the
        // job id currently occupying it (broadcast, awaiting confirmation).
        // Refreshed the same way as `job_states` (see `serialization.rs`).
        let mut in_flight_sources = serialization::build_in_flight_sources(&queue.jobs);

        for job_index in 0..queue.jobs.len() {
            if processed.len() >= limit {
                break;
            }
            if self.state.is_locking() {
                paused_reason = Some(DAEMON_LOCKING_REASON.to_string());
                break;
            }
            // Pause prevents another job from starting without disturbing its state.
            if self.queue_execution_paused()? {
                paused_reason = Some(EXECUTION_PAUSED_REASON.to_string());
                break;
            }
            let job = queue.jobs[job_index].clone();
            if let Some(target_id) = body.id.as_deref() {
                if job.id != target_id {
                    continue;
                }
            }

            let now = now_unix();
            if !queue_job_is_runnable(&job, force_target, now) {
                if body.id.is_some() {
                    break;
                }
                continue;
            }

            if let Some(reason) = serialization::skip_reason(&job, &in_flight_sources) {
                queue.jobs[job_index].last_error = Some(reason);
                if body.id.is_some() {
                    break;
                }
                continue;
            }

            queue.jobs[job_index].attempts += 1;
            queue.jobs[job_index].updated_at_unix = now;
            queue.jobs[job_index].next_attempt_after_unix = None;
            let job = queue.jobs[job_index].clone();
            let had_replay_bytes_at_start = job.receipt.signed_raw_transaction_hex.is_some();
            // A broadcast plan step is receipt-only forever; policy gates
            // stop fresh signing/submission but must never demote `sent`.
            let gate_block_reason = if super::in_flight::is_sent_plan_step(&job) {
                None
            } else {
                self.execution_gate_block_reason(&job.payload)?
            };
            let in_flight_result = self
                .process_in_flight_queue_job(
                    token,
                    queue,
                    job_index,
                    &job,
                    &job_states,
                    now,
                    gate_block_reason.is_none(),
                )
                .await?;

            #[rustfmt::skip]
            let result = if let Some(result) = in_flight_result {
                result
            } else if let Some(reason) = gate_block_reason {
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
                    .eth_stealth_send_with_profile_in_operation(
                        token,
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
                } => self
                    .eth_stealth_send_erc20_with_profile_in_operation(
                        token,
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

            super::outcomes::apply(&mut queue.jobs[job_index], result, now, policy, &mut tally);

            // A newly signed job crosses a durable prepare barrier before
            // any external network call. Then persist `submitted_unknown`
            // before the call so a crash can only lead to receipt lookup or
            // exact-byte rebroadcast, never another signature.
            if queue.jobs[job_index].state == super::QUEUE_STATE_PREPARED
                && queue.jobs[job_index]
                    .receipt
                    .signed_raw_transaction_hex
                    .is_some()
                && !had_replay_bytes_at_start
            {
                super::broadcast::persist_queue(&self.state.base_dir, queue)?;
                super::failpoints::hit(super::failpoints::AFTER_PREPARED_PERSIST);
                if self.state.is_locking() {
                    paused_reason = Some(DAEMON_LOCKING_REASON.to_string());
                } else if self.state.queue_execution_pause_latched() {
                    paused_reason = Some(EXECUTION_PAUSED_REASON.to_string());
                } else {
                    self.persist_queue_submission_marker(queue, job_index, now)?;
                    let submitted_job = queue.jobs[job_index].clone();
                    let broadcast_result = self
                        .broadcast_prepared_queue_job(token, &submitted_job, false, true)
                        .await;
                    super::outcomes::apply(
                        &mut queue.jobs[job_index],
                        broadcast_result,
                        now,
                        policy,
                        &mut tally,
                    );
                }
            }

            // Refresh the dependency-lookup snapshot immediately so a
            // dependent `PlanStepExecution` job processed later in THIS
            // batch sees this job's just-computed outcome.
            let job = &queue.jobs[job_index];
            job_states.insert(job.id.clone(), job.state.clone());
            serialization::refresh(&mut in_flight_sources, job);

            processed.push(super::queue_job_for_response(job.clone()));

            if body.id.is_some() || paused_reason.is_some() {
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
