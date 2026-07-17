//! One-time receive-address lifecycle (plan task 3.3).
//!
//! A one-time allocation runs `allocate → auto-watch → auto-sweep-on-funds →
//! retire → optional purge`:
//!
//! 1. **auto-watch** — on the scheduler/maintenance refresh cadence the stage
//!    queries the wallet profile's provider for the allocation's native
//!    balance and upserts the standard inventory address row (the same row
//!    shape `refresh_receiving_balances` writes), so the address is tracked
//!    exactly like a scanned one;
//! 2. **auto-sweep** — when the observed balance reaches
//!    `min_sweep_amount_hex` (any nonzero balance when unset) AND the Sweep
//!    execution-family gates hold AND the destination passes the allowlist /
//!    step-cap policy checks AND no cross-party destination linkage block
//!    applies, the stage enqueues ONE `EthSeedNativeSweep` job to the
//!    allocation's destination. Dedupe mirrors the stealth-deposit sweep
//!    rule: a live (active or broadcast) or confirmed job for the allocation
//!    suppresses re-enqueue; a terminally failed / parked job does NOT
//!    auto-retry (the record shows `sweep_failed` / `sweep_attention` and the
//!    operator rotates or purges);
//! 3. **retire** — when the sweep job reaches its terminal success state
//!    (`sent` for the legacy `EthSeed*` queue family — "broadcast, done";
//!    those families never poll receipts — or `confirmed` under W7.4-style
//!    finality should the payload ever gain it), the allocation is retired
//!    with the same index-never-reissued semantics as rotate-retire, but no
//!    replacement is issued;
//! 4. **purge** — with `purge_after_sweep`, the retired record is deleted
//!    with the 3.2 purge semantics (the `treasury.receive.purge` audit event
//!    included). The observation row written in step 1 keeps the index
//!    reserved against re-issue even after the record is gone.
//!
//! Fail-closed posture: gates off (`allow_plan_execution` /
//! `allow_sweep_execution`, or no enabled policy) means nothing enqueues —
//! the allocation simply accrues and its lifecycle reads `watching` with the
//! `execution_gates` blocker; `execution_paused` halts the drain exactly as
//! it does today. The auto-sweep is a destination-axis linkage input exactly
//! like a plan sweep: two one-time allocations bound to DIFFERENT parties
//! sweeping to the SAME destination are hard-blocked while
//! `block_cross_party_linkage` is on (default since 3.5).
//!
//! Lifecycle states are DERIVED at read time (never persisted): `watching`
//! (active, no live sweep job), `sweep_queued` (sweep job active — queued,
//! retrying, prepared, broadcast-in-flight), `swept` (job settled —
//! terminal success for its queue family — retire imminent), `retired`
//! (terminal record state), `purged` (record absence plus the purge audit
//! event).

use sigillum_api::{
    EvmProviderProfile, QueueJobPayload, TreasuryPolicy, TreasuryReceiveAllocation,
    WalletAddressActivityState, WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};
use crate::inventory::WalletInventoryState;
use crate::queue_store::QueueState;

use super::super::helpers::{compare_u256, is_zero_u256, now_unix, random_id};
use super::super::queue::{
    ExecutionFamily, execution_gate_denial, is_active_or_completed_queue_state,
    is_active_queue_state, queue_job_failed_state, queue_job_operator_action_required,
    queue_job_sweep_settled_state, queued_job,
};
use super::super::transaction_policy::{
    TransactionPolicyAction, TransactionPolicyCheck, TransactionPolicyKind,
    transaction_policy_actions,
};
use super::super::{ServiceResult, SigillumService};
use super::DISCOVERY_SOURCE_LOCAL_RPC;
use super::support::{
    load_inventory_state, quantity_hex_is_nonzero, save_inventory_state, upsert_address,
};
use super::treasury::{RECEIVE_STATUS_ACTIVE, RECEIVE_STATUS_RETIRED};

/// `lifecycle_state` values (derived, read-time only).
pub(super) const LIFECYCLE_WATCHING: &str = "watching";
pub(super) const LIFECYCLE_SWEEP_QUEUED: &str = "sweep_queued";
pub(super) const LIFECYCLE_SWEPT: &str = "swept";
pub(super) const LIFECYCLE_RETIRED: &str = "retired";
// `purged` is terminal record ABSENCE plus a `treasury.receive.purge` audit
// event — it can never be read off a record.

/// `sweep_blocker` values: why a `watching` allocation has not swept yet.
pub(super) const BLOCKER_AWAITING_BALANCE: &str = "awaiting_balance";
pub(super) const BLOCKER_BELOW_THRESHOLD: &str = "below_threshold";
pub(super) const BLOCKER_EXECUTION_GATES: &str = "execution_gates";
pub(super) const BLOCKER_DESTINATION_POLICY: &str = "destination_policy";
pub(super) const BLOCKER_STEP_CAP: &str = "step_cap";
pub(super) const BLOCKER_CROSS_PARTY_LINKAGE: &str = "cross_party_linkage";
pub(super) const BLOCKER_SWEEP_FAILED: &str = "sweep_failed";
pub(super) const BLOCKER_SWEEP_ATTENTION: &str = "sweep_attention";

/// Audit reason recorded on the automatic one-time retire.
const ONE_TIME_RETIRE_REASON: &str = "one_time_sweep_settled";

/// Read-time lifecycle derivation for one allocation. `None` for non-one-time
/// allocations (no lifecycle fields surface).
pub(super) struct OneTimeEvaluation {
    pub lifecycle_state: &'static str,
    pub sweep_blocker: Option<&'static str>,
}

impl OneTimeEvaluation {
    /// Eligible to enqueue the auto-sweep NOW: watching with nothing blocking.
    fn sweep_eligible(&self) -> bool {
        self.lifecycle_state == LIFECYCLE_WATCHING && self.sweep_blocker.is_none()
    }
}

/// Derive the lifecycle state of one allocation from the raw record fields,
/// its tracked sweep job's state, the freshest observed balance, the
/// treasury policy, the latched kill switch, and the destination-linkage
/// verdict (computed over the whole allocation set by the caller).
///
/// The evaluation order is the operator's debugging order: in-flight sweep
/// first, then "why not yet" — data, threshold, gates, destination policy,
/// linkage, and finally the last sweep's terminal disposition.
pub(super) fn evaluate_one_time_allocation(
    allocation: &TreasuryReceiveAllocation,
    sweep_job_state: Option<&str>,
    observed_balance: Option<&[u8; 32]>,
    policy: Option<&TreasuryPolicy>,
    pause_latched: bool,
    linkage_shared: bool,
) -> Option<OneTimeEvaluation> {
    if !allocation.one_time {
        return None;
    }
    if allocation.status != RECEIVE_STATUS_ACTIVE {
        return Some(OneTimeEvaluation {
            lifecycle_state: LIFECYCLE_RETIRED,
            sweep_blocker: None,
        });
    }
    if let Some(state) = sweep_job_state {
        if is_active_queue_state(state) {
            return Some(OneTimeEvaluation {
                lifecycle_state: LIFECYCLE_SWEEP_QUEUED,
                sweep_blocker: None,
            });
        }
        // Terminal success for the sweep's queue family (`sent` for legacy
        // EthSeed*/EthStealth* — "broadcast, done"; `confirmed` under W7.4
        // finality): the allocation is swept and the next settle pass
        // retires it. Terminal failure / parking fall through to the
        // watching blockers.
        if queue_job_sweep_settled_state(state) {
            return Some(OneTimeEvaluation {
                lifecycle_state: LIFECYCLE_SWEPT,
                sweep_blocker: None,
            });
        }
    }

    let watching = |sweep_blocker| {
        Some(OneTimeEvaluation {
            lifecycle_state: LIFECYCLE_WATCHING,
            sweep_blocker,
        })
    };
    let Some(balance) = observed_balance else {
        return watching(Some(BLOCKER_AWAITING_BALANCE));
    };
    if !threshold_met(allocation, balance) {
        return watching(Some(BLOCKER_BELOW_THRESHOLD));
    }
    if pause_latched || execution_gate_denial(policy, ExecutionFamily::Sweep).is_some() {
        return watching(Some(BLOCKER_EXECUTION_GATES));
    }
    if let Some(blocker) = destination_policy_blocker(policy, allocation, balance) {
        return watching(Some(blocker));
    }
    if linkage_shared
        && policy
            .map(|policy| policy.block_cross_party_linkage)
            .unwrap_or(false)
    {
        return watching(Some(BLOCKER_CROSS_PARTY_LINKAGE));
    }
    if let Some(state) = sweep_job_state {
        if queue_job_failed_state(state) {
            return watching(Some(BLOCKER_SWEEP_FAILED));
        }
        if queue_job_operator_action_required(state) {
            return watching(Some(BLOCKER_SWEEP_ATTENTION));
        }
    }
    watching(None)
}

/// Sweep threshold: any nonzero balance when `min_sweep_amount_hex` is unset;
/// otherwise the observed balance must reach it. An undecodable stored
/// threshold fails closed (creation validates; corruption must never sweep).
fn threshold_met(allocation: &TreasuryReceiveAllocation, balance: &[u8; 32]) -> bool {
    if is_zero_u256(balance) {
        return false;
    }
    match allocation.min_sweep_amount_hex.as_deref() {
        Some(minimum) => match decode_quantity_hex(minimum) {
            Ok(minimum) => compare_u256(balance, &minimum).is_ge(),
            Err(_) => false,
        },
        None => true,
    }
}

/// Re-check the destination against the treasury policy exactly like any
/// sweep destination (`authorize_transaction_policy`): the allowlist applies
/// whenever the policy is enabled; the per-step native cap is judged against
/// the observed balance (an upper bound on the swept amount).
fn destination_policy_blocker(
    policy: Option<&TreasuryPolicy>,
    allocation: &TreasuryReceiveAllocation,
    balance: &[u8; 32],
) -> Option<&'static str> {
    let policy = policy?;
    let destination = allocation.sweep_destination_address.as_deref()?;
    let actions = transaction_policy_actions(
        policy,
        TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(destination),
            asset_kind: "native",
            amount_hex: &super::treasury::encode_quantity_hex(balance),
        },
    );
    actions.first().map(|action| match action {
        TransactionPolicyAction::BlockStepCap => BLOCKER_STEP_CAP,
        _ => BLOCKER_DESTINATION_POLICY,
    })
}

/// Party identity for the destination-axis linkage check: the bound
/// counterparty, or the address itself when unbound (mirrors the stealth
/// deposit `stealth_sweep_identity_key` rule).
fn one_time_identity_key(allocation: &TreasuryReceiveAllocation) -> String {
    allocation
        .counterparty_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| format!("counterparty:{id}"))
        .unwrap_or_else(|| format!("unattributed:{}", allocation.address.to_ascii_lowercase()))
}

/// Would this allocation's auto-sweep share its destination with ANOTHER
/// one-time allocation bound to a DIFFERENT identity? The other side counts
/// while it is active or already has a sweep job (its linkage is either
/// pending or realized); retired records that never swept link nothing.
pub(super) fn one_time_destination_linkage_shared(
    allocation: &TreasuryReceiveAllocation,
    others: &[TreasuryReceiveAllocation],
) -> bool {
    let Some(destination) = allocation
        .sweep_destination_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let destination_key = destination.to_ascii_lowercase();
    let identity = one_time_identity_key(allocation);
    others.iter().any(|other| {
        other.id != allocation.id
            && other.one_time
            && (other.status == RECEIVE_STATUS_ACTIVE || other.sweep_job_id.is_some())
            && other
                .sweep_destination_address
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .as_deref()
                == Some(destination_key.as_str())
            && one_time_identity_key(other) != identity
    })
}

/// Freshest observed native balance for the allocation's (chain, address)
/// from the inventory address rows (written by the auto-watch stage, scans,
/// or the manual receiving refresh). `None` means never observed.
pub(super) fn observed_allocation_balance(
    state: &WalletInventoryState,
    allocation: &TreasuryReceiveAllocation,
) -> Option<[u8; 32]> {
    state
        .addresses
        .iter()
        .filter(|row| {
            row.chain_id == allocation.chain_id
                && row.address.eq_ignore_ascii_case(&allocation.address)
        })
        .max_by_key(|row| row.last_checked_at_unix)
        .and_then(|row| decode_quantity_hex(&row.native_balance_wei_hex).ok())
}

/// State of the allocation's tracked sweep job, when the job still exists.
fn tracked_sweep_job_state<'a>(
    allocation: &TreasuryReceiveAllocation,
    queue: &'a QueueState,
) -> Option<&'a str> {
    allocation
        .sweep_job_id
        .as_deref()
        .and_then(|id| queue.jobs.iter().find(|job| job.id == id))
        .map(|job| job.state.as_str())
}

/// One read-time evaluation of an allocation against the current stores.
pub(super) fn evaluate_allocation_in_stores(
    allocation: &TreasuryReceiveAllocation,
    state: &WalletInventoryState,
    queue: &QueueState,
    pause_latched: bool,
) -> Option<OneTimeEvaluation> {
    if !allocation.one_time {
        return None;
    }
    let balance = observed_allocation_balance(state, allocation);
    let linkage_shared =
        one_time_destination_linkage_shared(allocation, &state.receive_allocations);
    evaluate_one_time_allocation(
        allocation,
        tracked_sweep_job_state(allocation, queue),
        balance.as_ref(),
        state.treasury_policy.as_ref(),
        pause_latched,
        linkage_shared,
    )
}

/// Clone an allocation with the read-time lifecycle fields populated (the
/// stored record is never mutated with derived state).
pub(super) fn with_one_time_lifecycle(
    allocation: &TreasuryReceiveAllocation,
    state: &WalletInventoryState,
    queue: &QueueState,
    pause_latched: bool,
) -> TreasuryReceiveAllocation {
    let mut allocation = allocation.clone();
    match evaluate_allocation_in_stores(&allocation, state, queue, pause_latched) {
        Some(evaluation) => {
            allocation.lifecycle_state = Some(evaluation.lifecycle_state.to_string());
            allocation.sweep_blocker = evaluation.sweep_blocker.map(str::to_string);
        }
        None => {
            allocation.lifecycle_state = None;
            allocation.sweep_blocker = None;
        }
    }
    allocation
}

/// What one advancement pass did; feeds the scheduler cycle's due-work
/// accounting and the maintenance response's `one_time_receive` summary.
#[derive(Default)]
pub(in crate::service) struct OneTimeReceiveAdvance {
    /// Any active one-time allocation exists (the stage ran).
    pub tracked: bool,
    pub observed_allocations: usize,
    pub enqueued_sweeps: usize,
    pub retired_allocations: usize,
    pub purged_allocations: usize,
}

impl OneTimeReceiveAdvance {
    /// Work the scheduler should count as "advanced" (a cycle that only
    /// observed balances stays quiet, exactly like a deposits-only refresh
    /// counts via `refreshed`).
    pub fn advanced_work(&self) -> usize {
        self.observed_allocations
            + self.enqueued_sweeps
            + self.retired_allocations
            + self.purged_allocations
    }

    pub fn summary(&self) -> sigillum_api::OneTimeReceiveRunSummary {
        sigillum_api::OneTimeReceiveRunSummary {
            observed_allocations: self.observed_allocations,
            enqueued_sweeps: self.enqueued_sweeps,
            retired_allocations: self.retired_allocations,
            purged_allocations: self.purged_allocations,
        }
    }
}

fn seed_provider_for_allocation<'a>(
    registry: &'a crate::profiles::ProfileRegistry,
    allocation: &TreasuryReceiveAllocation,
) -> Option<&'a EvmProviderProfile> {
    let profile = registry
        .eth_seed_wallets
        .iter()
        .find(|profile| profile.name == allocation.wallet_profile)?;
    registry
        .evm_providers
        .iter()
        .find(|provider| provider.name == profile.provider_profile)
}

impl SigillumService {
    /// Advance every active one-time allocation by one pass: settle confirmed
    /// sweeps (retire + optional purge), observe balances on the refresh
    /// cadence (auto-watch), and enqueue due sweeps.
    ///
    /// Runs under the caller's operation guard (scheduler cycle, maintenance
    /// cycle) and reuses the request-time invariants by construction: sweeps
    /// are `EthSeedNativeSweep` jobs gated under the Sweep execution family
    /// at drain time exactly like operator-enqueued ones, the queue's durable
    /// barriers and never-re-sign rule apply unchanged, and the inventory
    /// store is saved once per pass (crash-equivalent anywhere else).
    pub(in crate::service) async fn advance_one_time_receive_allocations_state(
        &self,
        token: &str,
        queue: &mut QueueState,
        observe_balances: bool,
    ) -> ServiceResult<OneTimeReceiveAdvance> {
        let mut advance = OneTimeReceiveAdvance::default();
        let mut state = load_inventory_state(&self.state.base_dir)?;
        if !state
            .receive_allocations
            .iter()
            .any(|allocation| allocation.one_time && allocation.status == RECEIVE_STATUS_ACTIVE)
        {
            return Ok(advance);
        }
        advance.tracked = true;
        let mut dirty = false;
        let now = now_unix();
        let compartment_id = self.state.active_compartment_id_for(token);

        // ── 1. Settle: retire allocations whose sweep job settled ──
        let confirmed_ids: Vec<String> = state
            .receive_allocations
            .iter()
            .filter(|allocation| {
                allocation.one_time
                    && allocation.status == RECEIVE_STATUS_ACTIVE
                    && tracked_sweep_job_state(allocation, queue)
                        .map(queue_job_sweep_settled_state)
                        .unwrap_or(false)
            })
            .map(|allocation| allocation.id.clone())
            .collect();
        for id in confirmed_ids {
            let Some(position) = state
                .receive_allocations
                .iter()
                .position(|allocation| allocation.id == id)
            else {
                continue;
            };
            let purge = state.receive_allocations[position].purge_after_sweep;
            let counterparty_binding_removed = state.receive_allocations[position]
                .counterparty_id
                .is_some();
            // Same semantics as rotate-retire (index never re-issued — retired
            // records still count in `next_receive_index`), but no
            // replacement is issued: the address was used once, as promised.
            state.receive_allocations[position].status = RECEIVE_STATUS_RETIRED.into();
            state.receive_allocations[position].retired_at_unix = Some(now);
            dirty = true;
            advance.retired_allocations += 1;
            self.record_audit(
                compartment_id,
                AuditEventSpec::TreasuryReceiveRetire {
                    id: id.clone(),
                    reason: ONE_TIME_RETIRE_REASON.into(),
                },
            )?;
            if purge {
                // The 3.2 purge path, automated: the record (and the
                // counterparty binding it carried) leaves the store for good.
                // The observation row written by the auto-watch keeps the
                // index reserved, so purge cannot cause re-issue here.
                state.receive_allocations.remove(position);
                advance.purged_allocations += 1;
                self.record_audit(
                    compartment_id,
                    AuditEventSpec::TreasuryReceivePurge {
                        id: id.clone(),
                        counterparty_binding_removed,
                    },
                )?;
            }
        }

        // ── 2. Auto-watch: observe balances on the refresh cadence ──
        if observe_balances {
            let registry =
                crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
                    super::super::ServiceError::internal(format!(
                        "Failed to load profiles: {error}"
                    ))
                })?;
            let cap = self.state.runtime_policy().receiving_refresh_address_cap;
            let candidates: Vec<String> = state
                .receive_allocations
                .iter()
                .filter(|allocation| {
                    allocation.one_time && allocation.status == RECEIVE_STATUS_ACTIVE
                })
                .take(cap)
                .map(|allocation| allocation.id.clone())
                .collect();
            for id in candidates {
                let Some(allocation) = state
                    .receive_allocations
                    .iter()
                    .find(|allocation| allocation.id == id)
                    .cloned()
                else {
                    continue;
                };
                let Some(provider) = seed_provider_for_allocation(&registry, &allocation) else {
                    continue;
                };
                // Provider errors leave the stored balance in place; the next
                // cadence retries. They never fail the whole pass (same
                // partial-refresh posture as `refresh_receiving_balances`).
                let Ok(native_balance_wei_hex) = self
                    .evm_native_balance_for_provider(
                        provider.compartment_id,
                        provider,
                        &allocation.address,
                        "latest",
                    )
                    .await
                else {
                    continue;
                };
                let activity_state = if quantity_hex_is_nonzero(&native_balance_wei_hex) {
                    WalletAddressActivityState::Funded
                } else {
                    WalletAddressActivityState::Empty
                };
                upsert_address(
                    &mut state.addresses,
                    WalletInventoryAddress {
                        id: random_id(),
                        wallet_family: allocation.wallet_family.clone(),
                        wallet_profile: allocation.wallet_profile.clone(),
                        provider_profile: provider.name.clone(),
                        chain_id: provider.chain_id,
                        address: allocation.address.clone(),
                        derivation_path: allocation.derivation_path.clone(),
                        derivation_pattern: None,
                        account_index: None,
                        address_index: allocation.address_index,
                        activity_state,
                        native_balance_wei_hex,
                        transaction_count: 0,
                        last_activity_block: None,
                        classifications: Vec::new(),
                        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
                        first_seen_at_unix: now,
                        last_checked_at_unix: now,
                    },
                );
                dirty = true;
                advance.observed_allocations += 1;
            }
        }

        // ── 3. Enqueue due sweeps (dedupe + gates + policy + linkage) ──
        let pause_latched = self.state.queue_execution_pause_latched();
        let due_ids: Vec<String> = {
            let all_allocations = state.receive_allocations.clone();
            state
                .receive_allocations
                .iter()
                .filter(|allocation| {
                    allocation.one_time && allocation.status == RECEIVE_STATUS_ACTIVE
                })
                .filter(|allocation| {
                    let job_state = tracked_sweep_job_state(allocation, queue);
                    // Dedupe, mirroring the stealth-deposit sweep rule in its
                    // stricter (manual-enqueue) form: a live OR broadcast OR
                    // confirmed job suppresses re-enqueue. Terminal failure /
                    // parking also suppresses it — the record surfaces
                    // `sweep_failed` / `sweep_attention` and the operator
                    // decides (rotate retires; purge forgets).
                    if let Some(state) = job_state {
                        if is_active_or_completed_queue_state(state)
                            || queue_job_failed_state(state)
                            || queue_job_operator_action_required(state)
                        {
                            return false;
                        }
                    }
                    let balance = observed_allocation_balance(&state, allocation);
                    let linkage_shared =
                        one_time_destination_linkage_shared(allocation, &all_allocations);
                    evaluate_one_time_allocation(
                        allocation,
                        job_state,
                        balance.as_ref(),
                        state.treasury_policy.as_ref(),
                        pause_latched,
                        linkage_shared,
                    )
                    .map(|evaluation| evaluation.sweep_eligible())
                    .unwrap_or(false)
                })
                .map(|allocation| allocation.id.clone())
                .collect()
        };
        for id in due_ids {
            let Some(allocation) = state
                .receive_allocations
                .iter()
                .find(|allocation| allocation.id == id)
                .cloned()
            else {
                continue;
            };
            let Some(destination) = allocation.sweep_destination_address.clone() else {
                continue;
            };
            let job = queued_job(
                random_id(),
                now,
                QueueJobPayload::EthSeedNativeSweep {
                    wallet_profile: allocation.wallet_profile.clone(),
                    address: allocation.address.clone(),
                    derivation_path: allocation.derivation_path.clone(),
                    destination_address: Some(destination),
                    // Re-checked at execution against the spendable balance,
                    // exactly like a manually enqueued seed sweep.
                    min_value_wei_hex: allocation.min_sweep_amount_hex.clone(),
                    gas_limit: None,
                },
            );
            queue.jobs.push(job.clone());
            self.record_audit(
                compartment_id,
                AuditEventSpec::QueueEnqueue {
                    id: job.id.clone(),
                    job_kind: AuditQueueJobKind::EthSeedNativeSweep,
                },
            )?;
            if let Some(allocation) = state
                .receive_allocations
                .iter_mut()
                .find(|allocation| allocation.id == id)
            {
                allocation.sweep_job_id = Some(job.id);
            }
            dirty = true;
            advance.enqueued_sweeps += 1;
        }

        if dirty {
            save_inventory_state(&self.state.base_dir, &state)?;
        }
        Ok(advance)
    }
}

#[cfg(test)]
mod tests {
    use sigillum_api::{QueueJob, TreasuryAllowedDestination};

    use super::super::super::inventory::WALLET_FAMILY_ETH_SEED;
    use super::*;

    const ACTIVE: &str = RECEIVE_STATUS_ACTIVE;
    const DESTINATION: &str = "0x2222222222222222222222222222222222222222";

    fn allocation() -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: "alloc-1".into(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            address_index: 0,
            purpose: "one-time".into(),
            label: None,
            status: ACTIVE.into(),
            created_at_unix: 1,
            retired_at_unix: None,
            counterparty_id: None,
            one_time: true,
            sweep_destination_address: Some(DESTINATION.into()),
            min_sweep_amount_hex: Some("0x10".into()),
            purge_after_sweep: false,
            sweep_job_id: None,
            lifecycle_state: None,
            sweep_blocker: None,
        }
    }

    fn open_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: DESTINATION.into(),
                label: None,
            }],
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: true,
            allow_claim_execution: false,
            allow_gas_topups: false,
            max_gas_topup_wei_hex: None,
            allow_plan_execution: true,
            allow_sweep_execution: true,
            allow_revoke_execution: false,
            allow_exit_execution: false,
            execution_paused: false,
            max_fee_per_gas_cap_hex: None,
            simulation_freshness_secs: 900,
            hot_floor_wei_hex: "0x1".into(),
            hot_target_wei_hex: "0x1".into(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    fn funded() -> [u8; 32] {
        decode_quantity_hex("0x20").unwrap()
    }

    fn evaluate(
        allocation: &TreasuryReceiveAllocation,
        job_state: Option<&str>,
        policy: Option<&TreasuryPolicy>,
    ) -> Option<OneTimeEvaluation> {
        evaluate_one_time_allocation(allocation, job_state, Some(&funded()), policy, false, false)
    }

    #[test]
    fn non_one_time_allocations_have_no_lifecycle() {
        let mut plain = allocation();
        plain.one_time = false;
        assert!(evaluate(&plain, None, Some(&open_policy())).is_none());
    }

    #[test]
    fn lifecycle_follows_the_tracked_job_state() {
        let policy = open_policy();
        for state in [
            "queued",
            "blocked",
            "retrying",
            "prepared",
            "submitted_unknown",
        ] {
            let evaluation = evaluate(&allocation(), Some(state), Some(&policy)).unwrap();
            assert_eq!(
                evaluation.lifecycle_state, LIFECYCLE_SWEEP_QUEUED,
                "{state}"
            );
            assert!(!evaluation.sweep_eligible());
        }
        // `sent` (the legacy family's terminal state) and `confirmed` (W7.4)
        // both mean the sweep settled — retire is imminent.
        for state in ["sent", "confirmed"] {
            let evaluation = evaluate(&allocation(), Some(state), Some(&policy)).unwrap();
            assert_eq!(evaluation.lifecycle_state, LIFECYCLE_SWEPT, "{state}");
            assert!(!evaluation.sweep_eligible());
        }
        let evaluation = evaluate(&allocation(), Some("failed_terminal"), Some(&policy)).unwrap();
        assert_eq!(evaluation.lifecycle_state, LIFECYCLE_WATCHING);
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_SWEEP_FAILED));
        assert!(!evaluation.sweep_eligible());
        let evaluation = evaluate(
            &allocation(),
            Some("operator_action_required"),
            Some(&policy),
        )
        .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_SWEEP_ATTENTION));
    }

    #[test]
    fn retired_allocations_derive_retired() {
        let mut retired = allocation();
        retired.status = "retired".into();
        let evaluation = evaluate(&retired, None, Some(&open_policy())).unwrap();
        assert_eq!(evaluation.lifecycle_state, LIFECYCLE_RETIRED);
    }

    #[test]
    fn balance_and_threshold_gate_eligibility() {
        let policy = open_policy();
        // No observation yet.
        let evaluation =
            evaluate_one_time_allocation(&allocation(), None, None, Some(&policy), false, false)
                .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_AWAITING_BALANCE));

        // Below the 0x10 threshold.
        let dust = decode_quantity_hex("0x8").unwrap();
        let evaluation = evaluate_one_time_allocation(
            &allocation(),
            None,
            Some(&dust),
            Some(&policy),
            false,
            false,
        )
        .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_BELOW_THRESHOLD));
        assert!(!evaluation.sweep_eligible());

        // Exactly at the threshold is eligible.
        let evaluation = evaluate(&allocation(), None, Some(&policy)).unwrap();
        assert_eq!(evaluation.lifecycle_state, LIFECYCLE_WATCHING);
        assert_eq!(evaluation.sweep_blocker, None);
        assert!(evaluation.sweep_eligible());

        // Unset threshold: any nonzero balance; zero is never enough.
        let mut no_minimum = allocation();
        no_minimum.min_sweep_amount_hex = None;
        let tiny = decode_quantity_hex("0x1").unwrap();
        let evaluation = evaluate_one_time_allocation(
            &no_minimum,
            None,
            Some(&tiny),
            Some(&policy),
            false,
            false,
        )
        .unwrap();
        assert!(evaluation.sweep_eligible());
        let zero = [0u8; 32];
        let evaluation = evaluate_one_time_allocation(
            &no_minimum,
            None,
            Some(&zero),
            Some(&policy),
            false,
            false,
        )
        .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_BELOW_THRESHOLD));
    }

    #[test]
    fn gates_and_pause_block_eligibility() {
        // No policy at all: fail closed.
        let evaluation = evaluate(&allocation(), None, None).unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_EXECUTION_GATES));

        // Sweep family closed.
        let mut policy = open_policy();
        policy.allow_sweep_execution = false;
        let evaluation = evaluate(&allocation(), None, Some(&policy)).unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_EXECUTION_GATES));

        // Master gate closed.
        let mut policy = open_policy();
        policy.allow_plan_execution = false;
        let evaluation = evaluate(&allocation(), None, Some(&policy)).unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_EXECUTION_GATES));

        // Latched kill switch blocks even with all gates open.
        let evaluation = evaluate_one_time_allocation(
            &allocation(),
            None,
            Some(&funded()),
            Some(&open_policy()),
            true,
            false,
        )
        .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_EXECUTION_GATES));
        assert!(!evaluation.sweep_eligible());
    }

    #[test]
    fn destination_policy_and_step_cap_block() {
        // Enabled policy whose allowlist does not contain the destination.
        let mut policy = open_policy();
        policy.allowed_destinations = Vec::new();
        let evaluation = evaluate(&allocation(), None, Some(&policy)).unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_DESTINATION_POLICY));

        // Per-step native cap below the observed balance.
        let mut policy = open_policy();
        policy.max_step_native_wei_hex = Some("0x8".into());
        let evaluation = evaluate(&allocation(), None, Some(&policy)).unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_STEP_CAP));
    }

    #[test]
    fn linkage_blocks_only_under_fail_closed_policy() {
        // Shared destination + block on: hard block.
        let policy = open_policy();
        let evaluation = evaluate_one_time_allocation(
            &allocation(),
            None,
            Some(&funded()),
            Some(&policy),
            false,
            true,
        )
        .unwrap();
        assert_eq!(evaluation.sweep_blocker, Some(BLOCKER_CROSS_PARTY_LINKAGE));
        assert!(!evaluation.sweep_eligible());

        // Shared destination + block off: eligible (warn-only posture, same
        // as the stealth deposit sweep path).
        let mut policy = open_policy();
        policy.block_cross_party_linkage = false;
        let evaluation = evaluate_one_time_allocation(
            &allocation(),
            None,
            Some(&funded()),
            Some(&policy),
            false,
            true,
        )
        .unwrap();
        assert!(evaluation.sweep_eligible());
    }

    #[test]
    fn linkage_detection_matches_stealth_identity_rules() {
        let target = allocation();
        let mut other = allocation();
        other.id = "alloc-2".into();
        other.address = "0x3333333333333333333333333333333333333333".into();

        // Both unattributed: distinct addresses are DISTINCT identities
        // (mirroring `two_distinct_unattributed_deposits_to_same_wallet_default_link`)
        // — two unknown payers sweeping to one destination would link.
        assert!(one_time_destination_linkage_shared(
            &target,
            &[other.clone()]
        ));

        // Same address on both sides is the same identity: no linkage.
        let mut same = allocation();
        same.id = "alloc-2".into();
        assert!(!one_time_destination_linkage_shared(&target, &[same]));

        // Different counterparties, same destination: linkage.
        let mut target_bound = allocation();
        target_bound.counterparty_id = Some("party-1".into());
        other.counterparty_id = Some("party-2".into());
        assert!(one_time_destination_linkage_shared(
            &target_bound,
            &[other.clone()]
        ));

        // Same counterparty, same destination: no linkage.
        other.counterparty_id = Some("party-1".into());
        assert!(!one_time_destination_linkage_shared(
            &target_bound,
            &[other.clone()]
        ));

        // Different destination: no linkage.
        let mut other = allocation();
        other.id = "alloc-2".into();
        other.counterparty_id = Some("party-2".into());
        other.sweep_destination_address = Some("0x9999999999999999999999999999999999999999".into());
        assert!(!one_time_destination_linkage_shared(
            &target_bound,
            &[other]
        ));

        // A retired record that never swept links nothing.
        let mut other = allocation();
        other.id = "alloc-2".into();
        other.counterparty_id = Some("party-2".into());
        other.status = "retired".into();
        assert!(!one_time_destination_linkage_shared(
            &target_bound,
            &[other.clone()]
        ));
        // ...but a retired record WITH a sweep job did move funds there.
        other.sweep_job_id = Some("job-1".into());
        assert!(one_time_destination_linkage_shared(&target_bound, &[other]));
    }

    #[test]
    fn observed_balance_uses_the_freshest_matching_row() {
        let mut state = WalletInventoryState::default();
        let row = |balance: &str, chain_id: u64, checked: u64| WalletInventoryAddress {
            id: random_id(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: None,
            account_index: None,
            address_index: 0,
            activity_state: WalletAddressActivityState::Funded,
            native_balance_wei_hex: balance.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: checked,
        };
        assert_eq!(observed_allocation_balance(&state, &allocation()), None);
        state.addresses = vec![row("0x5", 1, 100), row("0x20", 1, 200), row("0x99", 2, 300)];
        assert_eq!(
            observed_allocation_balance(&state, &allocation()),
            Some(decode_quantity_hex("0x20").unwrap())
        );
    }

    #[test]
    fn lifecycle_enrichment_only_marks_one_time_records() {
        let state = WalletInventoryState::default();
        let queue = QueueState::default();
        let enriched = with_one_time_lifecycle(&allocation(), &state, &queue, false);
        assert_eq!(
            enriched.lifecycle_state.as_deref(),
            Some(LIFECYCLE_WATCHING)
        );
        assert_eq!(
            enriched.sweep_blocker.as_deref(),
            Some(BLOCKER_AWAITING_BALANCE)
        );

        let mut plain = allocation();
        plain.one_time = false;
        let enriched = with_one_time_lifecycle(&plain, &state, &queue, false);
        assert_eq!(enriched.lifecycle_state, None);
        assert_eq!(enriched.sweep_blocker, None);

        // A live tracked job derives sweep_queued.
        let mut queued_allocation = allocation();
        queued_allocation.sweep_job_id = Some("job-1".into());
        let queue = QueueState {
            jobs: vec![QueueJob {
                id: "job-1".into(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: 1,
                updated_at_unix: 1,
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthSeedNativeSweep {
                    wallet_profile: "seed-main".into(),
                    address: "0x1111111111111111111111111111111111111111".into(),
                    derivation_path: "m/44'/60'/0'/0/0".into(),
                    destination_address: Some(DESTINATION.into()),
                    min_value_wei_hex: None,
                    gas_limit: None,
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
                receipt: Default::default(),
            }],
        };
        let enriched = with_one_time_lifecycle(&queued_allocation, &state, &queue, false);
        assert_eq!(
            enriched.lifecycle_state.as_deref(),
            Some(LIFECYCLE_SWEEP_QUEUED)
        );
    }
}
