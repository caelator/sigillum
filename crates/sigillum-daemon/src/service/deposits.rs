//! Deposit tracking and management for stealth addresses.
//!
//! Manages creation, tracking, and sweeping of Ethereum stealth deposits
//! with auto-queueing and balance refresh capabilities.
//!
//! ## Deposit lifecycle
//!
//! 1. **Create** — generates a fresh stealth address from the wallet profile,
//!    persists a `pending` deposit record, and optionally configures auto-sweep.
//! 2. **Refresh** — queries on-chain balances, updates `observed_amount_hex`,
//!    transitions status, and auto-enqueues sweep jobs when thresholds are met.
//! 3. **Enqueue sweep** — places a native or ERC-20 sweep job on the queue.
//! 4. **Delete** — removes the deposit record (does not affect on-chain state).
//!
//! Both native and ERC-20 deposit creation share core logic extracted into
//! [`DepositBlueprint`] and [`SigillumService::persist_new_deposit`] to
//! avoid structural duplication while keeping the public API surface clean.

use std::collections::HashMap;

use sigillum_api::{
    EthStealthDeposit, EthStealthDepositCreateErc20Request, EthStealthDepositCreateNativeRequest,
    EthStealthDepositDeleteRequest, EthStealthDepositEnqueueSweepRequest,
    EthStealthDepositEnqueueSweepResponse, EthStealthDepositListResponse,
    EthStealthDepositMutationResponse, EthStealthDepositRefreshRequest,
    EthStealthDepositRefreshResponse, EthStealthGenerateRequest, EthStealthWalletProfile,
    EvmProviderProfile, QueueEnqueueResponse, QueueJob, QueueJobPayload,
};
use sigillum_core::{VaultLifecycle, decode_quantity_hex, derive_sigillum_ethereum_stealth_wallet};

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};

use super::helpers::{
    compare_u256, is_zero_u256, map_wallet_error, multiply_u256_u64, now_unix, random_id,
};
use super::{ServiceError, ServiceResult, SigillumService};

// ── Deposit Blueprint & Plans ──────────────────────────────────────────────

/// Intermediate representation capturing all parameters needed to construct a
/// new [`EthStealthDeposit`], shared between native and ERC-20 creation paths.
struct DepositBlueprint {
    wallet_profile: String,
    wallet_compartment_id: usize,
    provider_compartment_id: usize,
    wallet: String,
    short_name: String,
    asset_kind: String,
    token_address: Option<String>,
    expected_amount_hex: Option<String>,
    auto_queue_sweep: bool,
    sweep_destination_address: Option<String>,
    min_sweep_amount_hex: Option<String>,
    note: Option<String>,
}

#[derive(Clone)]
struct DepositRefreshPlan {
    deposit_index: usize,
    provider: EvmProviderProfile,
    wallet: EthStealthWalletProfile,
}

// ── Deposit Creation & Deletion ────────────────────────────────────────────

impl SigillumService {
    pub(crate) fn list_eth_stealth_deposits(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<EthStealthDepositListResponse> {
        let _ = self.require_session(token)?;
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        Ok(EthStealthDepositListResponse {
            deposits: deposits.eth_stealth,
        })
    }

    pub(crate) async fn create_eth_stealth_native_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositCreateNativeRequest,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        validate_optional_quantity(body.expected_value_wei_hex.as_deref(), "expected_value_wei")?;
        validate_optional_quantity(
            body.min_sweep_value_wei_hex.as_deref(),
            "min_sweep_value_wei",
        )?;

        self.persist_new_deposit(
            token,
            &wallet,
            &provider,
            body.ephemeral_private_key_hex,
            DepositBlueprint {
                wallet_profile: body.wallet_profile,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                asset_kind: "native".into(),
                token_address: None,
                expected_amount_hex: body.expected_value_wei_hex,
                auto_queue_sweep: body.auto_queue_sweep.unwrap_or(false),
                sweep_destination_address: body
                    .sweep_destination_address
                    .or(wallet.default_destination_address.clone()),
                min_sweep_amount_hex: body.min_sweep_value_wei_hex,
                note: body.note,
            },
        )
        .await
    }

    pub(crate) async fn create_eth_stealth_erc20_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositCreateErc20Request,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        validate_optional_quantity(body.expected_amount_hex.as_deref(), "expected_amount")?;
        validate_optional_quantity(body.min_sweep_amount_hex.as_deref(), "min_sweep_amount")?;
        let normalized_token = super::evm::normalize_address(&body.token_address)?;

        self.persist_new_deposit(
            token,
            &wallet,
            &provider,
            body.ephemeral_private_key_hex,
            DepositBlueprint {
                wallet_profile: body.wallet_profile,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                asset_kind: "erc20".into(),
                token_address: Some(normalized_token),
                expected_amount_hex: body.expected_amount_hex,
                auto_queue_sweep: body.auto_queue_sweep.unwrap_or(false),
                sweep_destination_address: body
                    .sweep_destination_address
                    .or(wallet.default_destination_address.clone()),
                min_sweep_amount_hex: body.min_sweep_amount_hex,
                note: body.note,
            },
        )
        .await
    }

    /// Shared deposit creation: derive stealth address, build record, persist, and audit.
    ///
    /// Both native and ERC-20 deposit flows converge here after validating their
    /// type-specific fields and constructing a [`DepositBlueprint`].
    async fn persist_new_deposit(
        &self,
        token: &str,
        wallet: &EthStealthWalletProfile,
        _provider: &EvmProviderProfile,
        ephemeral_private_key_hex: Option<String>,
        blueprint: DepositBlueprint,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let meta = self.with_vault(wallet.compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Wallet compartment is locked."))?;
            let derived = derive_sigillum_ethereum_stealth_wallet(
                master_key.as_ref(),
                &wallet.wallet,
                &wallet.short_name,
            )
            .map_err(map_wallet_error)?;
            Ok(derived.meta_address().clone())
        })?;
        let payment = self.eth_stealth_generate(EthStealthGenerateRequest {
            stealth_meta_address: meta.stealth_meta_address.clone(),
            ephemeral_private_key_hex,
        })?;

        let now = now_unix();
        let deposit = EthStealthDeposit {
            id: random_id(),
            status: "pending".into(),
            asset_kind: blueprint.asset_kind,
            wallet_profile: blueprint.wallet_profile,
            wallet_compartment_id: blueprint.wallet_compartment_id,
            provider_compartment_id: blueprint.provider_compartment_id,
            wallet: blueprint.wallet,
            short_name: blueprint.short_name,
            stealth_meta_address: meta.stealth_meta_address,
            stealth_address: payment.stealth_address,
            ephemeral_public_key_hex: payment.ephemeral_public_key_hex,
            view_tag_hex: payment.view_tag_hex,
            token_address: blueprint.token_address,
            expected_amount_hex: blueprint.expected_amount_hex,
            observed_amount_hex: None,
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: blueprint.auto_queue_sweep,
            sweep_destination_address: blueprint.sweep_destination_address,
            min_sweep_amount_hex: blueprint.min_sweep_amount_hex,
            queue_job_id: None,
            queue_job_state: None,
            note: blueprint.note,
            created_at_unix: now,
            updated_at_unix: now,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
        };

        let _guard = self.state.operation_guard().await;
        let mut state = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        state.eth_stealth.push(deposit.clone());
        state
            .eth_stealth
            .sort_by(|left, right| left.created_at_unix.cmp(&right.created_at_unix));
        crate::deposits::save_deposits(&self.state.base_dir, &state)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthCreate {
                id: deposit.id.clone(),
                wallet_profile: deposit.wallet_profile.clone(),
                asset_kind: deposit.asset_kind.clone(),
                token_address: deposit.token_address.clone(),
            },
        )?;

        Ok(EthStealthDepositMutationResponse {
            status: "created".into(),
            deposit,
        })
    }

    // ── Deposit Deletion ──────────────────────────────────────────────────

    pub(crate) async fn delete_eth_stealth_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositDeleteRequest,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let index = state
            .eth_stealth
            .iter()
            .position(|deposit| deposit.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Deposit not found."))?;
        let deposit = state.eth_stealth.remove(index);
        crate::deposits::save_deposits(&self.state.base_dir, &state)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthDelete {
                id: deposit.id.clone(),
            },
        )?;

        Ok(EthStealthDepositMutationResponse {
            status: "deleted".into(),
            deposit,
        })
    }

    // ── Deposit Refresh ───────────────────────────────────────────────────

    pub(crate) async fn refresh_eth_stealth_deposits(
        &self,
        token: Option<&str>,
        body: EthStealthDepositRefreshRequest,
    ) -> ServiceResult<EthStealthDepositRefreshResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let response = self
            .refresh_eth_stealth_deposits_state(token, &mut deposits, &mut queue, body)
            .await?;

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthRefresh {
                processed: response.processed,
                detected: response.detected,
                queued: response.queued,
            },
        )?;

        Ok(response)
    }

    // ── Deposit Sweep Enqueueing ──────────────────────────────────────────

    pub(crate) async fn enqueue_eth_stealth_deposit_sweep(
        &self,
        token: Option<&str>,
        body: EthStealthDepositEnqueueSweepRequest,
    ) -> ServiceResult<EthStealthDepositEnqueueSweepResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let deposit_snapshot = {
            let deposit = deposits
                .eth_stealth
                .iter_mut()
                .find(|deposit| deposit.id == body.id)
                .ok_or_else(|| ServiceError::not_found("Deposit not found."))?;
            let queue_state = deposit
                .queue_job_id
                .as_deref()
                .and_then(|id| queue.jobs.iter().find(|job| job.id == id))
                .map(|job| job.state.clone());
            if !body.force.unwrap_or(false)
                && queue_state
                    .as_deref()
                    .map(super::queue::is_active_or_completed_queue_state)
                    .unwrap_or(false)
            {
                return Err(ServiceError::conflict(
                    "Deposit already has an active or completed sweep job.",
                ));
            }

            let (provider, wallet) = self.resolve_wallet_profile(&deposit.wallet_profile)?;
            let enqueue = self
                .enqueue_deposit_sweep_job(token, deposit, &wallet, &provider, &mut queue, true)?;
            deposit.queue_job_id = Some(enqueue.job.id.clone());
            deposit.queue_job_state = Some(enqueue.job.state.clone());
            deposit.status = super::queue::queue_status(&enqueue.job.state);
            deposit.updated_at_unix = now_unix();
            (deposit.clone(), enqueue)
        };

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthEnqueueSweep {
                id: deposit_snapshot.0.id.clone(),
                job_id: deposit_snapshot.1.job.id.clone(),
            },
        )?;

        Ok(EthStealthDepositEnqueueSweepResponse {
            status: deposit_snapshot.1.status,
            deposit: deposit_snapshot.0,
            job: deposit_snapshot.1.job,
        })
    }

    // ── Sweep Job Construction ────────────────────────────────────────────

    fn enqueue_deposit_sweep_job(
        &self,
        token: &str,
        deposit: &EthStealthDeposit,
        wallet: &sigillum_api::EthStealthWalletProfile,
        provider: &sigillum_api::EvmProviderProfile,
        queue: &mut crate::queue_store::QueueState,
        strict_destination: bool,
    ) -> ServiceResult<QueueEnqueueResponse> {
        let destination = deposit
            .sweep_destination_address
            .clone()
            .or_else(|| wallet.default_destination_address.clone());

        let job = if deposit.asset_kind == "erc20" {
            let recipient_address = destination.ok_or_else(|| {
                ServiceError::bad_request(
                    "ERC-20 deposit requires sweep_destination_address or wallet default destination.",
                )
            })?;
            QueueJob {
                id: random_id(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: now_unix(),
                updated_at_unix: now_unix(),
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthErc20Sweep {
                    wallet_profile: deposit.wallet_profile.clone(),
                    stealth_address: deposit.stealth_address.clone(),
                    ephemeral_public_key_hex: deposit.ephemeral_public_key_hex.clone(),
                    token_address: deposit.token_address.clone().ok_or_else(|| {
                        ServiceError::internal("ERC-20 deposit missing token_address")
                    })?,
                    recipient_address: Some(recipient_address),
                    min_amount_hex: deposit.min_sweep_amount_hex.clone(),
                    gas_limit: provider.erc20_gas_limit,
                    view_tag_hex: Some(deposit.view_tag_hex.clone()),
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
            }
        } else {
            let destination_address = if strict_destination {
                destination.ok_or_else(|| {
                    ServiceError::bad_request(
                        "Native deposit requires sweep_destination_address or wallet default destination.",
                    )
                })?
            } else {
                destination.unwrap_or_else(|| {
                    wallet
                        .default_destination_address
                        .clone()
                        .unwrap_or_default()
                })
            };
            if destination_address.is_empty() {
                return Err(ServiceError::bad_request(
                    "Native deposit requires sweep_destination_address or wallet default destination.",
                ));
            }
            QueueJob {
                id: random_id(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: now_unix(),
                updated_at_unix: now_unix(),
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthNativeSweep {
                    wallet_profile: deposit.wallet_profile.clone(),
                    stealth_address: deposit.stealth_address.clone(),
                    ephemeral_public_key_hex: deposit.ephemeral_public_key_hex.clone(),
                    destination_address: Some(destination_address),
                    min_value_wei_hex: deposit.min_sweep_amount_hex.clone(),
                    gas_limit: provider.native_gas_limit,
                    view_tag_hex: Some(deposit.view_tag_hex.clone()),
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
            }
        };

        queue.jobs.push(job.clone());
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::QueueEnqueue {
                id: job.id.clone(),
                job_kind: AuditQueueJobKind::from_payload(&job.payload),
            },
        )?;

        Ok(QueueEnqueueResponse {
            status: "queued".into(),
            job,
        })
    }

    // ── Deposit State Refresh & Sync ───────────────────────────────────────

    pub(super) async fn refresh_eth_stealth_deposits_state(
        &self,
        token: &str,
        deposits: &mut crate::deposits::DepositState,
        queue: &mut crate::queue_store::QueueState,
        body: EthStealthDepositRefreshRequest,
    ) -> ServiceResult<EthStealthDepositRefreshResponse> {
        let limit = self
            .state
            .runtime_policy()
            .deposit_refresh_limit(body.limit);
        let auto_enqueue = body.auto_enqueue.unwrap_or(true);
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let mut processed = Vec::new();
        let mut detected = 0usize;
        let mut queued = 0usize;
        let mut plans = Vec::new();
        let mut observation_plans = Vec::new();

        for (deposit_index, deposit) in deposits.eth_stealth.iter().enumerate() {
            if plans.len() >= limit {
                break;
            }
            if let Some(id) = body.id.as_deref() {
                if deposit.id != id {
                    continue;
                }
            }

            let (provider, wallet) = super::profiles::resolve_wallet_profile_in_registry(
                &registry,
                &deposit.wallet_profile,
            )?;
            plans.push(DepositRefreshPlan {
                deposit_index,
                provider: provider.clone(),
                wallet,
            });
            observation_plans.push(super::evm::EvmBalanceObservationPlan {
                deposit_index,
                provider_compartment_id: deposit.provider_compartment_id,
                provider,
                owner_address: deposit.stealth_address.clone(),
                token_address: deposit.token_address.clone(),
            });

            if body.id.is_some() {
                break;
            }
        }

        let observations = self.fetch_balance_observations(observation_plans).await?;
        let plans_by_index: HashMap<usize, DepositRefreshPlan> = plans
            .into_iter()
            .map(|plan| (plan.deposit_index, plan))
            .collect();

        for observation in observations {
            let plan = plans_by_index
                .get(&observation.deposit_index)
                .ok_or_else(|| {
                    ServiceError::internal("Missing deposit refresh plan for observation")
                })?;
            let deposit = deposits
                .eth_stealth
                .get_mut(observation.deposit_index)
                .ok_or_else(|| ServiceError::internal("Deposit index went out of range"))?;
            let native_balance = decode_quantity_hex(&observation.native_balance_wei_hex)
                .map_err(map_wallet_error)?;

            deposit.observed_native_balance_wei_hex =
                (deposit.asset_kind == "erc20").then(|| observation.native_balance_wei_hex.clone());
            deposit.observed_amount_hex = Some(observation.observed_amount_hex.clone());
            deposit.last_checked_at_unix = Some(now_unix());
            deposit.updated_at_unix = now_unix();

            let (queue_state, _) = sync_eth_stealth_deposit_with_queue(deposit, queue);
            let observed_amount_raw =
                decode_quantity_hex(&observation.observed_amount_hex).map_err(map_wallet_error)?;
            let min_ready = match deposit.min_sweep_amount_hex.as_deref() {
                Some(minimum) => compare_u256(
                    &observed_amount_raw,
                    &decode_quantity_hex(minimum).map_err(map_wallet_error)?,
                )
                .is_ge(),
                None => !is_zero_u256(&observed_amount_raw),
            };

            if !is_zero_u256(&observed_amount_raw) {
                detected += 1;
            }

            deposit.status = if let Some(job_state) = queue_state.clone() {
                super::queue::queue_status(&job_state)
            } else if is_zero_u256(&observed_amount_raw) {
                "pending".into()
            } else if deposit.asset_kind == "erc20"
                && !gas_balance_sufficient_for_erc20(deposit, &plan.provider, &native_balance)?
            {
                "funded_needs_gas".into()
            } else {
                "funded".into()
            };

            let has_active_job = queue_state
                .as_deref()
                .map(super::queue::is_active_queue_state)
                .unwrap_or(false);
            if auto_enqueue && deposit.auto_queue_sweep && !has_active_job && min_ready {
                let enqueue_result = self.enqueue_deposit_sweep_job(
                    token,
                    deposit,
                    &plan.wallet,
                    &plan.provider,
                    queue,
                    false,
                )?;
                queued += 1;
                deposit.queue_job_id = Some(enqueue_result.job.id.clone());
                deposit.queue_job_state = Some(enqueue_result.job.state.clone());
                deposit.status = super::queue::queue_status(&enqueue_result.job.state);
            }

            processed.push(deposit.clone());
        }

        Ok(EthStealthDepositRefreshResponse {
            processed: processed.len(),
            detected,
            queued,
            deposits: processed,
        })
    }
}

// ── Validation & Helper Functions ──────────────────────────────────────────

fn validate_optional_quantity(value: Option<&str>, label: &str) -> ServiceResult<()> {
    if let Some(value) = value {
        decode_quantity_hex(value).map_err(|_| {
            ServiceError::bad_request(format!("{label} must be a valid hex quantity"))
        })?;
    }
    Ok(())
}

fn gas_balance_sufficient_for_erc20(
    deposit: &EthStealthDeposit,
    provider: &sigillum_api::EvmProviderProfile,
    native_balance: &[u8; 32],
) -> ServiceResult<bool> {
    let Some(max_fee_hex) = provider.max_fee_per_gas_hex.as_deref() else {
        return Ok(false);
    };
    let max_fee = decode_quantity_hex(max_fee_hex).map_err(map_wallet_error)?;
    let gas_limit = provider.erc20_gas_limit.unwrap_or(65_000);
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    let min_amount_ready = match deposit.min_sweep_amount_hex.as_deref() {
        Some(minimum) => {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            let observed = deposit
                .observed_amount_hex
                .as_deref()
                .map(decode_quantity_hex)
                .transpose()
                .map_err(map_wallet_error)?
                .unwrap_or([0u8; 32]);
            compare_u256(&observed, &minimum).is_ge()
        }
        None => true,
    };
    Ok(min_amount_ready && compare_u256(native_balance, &gas_cost).is_ge())
}

// ── Queue Synchronization ─────────────────────────────────────────────────

pub(super) fn sync_eth_stealth_deposits_with_queue(
    deposits: &mut crate::deposits::DepositState,
    queue: &crate::queue_store::QueueState,
) -> usize {
    let mut reconciled = 0usize;
    for deposit in &mut deposits.eth_stealth {
        if sync_eth_stealth_deposit_with_queue(deposit, queue).1 {
            reconciled += 1;
        }
    }
    reconciled
}

fn sync_eth_stealth_deposit_with_queue(
    deposit: &mut EthStealthDeposit,
    queue: &crate::queue_store::QueueState,
) -> (Option<String>, bool) {
    let previous_queue_job_state = deposit.queue_job_state.clone();
    let previous_status = deposit.status.clone();
    let previous_broadcast = deposit.broadcast_transaction_hash_hex.clone();
    let job = deposit
        .queue_job_id
        .as_deref()
        .and_then(|id| queue.jobs.iter().find(|candidate| candidate.id == id));
    let queue_state = job.map(|job| job.state.clone());
    deposit.queue_job_state = queue_state.clone();
    if let Some(hash) = job.and_then(|job| job.broadcast_transaction_hash_hex.clone()) {
        deposit.broadcast_transaction_hash_hex = Some(hash);
    }
    if let Some(state) = queue_state.as_deref() {
        deposit.status = super::queue::queue_status(state);
    }
    (
        queue_state,
        previous_queue_job_state != deposit.queue_job_state
            || previous_status != deposit.status
            || previous_broadcast != deposit.broadcast_transaction_hash_hex,
    )
}
