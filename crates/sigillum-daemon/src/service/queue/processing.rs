//! Queue processing loop and execution-result state transitions.
//!
//! The drain loop itself lives here; per-source serialization (W7.4) is
//! split into `serialization.rs`, applying a job's outcome to its persisted
//! fields + the drain tally is split into `outcomes.rs`, and the
//! operation-progress job selection into `selection.rs` (house architecture
//! cap).

use std::collections::HashMap;

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest,
    OPERATION_KIND_QUEUE_PROCESS, OPERATION_STATE_CANCELED, OPERATION_STATE_COMPLETED,
    OPERATION_STATE_FAILED, QueueJobPayload, QueueProcessRequest, QueueProcessResponse,
    StealthPaymentRef,
};

use crate::audit_log::AuditEventSpec;
use crate::operation_registry::OperationHandle;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::QueueExecution;
use super::gates::EXECUTION_PAUSED_REASON;
use super::selection::QueueDrainSelection;
use super::serialization;
use super::state::queue_job_is_runnable;
use super::tally::QueueDrainTally;

/// `QueueProcessResponse::paused_reason` value when the drain stopped early
/// because its tracking operation was canceled. Cancellation is honored at
/// the same boundary as the `execution_paused` kill switch — BETWEEN jobs,
/// never mid-broadcast — so a canceled drain's in-flight job finished its
/// current attempt and the remaining selected jobs keep state and attempts
/// intact (processed vs remaining counts are reported in the response tally
/// and the operation's progress).
pub(crate) const OPERATION_CANCELED_REASON: &str =
    "operation_canceled: the operator canceled this queue drain";

impl SigillumService {
    /// Process queued jobs (the queue drain).
    ///
    /// Both the synchronous and `run_async` paths share one pipeline: the
    /// request is authenticated synchronously up front (so async
    /// submissions fail fast on bad input), and [`Self::execute_queue_process`]
    /// drives the drain loop under the operation guard with per-job
    /// progress and cooperative cancellation between jobs.
    pub(crate) async fn process_queue(
        &self,
        token: Option<&str>,
        body: QueueProcessRequest,
    ) -> ServiceResult<QueueProcessResponse> {
        let token = self.require_session(token)?;
        if body.run_async == Some(true) {
            let operation = self.spawn_async_queue_process(token, body);
            return Ok(QueueProcessResponse {
                processed: 0,
                succeeded: 0,
                blocked: 0,
                retrying: 0,
                operator_action_required: 0,
                failed: 0,
                confirmed: 0,
                failures_by_cause: Default::default(),
                paused_reason: None,
                jobs: Vec::new(),
                operation: Some(operation),
            });
        }
        // Synchronous path: identical behavior to the historical endpoint,
        // including the response contract (no `operation` field). The drain
        // is still registered as an operation so other clients can observe
        // or cancel it mid-run.
        let operation = self
            .state
            .start_operation(OPERATION_KIND_QUEUE_PROCESS, Vec::new());
        self.execute_queue_process(token, body, operation).await
    }

    /// Spawn a drain as a background daemon operation, returning the
    /// operation tracking it.
    fn spawn_async_queue_process(
        &self,
        token: &str,
        body: QueueProcessRequest,
    ) -> sigillum_api::Operation {
        let operation = self
            .state
            .start_operation(OPERATION_KIND_QUEUE_PROCESS, Vec::new());
        let operation_id = operation.id().to_string();
        let service = self.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            if let Err(error) = service.execute_queue_process(&token, body, operation).await {
                tracing::warn!(error = %error, "async queue drain failed");
            }
        });
        self.state
            .get_operation(&operation_id)
            .expect("operation registered above")
    }

    /// Execute a drain under the operation guard.
    ///
    /// The guard is held for the whole run exactly like the historical
    /// synchronous path, so mutation-serialization semantics are unchanged.
    /// Cancellation is cooperative: the loop checks the operation's cancel
    /// flag BETWEEN jobs (never mid-broadcast — the durable
    /// prepared/submitted_unknown barriers and the kill-switch checks
    /// already bracket the dangerous region), so an in-flight job always
    /// finishes its current attempt before the drain stops. Every error
    /// exit marks the operation `failed` instead of leaking a permanently
    /// `running` record.
    async fn execute_queue_process(
        &self,
        token: &str,
        body: QueueProcessRequest,
        operation: OperationHandle,
    ) -> ServiceResult<QueueProcessResponse> {
        let result = self
            .execute_queue_process_inner(token, body, &operation)
            .await;
        if let Err(error) = &result {
            self.state.finish_operation(
                operation.id(),
                OPERATION_STATE_FAILED,
                Some(error.message().to_string()),
            );
        }
        result
    }

    async fn execute_queue_process_inner(
        &self,
        token: &str,
        body: QueueProcessRequest,
        operation: &OperationHandle,
    ) -> ServiceResult<QueueProcessResponse> {
        let _guard = self.state.operation_guard().await;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let processed = self
            .process_queue_state(token, &mut queue, body, Some(operation))
            .await?;

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

        let canceled = processed.paused_reason.as_deref() == Some(OPERATION_CANCELED_REASON);
        self.state.finish_operation(
            operation.id(),
            if canceled {
                OPERATION_STATE_CANCELED
            } else {
                OPERATION_STATE_COMPLETED
            },
            None,
        );

        Ok(processed)
    }

    pub(in crate::service) async fn process_queue_state(
        &self,
        token: &str,
        queue: &mut crate::queue_store::QueueState,
        body: QueueProcessRequest,
        operation: Option<&OperationHandle>,
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
        // call. The drain uses indexes (rather than `iter_mut`) so it can
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

        // Operation progress: the selected job set for this run (see
        // `selection.rs` for the exact semantics), reported as
        // `progress.total`; `progress.processed` tracks attempted jobs.
        let mut selection = operation.map(|operation| {
            let selection = QueueDrainSelection::new(queue, &body, limit, now_unix());
            self.state
                .operation_set_progress_total(operation.id(), selection.total() as u64);
            selection
        });

        for job_index in 0..queue.jobs.len() {
            if processed.len() >= limit {
                break;
            }
            // Cooperative cancel checkpoint: BETWEEN jobs only, never
            // mid-broadcast. Behaves exactly like the `execution_paused`
            // boundary below — no new job starts; any in-flight job has
            // already finished its current attempt.
            if operation.is_some_and(|operation| operation.cancellation_requested()) {
                paused_reason = Some(OPERATION_CANCELED_REASON.to_string());
                break;
            }
            // Pause is immediate: no new job starts. Any in-flight job finishes
            // its current attempt; remaining jobs keep state and attempts intact.
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
                // A drain-start-selected job parked now (its source became
                // in-flight this batch) shrinks the operation's total.
                if let (Some(selection), Some(operation)) = (selection.as_mut(), operation) {
                    if let Some(total) = selection.deselect(job_index) {
                        self.state
                            .operation_set_progress_total(operation.id(), total as u64);
                    }
                }
                queue.jobs[job_index].last_error = Some(reason);
                if body.id.is_some() {
                    break;
                }
                continue;
            }

            // A job admitted mid-drain that was not selected at drain start
            // (its backoff expired or its source freed up) grows the total.
            if let (Some(selection), Some(operation)) = (selection.as_mut(), operation) {
                if let Some(total) = selection.select(job_index) {
                    self.state
                        .operation_set_progress_total(operation.id(), total as u64);
                }
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
                if self.state.queue_execution_pause_latched() {
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
            if let Some(operation) = operation {
                self.state
                    .operation_set_progress(operation.id(), processed.len() as u64);
            }

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
            operation: None,
        })
    }
}
