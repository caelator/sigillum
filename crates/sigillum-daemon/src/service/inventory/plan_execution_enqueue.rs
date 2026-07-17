//! W7.2 — controlled enqueue of approved consolidation plan steps.
//!
//! Converts approved, freshly simulated, unblocked plan steps into
//! `PlanStepExecution` queue jobs. Everything is re-validated server-side at
//! enqueue time against CURRENT state — policy gates (W7.1), treasury
//! destination/cap rules, linkage policy, simulation pass + freshness (W6.2),
//! claim gate (W5), gas top-up opt-in (W6.1), idempotency, and dependency
//! ordering (W6.4) — never trusting verdicts recorded at plan generation or
//! approval time. Enqueued jobs stay hard-blocked at drain time
//! ("plan-step execution is not enabled yet") until W7.3 lifts the block.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};
use sigillum_api::{
    ConsolidationPlan, ConsolidationPlanStep, PlanEnqueuePlanRequest, PlanEnqueuePlanResponse,
    PlanEnqueueSkippedStep, PlanEnqueueStepRequest, PlanEnqueueStepResponse, PlanEnqueuedStep,
    PlanStepExecutionPayload, QueueJob, QueueJobPayload, RiskCatalogEntry, TreasuryPolicy,
    WalletAssetKind, WalletPlanStatus, WalletPlanStepAction, WalletPlanStepStatus,
    WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::queue_store::QueueState;

use super::super::helpers::{now_unix, random_id, session_fingerprint_hex};
use super::super::queue::{
    execution_gate_denial, is_active_or_completed_queue_state, mark_job_operator_action_required,
    plan_action_execution_family, queue_job_failed_state, queue_job_operator_action_required,
    queued_job,
};
use super::super::{ServiceError, ServiceResult, SigillumService};
use super::claim_gate::claim_execution_gate_satisfied;
use super::export::{
    dependencies_contain_cycle, selected_step_indexes, stable_topological_selected_indexes,
};
use super::gas_topup::gas_topup_policy_enabled;
use super::planner::{analyze_plan_linkage, plan_policy_violations, summarize_plan_steps};
use super::preflight::{PlanStepPreflight, PlanStepPreflightCall, prepare_plan_step_preflight};
use super::simulation::{DEFAULT_SIMULATION_FRESHNESS_SECS, simulation_is_stale};
use super::support::{load_inventory_state, save_inventory_state};
use super::treasury::{add_u256, policy_blockers_for_step};

// ── Named refusal reasons ──────────────────────────────────────────────────
// Intrinsic reasons mirror export.rs skip-reason naming where they overlap
// ("blocked", "not_approved"); treasury reasons reuse the
// TransactionPolicyAction wire strings ("block_destination", "block_step_cap",
// "block_plan_cap", "block_unsimulated"); dependency reasons follow export's
// `dependency_<mode>:{step_id}` convention.

const REASON_ACTION_NOT_EXECUTABLE: &str = "action_not_executable";
const REASON_NOT_APPROVED: &str = "not_approved";
const REASON_BLOCKED: &str = "blocked";
const REASON_CROSS_PARTY_LINKAGE: &str = "cross_party_linkage";
const REASON_BLOCK_UNSIMULATED: &str = "block_unsimulated";
const REASON_SIMULATION_STALE: &str = "simulation_stale";
const REASON_BLOCK_PLAN_CAP: &str = "block_plan_cap";
const REASON_CLAIM_EXECUTION_DISABLED: &str = "claim_execution_disabled";
const REASON_GAS_TOPUP_DISABLED: &str = "gas_topup_disabled";
const REASON_ALREADY_ENQUEUED: &str = "already_enqueued";
const REASON_OPERATOR_ACTION_REQUIRED: &str = "operator_action_required";
const REASON_PREFLIGHT_PREPARE_ERROR: &str = "preflight_prepare_error";

// ── Evaluation context and verdicts ────────────────────────────────────────

struct EnqueueContext<'a> {
    policy: Option<&'a TreasuryPolicy>,
    risk_catalog: &'a [RiskCatalogEntry],
    freshness_secs: u64,
    now: u64,
    plan_cap_violated: bool,
    /// Step ids the linkage analyzer flagged on a FRESH re-evaluation (only
    /// populated when `block_cross_party_linkage` is enabled).
    linked_step_ids: HashSet<String>,
    /// Freshly recomputed linkage warnings, keyed by step id, for persistence
    /// back onto the plan regardless of the hard-block policy.
    linkage_warnings_by_step_id: HashMap<String, Vec<String>>,
}

/// A refusal with its short named reason (bulk skip list), the full
/// single-step error, and any fail-closed side effects to apply.
struct StepRefusal {
    reason: String,
    error: ServiceError,
    /// A terminally failed prior job that must transition to
    /// `operator_action_required` (E1 semantics).
    park_failed_job_id: Option<String>,
    /// Whether the step's approval must be withdrawn so the operator has to
    /// inspect and re-approve before re-enqueue.
    withdraw_step_approval: bool,
}

impl StepRefusal {
    fn plain(reason: impl Into<String>, error: ServiceError) -> Self {
        Self {
            reason: reason.into(),
            error,
            park_failed_job_id: None,
            withdraw_step_approval: false,
        }
    }
}

/// Build enqueue context from current state: linkage warnings are always
/// recomputed fresh, while cross-party hard blocks stay policy-gated.
fn build_enqueue_context<'a>(
    state: &WalletInventoryState,
    plan: &ConsolidationPlan,
    policy: Option<&'a TreasuryPolicy>,
    risk_catalog: &'a [RiskCatalogEntry],
    now: u64,
) -> EnqueueContext<'a> {
    let freshness_secs = policy
        .map(|policy| policy.simulation_freshness_secs)
        .unwrap_or(DEFAULT_SIMULATION_FRESHNESS_SECS);
    let plan_cap_violated = policy
        .map(|policy| !plan_policy_violations(policy, &plan.steps).is_empty())
        .unwrap_or(false);
    // Same re-evaluation the approval path performs, on a scratch copy:
    // enqueue never trusts linkage verdicts recorded earlier.
    let linkage_state = WalletInventoryState {
        receive_allocations: state.receive_allocations.clone(),
        parties: state.parties.clone(),
        ..Default::default()
    };
    let mut scratch = plan.steps.clone();
    for step in &mut scratch {
        step.linkage_warnings.clear();
    }
    let _ = analyze_plan_linkage(&linkage_state, &mut scratch);
    let linkage_warnings_by_step_id: HashMap<String, Vec<String>> = scratch
        .iter()
        .map(|step| (step.id.clone(), step.linkage_warnings.clone()))
        .collect();
    let linked_step_ids = if policy
        .map(|policy| policy.block_cross_party_linkage)
        .unwrap_or(false)
    {
        scratch
            .iter()
            .filter(|step| !step.linkage_warnings.is_empty())
            .map(|step| step.id.clone())
            .collect()
    } else {
        HashSet::new()
    };
    EnqueueContext {
        policy,
        risk_catalog,
        freshness_secs,
        now,
        plan_cap_violated,
        linked_step_ids,
        linkage_warnings_by_step_id,
    }
}

fn refresh_plan_linkage_warnings(
    state: &mut WalletInventoryState,
    plan_index: usize,
    ctx: &EnqueueContext<'_>,
) -> bool {
    let mut changed = false;
    let plan = &mut state.consolidation_plans[plan_index];
    for step in &mut plan.steps {
        let fresh_warnings = ctx
            .linkage_warnings_by_step_id
            .get(&step.id)
            .cloned()
            .unwrap_or_default();
        if step.linkage_warnings != fresh_warnings {
            step.linkage_warnings = fresh_warnings;
            changed = true;
        }
    }
    if changed {
        plan.updated_at_unix = ctx.now;
    }
    changed
}

/// Locate the queue job recorded for `(plan_id, step_id)`. The persisted
/// step marker is the fast path; the payload scan is the fail-closed recovery
/// guard for a marker lost to a crash between the queue and inventory writes.
fn find_step_job<'q>(
    queue: &'q QueueState,
    plan_id: &str,
    step_id: &str,
    marker: Option<&str>,
) -> Option<&'q QueueJob> {
    if let Some(marker) = marker {
        if let Some(job) = queue.jobs.iter().find(|job| job.id == marker) {
            return Some(job);
        }
    }
    queue.jobs.iter().rev().find(|job| {
        matches!(&job.payload, QueueJobPayload::PlanStepExecution(payload)
            if payload.plan_id == plan_id && payload.step_id == step_id)
    })
}

/// Re-validate one step against CURRENT state and build its queue job.
///
/// Pure with respect to persistent state: side effects a refusal demands are
/// described on the returned [`StepRefusal`] and applied by the caller.
#[allow(clippy::too_many_arguments)]
fn step_enqueue_verdict(
    ctx: &EnqueueContext<'_>,
    plan: &ConsolidationPlan,
    step: &ConsolidationPlanStep,
    queue: &QueueState,
    batch_job_ids: &HashMap<String, String>,
    batch_skipped: &HashMap<String, String>,
) -> Result<QueueJob, Box<StepRefusal>> {
    // 1. The action must be executable at all.
    let Some(family) = plan_action_execution_family(&step.action) else {
        return Err(Box::new(StepRefusal::plain(
            REASON_ACTION_NOT_EXECUTABLE,
            ServiceError::bad_request(format!(
                "{REASON_ACTION_NOT_EXECUTABLE}: {} steps cannot be enqueued for execution",
                step.action.as_str()
            )),
        )));
    };

    // 2. Idempotency: a step is enqueued at most once. A terminally failed
    //    job parks as operator_action_required and withdraws approval; the
    //    operator must inspect and re-approve before re-enqueue (E1).
    if let Some(existing) = find_step_job(queue, &plan.id, &step.id, step.queued_job_id.as_deref())
    {
        let job_id = existing.id.clone();
        let job_state = existing.state.clone();
        if is_active_or_completed_queue_state(&job_state) {
            return Err(Box::new(StepRefusal::plain(
                REASON_ALREADY_ENQUEUED,
                ServiceError::conflict(format!(
                    "{REASON_ALREADY_ENQUEUED}: step {} already has queue job {} in state {}",
                    step.id, job_id, job_state
                )),
            )));
        }
        if queue_job_failed_state(&job_state) {
            return Err(Box::new(StepRefusal {
                reason: REASON_OPERATOR_ACTION_REQUIRED.into(),
                error: ServiceError::conflict(format!(
                    "{REASON_OPERATOR_ACTION_REQUIRED}: step {}'s queue job {} failed; \
                     inspect the failure and re-approve the step before re-enqueue",
                    step.id, job_id
                )),
                park_failed_job_id: Some(job_id),
                withdraw_step_approval: true,
            }));
        }
        if queue_job_operator_action_required(&job_state)
            && !(step.approved && step.status == WalletPlanStepStatus::Approved)
        {
            return Err(Box::new(StepRefusal::plain(
                REASON_OPERATOR_ACTION_REQUIRED,
                ServiceError::conflict(format!(
                    "{REASON_OPERATOR_ACTION_REQUIRED}: step {}'s queue job {} awaits operator \
                     review; re-approve the step before re-enqueue",
                    step.id, job_id
                )),
            )));
        }
        // operator_action_required + re-approved: the operator inspected and
        // re-approved, so a fresh enqueue is allowed (marker moves on).
    }

    // 3. Approval, re-checked against current step state.
    if !(step.approved && step.status == WalletPlanStepStatus::Approved) {
        return Err(Box::new(StepRefusal::plain(
            REASON_NOT_APPROVED,
            ServiceError::forbidden(format!(
                "{REASON_NOT_APPROVED}: step {} must be approved before enqueue",
                step.id
            )),
        )));
    }

    // 4. Any blocker refuses (mirrors export's fail-closed skip).
    if !step.blockers.is_empty() {
        return Err(Box::new(StepRefusal::plain(
            REASON_BLOCKED,
            ServiceError::forbidden(format!(
                "step_blocked: {} is blocked by {}",
                step.id,
                step.blockers.join(",")
            )),
        )));
    }

    // 5. Linkage policy, freshly re-evaluated (same pattern as approval).
    if ctx.linked_step_ids.contains(&step.id) {
        return Err(Box::new(StepRefusal::plain(
            REASON_CROSS_PARTY_LINKAGE,
            ServiceError::forbidden(format!(
                "{REASON_CROSS_PARTY_LINKAGE}: step {}'s destination would publicly link \
                 multiple counterparties; set a distinct per-party destination",
                step.id
            )),
        )));
    }

    // 6. Simulation must have passed (TransactionPolicyAction::BlockUnsimulated).
    if step.simulation_status != WalletSimulationStatus::Passed {
        return Err(Box::new(StepRefusal::plain(
            REASON_BLOCK_UNSIMULATED,
            ServiceError::policy_violation(REASON_BLOCK_UNSIMULATED),
        )));
    }

    // 7. ... and be fresh per the W6.2 window (stale => re-simulate).
    if simulation_is_stale(&step.simulation_evidence, ctx.freshness_secs, ctx.now) {
        return Err(Box::new(StepRefusal::plain(
            REASON_SIMULATION_STALE,
            ServiceError::forbidden(format!(
                "{REASON_SIMULATION_STALE}: simulation evidence for step {} is older than the \
                 {}s freshness window; re-simulate the step before enqueue",
                step.id, ctx.freshness_secs
            )),
        )));
    }

    // 8. W7.1 policy gates: master + per-family + kill switch.
    if let Some(denial) = execution_gate_denial(ctx.policy, family) {
        return Err(Box::new(StepRefusal::plain(
            denial.clone(),
            ServiceError::execution_gate_denied(denial),
        )));
    }

    // 9. Treasury destination-allowlist and step-cap rules, re-evaluated with
    //    the same function the approval path uses.
    if let Some(policy) = ctx.policy {
        let blockers = policy_blockers_for_step(
            policy,
            step.action.as_str(),
            step.destination_address.as_deref(),
            step.asset_kind.as_str(),
            &step.amount_hex,
        );
        if let Some(first) = blockers.first() {
            return Err(Box::new(StepRefusal::plain(
                first.clone(),
                ServiceError::policy_violation(first.clone()),
            )));
        }
    }

    // 10. Plan-wide native cap (TransactionPolicyAction::BlockPlanCap): a
    //     cap-violating plan refuses every step (fail closed).
    if ctx.plan_cap_violated {
        return Err(Box::new(StepRefusal::plain(
            REASON_BLOCK_PLAN_CAP,
            ServiceError::policy_violation(REASON_BLOCK_PLAN_CAP),
        )));
    }

    // 11. Claim steps additionally satisfy W5's claim gate.
    if step.action == WalletPlanStepAction::ClaimReward
        && !claim_execution_gate_satisfied(ctx.policy, ctx.risk_catalog, step)
    {
        return Err(Box::new(StepRefusal::plain(
            REASON_CLAIM_EXECUTION_DISABLED,
            ServiceError::execution_gate_denied(format!(
                "{REASON_CLAIM_EXECUTION_DISABLED}: the claim execution gate is not satisfied \
                 for step {}",
                step.id
            )),
        )));
    }

    // 12. fund_gas steps additionally require W6.1's opt-in.
    if step.action == WalletPlanStepAction::FundGas && !gas_topup_policy_enabled(ctx.policy) {
        return Err(Box::new(StepRefusal::plain(
            REASON_GAS_TOPUP_DISABLED,
            ServiceError::execution_gate_denied(format!(
                "{REASON_GAS_TOPUP_DISABLED}: allow_gas_topups is disabled (step {})",
                step.id
            )),
        )));
    }

    // 13. Dependency ordering (W6.4): every prerequisite must already be
    //     enqueued (this batch or earlier) and not have failed.
    let plan_step_ids: HashSet<&str> = plan.steps.iter().map(|step| step.id.as_str()).collect();
    let mut prerequisite_job_ids = Vec::new();
    for dep_id in &step.depends_on {
        if !plan_step_ids.contains(dep_id.as_str()) {
            let reason = format!("dependency_missing:{dep_id}");
            let error = ServiceError::bad_request(format!(
                "{reason}: prerequisite step is not part of this plan"
            ));
            return Err(Box::new(StepRefusal::plain(reason, error)));
        }
        if let Some(dep_reason) = batch_skipped.get(dep_id.as_str()) {
            let reason = if dep_reason == REASON_BLOCKED {
                format!("dependency_blocked:{dep_id}")
            } else {
                format!("dependency_skipped:{dep_id}")
            };
            let error = ServiceError::conflict(format!(
                "{reason}: prerequisite step was not eligible for enqueue"
            ));
            return Err(Box::new(StepRefusal::plain(reason, error)));
        }
        if let Some(job_id) = batch_job_ids.get(dep_id.as_str()) {
            prerequisite_job_ids.push(job_id.clone());
            continue;
        }
        let dep_marker = plan
            .steps
            .iter()
            .find(|step| &step.id == dep_id)
            .and_then(|step| step.queued_job_id.clone());
        match find_step_job(queue, &plan.id, dep_id, dep_marker.as_deref()) {
            Some(job) if is_active_or_completed_queue_state(&job.state) => {
                prerequisite_job_ids.push(job.id.clone());
            }
            Some(job) => {
                let reason = format!("dependency_job_not_succeeded:{dep_id}:{}", job.state);
                let error = ServiceError::conflict(format!(
                    "{reason}: prerequisite step's queue job is not pending or completed"
                ));
                return Err(Box::new(StepRefusal::plain(reason, error)));
            }
            None => {
                let reason = format!("dependency_not_enqueued:{dep_id}");
                let error = ServiceError::conflict(format!(
                    "{reason}: enqueue the prerequisite step first"
                ));
                return Err(Box::new(StepRefusal::plain(reason, error)));
            }
        }
    }

    // 14. Copy the preflight-prepared call verbatim (never rebuilt later).
    let call = match prepare_plan_step_preflight(step) {
        Ok(PlanStepPreflight::Call(call)) => call,
        Ok(PlanStepPreflight::Unsupported { evidence }) => {
            return Err(Box::new(StepRefusal::plain(
                REASON_ACTION_NOT_EXECUTABLE,
                ServiceError::bad_request(format!(
                    "{REASON_ACTION_NOT_EXECUTABLE}: no local transaction builder for step {} \
                     ({})",
                    step.id,
                    evidence.join(",")
                )),
            )));
        }
        Err(error) => {
            return Err(Box::new(StepRefusal::plain(
                REASON_PREFLIGHT_PREPARE_ERROR,
                ServiceError::bad_request(format!(
                    "{REASON_PREFLIGHT_PREPARE_ERROR}: {}",
                    error.message()
                )),
            )));
        }
    };

    let payload = PlanStepExecutionPayload {
        plan_id: plan.id.clone(),
        step_id: step.id.clone(),
        chain_id: step.chain_id,
        source_address: step.address.clone(),
        derivation_path: step.derivation_path.clone(),
        wallet_family: step.wallet_family.clone(),
        wallet_profile: step.wallet_profile.clone(),
        provider_profile: step.provider_profile.clone(),
        action: step.action.clone(),
        asset_kind: step.asset_kind.clone(),
        asset_address: step.asset_address.clone(),
        amount_hex: step.amount_hex.clone(),
        destination_address: step.destination_address.clone(),
        call_label: call.label.to_string(),
        call_target_address: call.target_address.clone(),
        call_data_hex: call.data_hex.clone(),
        call_value_wei_hex: call.value_hex.clone(),
        simulation_evidence_hash_hex: plan_step_evidence_hash_hex(&plan.id, step, &call),
        fee_basis: parse_evidence_value(&step.simulation_evidence, "fee_basis"),
        max_priority_fee_per_gas_hex: parse_evidence_value(
            &step.simulation_evidence,
            "max_priority_fee_per_gas_hex",
        ),
        max_fee_per_gas_hex: parse_evidence_value(&step.simulation_evidence, "max_fee_per_gas_hex"),
        prerequisite_job_ids,
    };
    Ok(queued_job(
        random_id(),
        ctx.now,
        QueueJobPayload::PlanStepExecution(Box::new(payload)),
    ))
}

// ── Evidence hash ──────────────────────────────────────────────────────────

/// SHA-256 commitment over the prepared call and the step's simulation
/// evidence, hex encoded.
///
/// Canonical input (UTF-8, hashed as one string): `key=value\n` lines in this
/// FIXED order — `plan_id`, `step_id`, `chain_id`, `action`,
/// `source_address`, `derivation_path`, `wallet_family`, `wallet_profile`,
/// `provider_profile`, `asset_kind`, `asset_address` (empty when absent),
/// `amount_hex`, `destination_address` (empty when absent), `call_label`,
/// `call_target_address`, `call_data_hex`, `call_value_wei_hex` (empty when
/// absent) — followed by one `evidence=<item>\n` line per simulation-evidence
/// entry sorted lexicographically.
///
/// W7.3 MUST recompute this hash with the exact same canonicalization from
/// the step's live state and refuse to sign on mismatch (tamper detection
/// between preflight/enqueue and execution).
pub(in crate::service) fn plan_step_evidence_hash_hex(
    plan_id: &str,
    step: &ConsolidationPlanStep,
    call: &PlanStepPreflightCall,
) -> String {
    plan_step_evidence_hash_hex_parts(
        plan_id,
        step,
        call.label,
        &call.target_address,
        &call.data_hex,
        call.value_hex.as_deref(),
    )
}

/// Same canonicalization as [`plan_step_evidence_hash_hex`], but takes the
/// prepared-call fields as plain string parts instead of a
/// [`PlanStepPreflightCall`] (whose type is private to this module). W7.3
/// (`service::queue::plan_steps`) re-verifies the evidence hash from a
/// [`sigillum_api::PlanStepExecutionPayload`]'s own stored call fields
/// against the step's CURRENT live state via this entry point — never by
/// rebuilding the call through [`prepare_plan_step_preflight`].
pub(in crate::service) fn plan_step_evidence_hash_hex_parts(
    plan_id: &str,
    step: &ConsolidationPlanStep,
    call_label: &str,
    call_target_address: &str,
    call_data_hex: &str,
    call_value_wei_hex: Option<&str>,
) -> String {
    let mut canonical = String::new();
    let mut push = |key: &str, value: &str| {
        canonical.push_str(key);
        canonical.push('=');
        canonical.push_str(value);
        canonical.push('\n');
    };
    push("plan_id", plan_id);
    push("step_id", &step.id);
    push("chain_id", &step.chain_id.to_string());
    push("action", step.action.as_str());
    push("source_address", &step.address);
    push("derivation_path", &step.derivation_path);
    push("wallet_family", &step.wallet_family);
    push("wallet_profile", &step.wallet_profile);
    push("provider_profile", &step.provider_profile);
    push("asset_kind", step.asset_kind.as_str());
    push("asset_address", step.asset_address.as_deref().unwrap_or(""));
    push("amount_hex", &step.amount_hex);
    push(
        "destination_address",
        step.destination_address.as_deref().unwrap_or(""),
    );
    push("call_label", call_label);
    push("call_target_address", call_target_address);
    push("call_data_hex", call_data_hex);
    push("call_value_wei_hex", call_value_wei_hex.unwrap_or(""));
    let mut evidence = step.simulation_evidence.clone();
    evidence.sort();
    for item in &evidence {
        push("evidence", item);
    }
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// W7.3 pre-signing guard: recompute the step's evidence hash from its
/// CURRENT live state (looked up by `plan_id`/`step_id`) and the job's own
/// stored prepared-call fields, and compare against
/// `payload.simulation_evidence_hash_hex`. Returns the tamper-naming reason
/// on any mismatch (including a missing plan/step, which cannot be
/// distinguished from tampering and must fail closed the same way) — the
/// caller must treat this as `operator_action_required` and never sign.
pub(in crate::service) fn verify_plan_step_execution_evidence(
    state: &WalletInventoryState,
    payload: &sigillum_api::PlanStepExecutionPayload,
) -> Result<(), String> {
    let plan = state
        .consolidation_plans
        .iter()
        .find(|plan| plan.id == payload.plan_id)
        .ok_or_else(|| {
            format!(
                "evidence_hash_tamper: plan {} not found for step {}",
                payload.plan_id, payload.step_id
            )
        })?;
    let step = plan
        .steps
        .iter()
        .find(|step| step.id == payload.step_id)
        .ok_or_else(|| {
            format!(
                "evidence_hash_tamper: step {} not found in plan {}",
                payload.step_id, payload.plan_id
            )
        })?;
    let recomputed = plan_step_evidence_hash_hex_parts(
        &payload.plan_id,
        step,
        &payload.call_label,
        &payload.call_target_address,
        &payload.call_data_hex,
        payload.call_value_wei_hex.as_deref(),
    );
    if recomputed != payload.simulation_evidence_hash_hex {
        return Err(format!(
            "evidence_hash_tamper: simulation evidence hash mismatch for step {} (plan {}); \
             the step's prepared call or simulation evidence changed since this job was enqueued",
            payload.step_id, payload.plan_id
        ));
    }
    Ok(())
}

fn parse_evidence_value(evidence: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    evidence
        .iter()
        .rev()
        .find_map(|item| item.strip_prefix(&prefix))
        .map(str::to_string)
}

// ── Typed confirmation phrase ──────────────────────────────────────────────

/// Exact phrase the operator must type to bulk-enqueue a plan.
fn plan_enqueue_confirmation_phrase(step_count: usize, total_native_wei_decimal: &str) -> String {
    format!("EXECUTE {step_count} PLAN STEPS TOTAL {total_native_wei_decimal} WEI")
}

/// Decimal rendering of a big-endian u256 (long division by 10).
fn u256_decimal_string(value: &[u8; 32]) -> String {
    let mut digits = Vec::new();
    let mut current = *value;
    loop {
        let mut remainder = 0u32;
        let mut all_zero = true;
        for byte in current.iter_mut() {
            let accumulator = remainder * 256 + u32::from(*byte);
            *byte = (accumulator / 10) as u8;
            remainder = accumulator % 10;
            if *byte != 0 {
                all_zero = false;
            }
        }
        digits.push(char::from(b'0' + remainder as u8));
        if all_zero {
            break;
        }
    }
    digits.iter().rev().collect()
}

fn batch_total_native_wei(steps: &[&ConsolidationPlanStep]) -> [u8; 32] {
    let mut total = [0u8; 32];
    for step in steps {
        if step.asset_kind == WalletAssetKind::Native {
            total = add_u256(
                &total,
                &decode_quantity_hex(&step.amount_hex).unwrap_or([0u8; 32]),
            );
        }
    }
    total
}

// ── Service entry points ───────────────────────────────────────────────────

impl SigillumService {
    pub(crate) async fn enqueue_consolidation_plan_step(
        &self,
        token: Option<&str>,
        body: PlanEnqueueStepRequest,
    ) -> ServiceResult<PlanEnqueueStepResponse> {
        let token = self.require_session(token)?;
        if !body.confirm {
            return Err(ServiceError::bad_request(
                "confirm must be true to enqueue this step",
            ));
        }
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let policy = state.treasury_policy.clone();
        let risk_catalog = state.risk_catalog.clone();
        let now = now_unix();

        let plan_index = state
            .consolidation_plans
            .iter()
            .position(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        let plan = state.consolidation_plans[plan_index].clone();
        if dependencies_contain_cycle(&plan) {
            return Err(ServiceError::bad_request(
                "Consolidation plan step dependencies contain a cycle.",
            ));
        }
        let step_index = plan
            .steps
            .iter()
            .position(|step| step.id == body.step_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan step not found."))?;

        let ctx = build_enqueue_context(&state, &plan, policy.as_ref(), &risk_catalog, now);
        let linkage_warnings_changed = refresh_plan_linkage_warnings(&mut state, plan_index, &ctx);
        match step_enqueue_verdict(
            &ctx,
            &plan,
            &plan.steps[step_index],
            &queue,
            &HashMap::new(),
            &HashMap::new(),
        ) {
            Err(refusal) => {
                let refusal_persisted = self.apply_refusal_side_effects(
                    &mut state, plan_index, step_index, &mut queue, &refusal, now,
                )?;
                if linkage_warnings_changed && !refusal_persisted {
                    save_inventory_state(&self.state.base_dir, &state)?;
                }
                Err(refusal.error)
            }
            Ok(job) => {
                queue.jobs.push(job.clone());
                crate::queue_store::save_queue(&self.state.base_dir, &queue).map_err(|error| {
                    ServiceError::internal(format!("Failed to save queue: {error}"))
                })?;
                let plan = &mut state.consolidation_plans[plan_index];
                plan.steps[step_index].queued_job_id = Some(job.id.clone());
                plan.updated_at_unix = now;
                save_inventory_state(&self.state.base_dir, &state)?;

                self.record_plan_step_enqueue_audit(token, &body.plan_id, &body.step_id, &job)?;
                Ok(PlanEnqueueStepResponse {
                    status: "queued".into(),
                    plan_id: body.plan_id,
                    step_id: body.step_id,
                    job,
                })
            }
        }
    }

    pub(crate) async fn enqueue_consolidation_plan(
        &self,
        token: Option<&str>,
        body: PlanEnqueuePlanRequest,
    ) -> ServiceResult<PlanEnqueuePlanResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let policy = state.treasury_policy.clone();
        let risk_catalog = state.risk_catalog.clone();
        let now = now_unix();

        let plan_index = state
            .consolidation_plans
            .iter()
            .position(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        let plan = state.consolidation_plans[plan_index].clone();
        if dependencies_contain_cycle(&plan) {
            return Err(ServiceError::bad_request(
                "Consolidation plan step dependencies contain a cycle.",
            ));
        }

        // Pass 1 (pure evaluation, no side effects): walk every step in the
        // same stable topological (sequence, index) order export uses and
        // split into would-enqueue jobs and skipped steps with named reasons.
        let ctx = build_enqueue_context(&state, &plan, policy.as_ref(), &risk_catalog, now);
        let selected = selected_step_indexes(&plan, &[]);
        let ordered = stable_topological_selected_indexes(&plan, &selected);
        let mut batch_jobs: Vec<(usize, QueueJob)> = Vec::new();
        let mut batch_job_ids: HashMap<String, String> = HashMap::new();
        let mut skipped_reasons: HashMap<String, String> = HashMap::new();
        let mut skipped = Vec::new();
        let mut refusals: Vec<(usize, Box<StepRefusal>)> = Vec::new();
        for step_index in ordered {
            let step = &plan.steps[step_index];
            match step_enqueue_verdict(&ctx, &plan, step, &queue, &batch_job_ids, &skipped_reasons)
            {
                Ok(job) => {
                    batch_job_ids.insert(step.id.clone(), job.id.clone());
                    batch_jobs.push((step_index, job));
                }
                Err(refusal) => {
                    skipped_reasons.insert(step.id.clone(), refusal.reason.clone());
                    skipped.push(PlanEnqueueSkippedStep {
                        step_id: step.id.clone(),
                        action: step.action.clone(),
                        reason: refusal.reason.clone(),
                    });
                    refusals.push((step_index, refusal));
                }
            }
        }

        if batch_jobs.is_empty() {
            if refresh_plan_linkage_warnings(&mut state, plan_index, &ctx) {
                save_inventory_state(&self.state.base_dir, &state)?;
            }
            let first_reason = skipped
                .first()
                .map(|skip| format!("{} ({})", skip.step_id, skip.reason))
                .unwrap_or_else(|| "plan has no steps".into());
            return Err(ServiceError::bad_request(format!(
                "no_steps_eligible: no plan steps are eligible for enqueue; first skip: {first_reason}"
            )));
        }

        // Typed confirmation: computed fresh from the ACTUAL would-enqueue
        // set; a mismatch changes nothing and returns the expected phrase
        // (message + machine-readable `action`) for UI/CLI to render.
        let batch_steps: Vec<&ConsolidationPlanStep> = batch_jobs
            .iter()
            .map(|(step_index, _)| &plan.steps[*step_index])
            .collect();
        let total_wei = u256_decimal_string(&batch_total_native_wei(&batch_steps));
        let expected = plan_enqueue_confirmation_phrase(batch_jobs.len(), &total_wei);
        if body.confirmation != expected {
            return Err(ServiceError::bad_request_with_action(
                format!(
                    "confirmation_mismatch: type the exact phrase \"{expected}\" to enqueue \
                     {} steps",
                    batch_jobs.len()
                ),
                expected,
            ));
        }

        // Pass 2 (apply): park failed prior jobs / withdraw approvals for the
        // refused steps, then enqueue the eligible ones in order.
        let _ = refresh_plan_linkage_warnings(&mut state, plan_index, &ctx);
        for (step_index, refusal) in &refusals {
            self.apply_refusal_state_changes(
                &mut state,
                plan_index,
                *step_index,
                &mut queue,
                refusal,
                now,
            );
        }
        for (step_index, job) in &batch_jobs {
            queue.jobs.push(job.clone());
            state.consolidation_plans[plan_index].steps[*step_index].queued_job_id =
                Some(job.id.clone());
        }
        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        let plan_state = &mut state.consolidation_plans[plan_index];
        plan_state.updated_at_unix = now;
        plan_state.summary = summarize_plan_steps(&plan_state.steps);
        plan_state.status = plan_status_for_steps(plan_state);
        save_inventory_state(&self.state.base_dir, &state)?;

        let mut enqueued = Vec::new();
        for (step_index, job) in &batch_jobs {
            let step_id = plan.steps[*step_index].id.clone();
            self.record_plan_step_enqueue_audit(token, &body.plan_id, &step_id, job)?;
            enqueued.push(PlanEnqueuedStep {
                step_id,
                job_id: job.id.clone(),
            });
        }
        Ok(PlanEnqueuePlanResponse {
            status: "queued".into(),
            plan_id: body.plan_id,
            enqueued,
            skipped,
        })
    }

    /// Apply and persist a single-step refusal's fail-closed side effects.
    fn apply_refusal_side_effects(
        &self,
        state: &mut WalletInventoryState,
        plan_index: usize,
        step_index: usize,
        queue: &mut QueueState,
        refusal: &StepRefusal,
        now: u64,
    ) -> ServiceResult<bool> {
        if refusal.park_failed_job_id.is_none() && !refusal.withdraw_step_approval {
            return Ok(false);
        }
        self.apply_refusal_state_changes(state, plan_index, step_index, queue, refusal, now);
        crate::queue_store::save_queue(&self.state.base_dir, queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        let plan = &mut state.consolidation_plans[plan_index];
        plan.updated_at_unix = now;
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = plan_status_for_steps(plan);
        save_inventory_state(&self.state.base_dir, state)?;
        Ok(true)
    }

    /// In-memory part of the refusal side effects (persistence is the
    /// caller's responsibility so bulk enqueue saves once).
    fn apply_refusal_state_changes(
        &self,
        state: &mut WalletInventoryState,
        plan_index: usize,
        step_index: usize,
        queue: &mut QueueState,
        refusal: &StepRefusal,
        now: u64,
    ) {
        if let Some(job_id) = refusal.park_failed_job_id.as_deref() {
            if let Some(job) = queue.jobs.iter_mut().find(|job| job.id == job_id) {
                mark_job_operator_action_required(
                    job,
                    format!(
                        "{REASON_OPERATOR_ACTION_REQUIRED}: plan-step job failed; inspect and \
                         re-approve the step before re-enqueue"
                    ),
                    now,
                );
            }
        }
        if refusal.withdraw_step_approval {
            let step = &mut state.consolidation_plans[plan_index].steps[step_index];
            step.approved = false;
            step.status = WalletPlanStepStatus::ReviewRequired;
        }
    }

    fn record_plan_step_enqueue_audit(
        &self,
        token: &str,
        plan_id: &str,
        step_id: &str,
        job: &QueueJob,
    ) -> ServiceResult<()> {
        let action_family = match &job.payload {
            QueueJobPayload::PlanStepExecution(payload) => {
                plan_action_execution_family(&payload.action)
                    .map(|family| family.as_str())
                    .unwrap_or("unknown")
            }
            _ => "unknown",
        };
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanEnqueueStep {
                plan_id: plan_id.into(),
                step_id: step_id.into(),
                job_id: job.id.clone(),
                action_family: action_family.into(),
                session_fingerprint_hex: session_fingerprint_hex(token),
            },
        )
    }
}

fn plan_status_for_steps(plan: &ConsolidationPlan) -> WalletPlanStatus {
    if plan.summary.total_steps == 0 {
        WalletPlanStatus::Empty
    } else if plan.summary.blocked_steps > 0 || !plan.policy_violations.is_empty() {
        WalletPlanStatus::Blocked
    } else if plan.summary.review_required_steps > 0 {
        WalletPlanStatus::ReviewRequired
    } else {
        WalletPlanStatus::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_step() -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: "step_1".into(),
            sequence: 0,
            depends_on: Vec::new(),
            action: "sweep_native".into(),
            status: "approved".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: "native".into(),
            asset_address: None,
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            exit_token0_address: None,
            exit_token1_address: None,
            exit_amount0_min_hex: None,
            exit_amount1_min_hex: None,
            exit_deadline_unix: None,
            amount_hex: "0x2540be400".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "passed".into(),
            simulation_evidence: vec![
                "fee_basis=static_profile".into(),
                "max_priority_fee_per_gas_hex=0x1".into(),
                "max_fee_per_gas_hex=0x2".into(),
                "simulated_at_unix=100".into(),
            ],
            risk_level: "low".into(),
            blockers: Vec::new(),
            linkage_warnings: Vec::new(),
            auto_eligible: true,
            approved: true,
            queued_job_id: None,
        }
    }

    fn sample_call() -> PlanStepPreflightCall {
        match prepare_plan_step_preflight(&sample_step()).unwrap() {
            PlanStepPreflight::Call(call) => call,
            PlanStepPreflight::Unsupported { .. } => panic!("expected call"),
        }
    }

    #[test]
    fn evidence_hash_is_deterministic_and_order_insensitive_for_evidence() {
        let step = sample_step();
        let call = sample_call();
        let first = plan_step_evidence_hash_hex("plan_1", &step, &call);
        let second = plan_step_evidence_hash_hex("plan_1", &step, &call);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);

        let mut reordered = step.clone();
        reordered.simulation_evidence.reverse();
        assert_eq!(
            plan_step_evidence_hash_hex("plan_1", &reordered, &call),
            first
        );
    }

    #[test]
    fn evidence_hash_detects_tampered_call_and_evidence() {
        let step = sample_step();
        let call = sample_call();
        let baseline = plan_step_evidence_hash_hex("plan_1", &step, &call);

        let mut tampered_call = call.clone();
        tampered_call.target_address = "0x8888888888888888888888888888888888888888".into();
        assert_ne!(
            plan_step_evidence_hash_hex("plan_1", &step, &tampered_call),
            baseline
        );

        let mut tampered_step = step.clone();
        tampered_step.simulation_evidence.push("extra=1".into());
        assert_ne!(
            plan_step_evidence_hash_hex("plan_1", &tampered_step, &call),
            baseline
        );

        assert_ne!(
            plan_step_evidence_hash_hex("plan_2", &step, &call),
            baseline
        );
    }

    /// Every committed field emits its own key line, so embedded delimiters in
    /// a value cannot erase or forge the following field commitment.
    #[test]
    fn evidence_hash_resists_field_delimiter_injection() {
        let base = sample_step();
        let call = sample_call();

        let mut case_x = base.clone();
        case_x.asset_address = None;
        case_x.amount_hex = "0x1".into();

        let mut case_y = base.clone();
        case_y.asset_address = Some("\namount_hex=0x1".to_string());
        case_y.amount_hex = String::new();

        let h_x = plan_step_evidence_hash_hex("plan_1", &case_x, &call);
        let h_y = plan_step_evidence_hash_hex("plan_1", &case_y, &call);

        assert_ne!(
            h_x, h_y,
            "delimiter injection cannot forge a matching evidence commitment"
        );
        assert_eq!(h_x.len(), 64);
        assert_eq!(h_y.len(), 64);
    }

    #[test]
    fn u256_decimal_string_renders_zero_small_and_large_values() {
        assert_eq!(u256_decimal_string(&[0u8; 32]), "0");
        assert_eq!(
            u256_decimal_string(&decode_quantity_hex("0x2540be400").unwrap()),
            "10000000000"
        );
        assert_eq!(
            u256_decimal_string(&decode_quantity_hex("0xde0b6b3a7640000").unwrap()),
            "1000000000000000000"
        );
        assert_eq!(
            u256_decimal_string(&[0xff; 32]),
            format!("{}", {
                // 2^256 - 1
                "115792089237316195423570985008687907853269984665640564039457584007913129639935"
            })
        );
    }

    #[test]
    fn confirmation_phrase_includes_step_count_and_total_wei() {
        assert_eq!(
            plan_enqueue_confirmation_phrase(2, "10000000000"),
            "EXECUTE 2 PLAN STEPS TOTAL 10000000000 WEI"
        );
    }

    #[test]
    fn batch_total_only_counts_native_steps() {
        let native = sample_step();
        let mut erc20 = sample_step();
        erc20.id = "step_2".into();
        erc20.action = "sweep_erc20".into();
        erc20.asset_kind = "erc20".into();
        erc20.asset_address = Some("0x2222222222222222222222222222222222222222".into());
        let total = batch_total_native_wei(&[&native, &erc20]);
        assert_eq!(u256_decimal_string(&total), "10000000000");
    }

    fn sample_plan(step: ConsolidationPlanStep) -> ConsolidationPlan {
        ConsolidationPlan {
            id: "plan_1".into(),
            status: "approved".into(),
            chain_id: 1,
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            origin: None,
            created_at_unix: 1,
            updated_at_unix: 1,
            summary: sigillum_api::ConsolidationPlanSummary {
                total_steps: 1,
                blocked_steps: 0,
                review_required_steps: 0,
                approved_steps: 1,
                executable_steps: 1,
                value_items: 1,
            },
            policy_violations: Vec::new(),
            linkage_findings: Vec::new(),
            steps: vec![step],
        }
    }

    fn sample_payload_for(
        step: &ConsolidationPlanStep,
        call: &PlanStepPreflightCall,
    ) -> PlanStepExecutionPayload {
        PlanStepExecutionPayload {
            plan_id: "plan_1".into(),
            step_id: step.id.clone(),
            chain_id: step.chain_id,
            source_address: step.address.clone(),
            derivation_path: step.derivation_path.clone(),
            wallet_family: step.wallet_family.clone(),
            wallet_profile: step.wallet_profile.clone(),
            provider_profile: step.provider_profile.clone(),
            action: step.action.clone(),
            asset_kind: step.asset_kind.clone(),
            asset_address: step.asset_address.clone(),
            amount_hex: step.amount_hex.clone(),
            destination_address: step.destination_address.clone(),
            call_label: call.label.to_string(),
            call_target_address: call.target_address.clone(),
            call_data_hex: call.data_hex.clone(),
            call_value_wei_hex: call.value_hex.clone(),
            simulation_evidence_hash_hex: plan_step_evidence_hash_hex("plan_1", step, call),
            fee_basis: None,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            prerequisite_job_ids: Vec::new(),
        }
    }

    #[test]
    fn verify_plan_step_execution_evidence_accepts_unchanged_step() {
        let step = sample_step();
        let call = sample_call();
        let payload = sample_payload_for(&step, &call);
        let state = WalletInventoryState {
            consolidation_plans: vec![sample_plan(step)],
            ..Default::default()
        };

        assert!(verify_plan_step_execution_evidence(&state, &payload).is_ok());
    }

    #[test]
    fn verify_plan_step_execution_evidence_rejects_tampered_call_field() {
        let step = sample_step();
        let call = sample_call();
        let mut payload = sample_payload_for(&step, &call);
        // Tamper with the persisted job's own call field after enqueue (as
        // if the queue store were edited out of band).
        payload.call_target_address = "0x8888888888888888888888888888888888888888".into();
        let state = WalletInventoryState {
            consolidation_plans: vec![sample_plan(step)],
            ..Default::default()
        };

        let error = verify_plan_step_execution_evidence(&state, &payload).unwrap_err();
        assert!(error.starts_with("evidence_hash_tamper:"), "{error}");
    }

    #[test]
    fn verify_plan_step_execution_evidence_rejects_step_that_changed_since_enqueue() {
        let step = sample_step();
        let call = sample_call();
        let payload = sample_payload_for(&step, &call);
        let mut changed_step = step.clone();
        // Re-simulation (or any other live-state change) after enqueue
        // changes the evidence array, so the recomputed hash no longer
        // matches what was committed at enqueue time.
        changed_step
            .simulation_evidence
            .push("resimulated=true".into());
        let state = WalletInventoryState {
            consolidation_plans: vec![sample_plan(changed_step)],
            ..Default::default()
        };

        let error = verify_plan_step_execution_evidence(&state, &payload).unwrap_err();
        assert!(error.starts_with("evidence_hash_tamper:"), "{error}");
    }

    #[test]
    fn verify_plan_step_execution_evidence_rejects_missing_plan_or_step() {
        let step = sample_step();
        let call = sample_call();
        let payload = sample_payload_for(&step, &call);

        let empty_state = WalletInventoryState::default();
        let error = verify_plan_step_execution_evidence(&empty_state, &payload).unwrap_err();
        assert!(error.starts_with("evidence_hash_tamper:"), "{error}");
        assert!(error.contains("plan"), "{error}");

        let mut other_plan = sample_plan(step);
        other_plan.steps.clear();
        let state = WalletInventoryState {
            consolidation_plans: vec![other_plan],
            ..Default::default()
        };
        let error = verify_plan_step_execution_evidence(&state, &payload).unwrap_err();
        assert!(error.starts_with("evidence_hash_tamper:"), "{error}");
        assert!(error.contains("step"), "{error}");
    }

    #[test]
    fn parse_evidence_value_returns_last_match() {
        let evidence = vec![
            "fee_basis=static_profile".into(),
            "fee_basis=estimated".into(),
        ];
        assert_eq!(
            parse_evidence_value(&evidence, "fee_basis").as_deref(),
            Some("estimated")
        );
        assert_eq!(parse_evidence_value(&evidence, "missing"), None);
    }
}
