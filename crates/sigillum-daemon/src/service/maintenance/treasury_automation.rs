use std::collections::BTreeMap;

use axum::http::StatusCode;
use sigillum_api::{
    ConsolidationPlan, ConsolidationPlanApproveRequest, ConsolidationPlanSimulateRequest,
    ConsolidationPlanStep, MaintenanceFailureBreakdown, PlanEnqueueStepRequest,
    TreasuryAutomationRunSummary, WalletAssetKind, WalletInventoryAddress, WalletPlanStatus,
    WalletPlanStepAction, WalletPlanStepStatus, WalletSignerStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;
use crate::queue_store::QueueState;

use super::super::helpers::{compare_u256, now_unix, random_id, subtract_u256};
use super::super::inventory::WALLET_FAMILY_ETH_SEED;
use super::super::inventory::planner::{
    analyze_plan_linkage, apply_linkage_blockers, apply_policy_blockers_to_step,
    assign_step_ordering, plan_policy_violations, summarize_plan_steps,
};
use super::super::queue::{ExecutionFamily, execution_gate_denial, is_active_queue_state};
use super::super::{ServiceError, ServiceResult, SigillumService};

const AUTOMATION_ORIGIN: &str = "treasury_automation";
const MAX_SKIPPED_REASONS: usize = 8;

pub(in crate::service) struct TreasuryAutomationOutcome {
    pub summary: TreasuryAutomationRunSummary,
    pub failures: MaintenanceFailureBreakdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutomationCandidate {
    step: ConsolidationPlanStep,
}

#[derive(Default)]
struct CandidateSelection {
    candidates: Vec<AutomationCandidate>,
    skipped_steps: usize,
    skipped_reasons: Vec<String>,
}

impl SigillumService {
    pub(in crate::service) async fn run_treasury_automation(
        &self,
        token: &str,
    ) -> ServiceResult<Option<TreasuryAutomationOutcome>> {
        let state = load_inventory(&self.state.base_dir)?;
        let Some(policy) = state.treasury_policy.clone() else {
            return Ok(None);
        };
        if !policy.enabled || !policy.allow_treasury_automation {
            return Ok(None);
        }
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let queue = load_queue(&self.state.base_dir)?;
        let mut selection = select_automation_candidates(&state, &registry, &queue);
        if selection.candidates.is_empty() && selection.skipped_steps == 0 {
            return Ok(Some(TreasuryAutomationOutcome {
                summary: TreasuryAutomationRunSummary::default(),
                failures: MaintenanceFailureBreakdown::default(),
            }));
        }

        let generated_plans = {
            let _guard = self.state.operation_guard().await;
            let mut current = load_inventory(&self.state.base_dir)?;
            let candidates = std::mem::take(&mut selection.candidates);
            let plans = persist_generated_plans(&mut current, candidates, now_unix());
            if !plans.is_empty() {
                save_inventory(&self.state.base_dir, &current)?;
            }
            plans
        };
        for plan in &generated_plans {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::WalletConsolidationPlanGenerate {
                    id: plan.id.clone(),
                    steps: plan.summary.total_steps,
                    blocked: plan.summary.blocked_steps,
                },
            )?;
        }

        let mut failures = MaintenanceFailureBreakdown::default();
        let mut simulation_failed_plan_ids = Vec::new();
        for plan in &generated_plans {
            if let Err(error) = self
                .simulate_consolidation_plan(
                    Some(token),
                    ConsolidationPlanSimulateRequest {
                        plan_id: plan.id.clone(),
                        step_ids: Vec::new(),
                    },
                )
                .await
            {
                if plan.steps.iter().any(|step| !step.blockers.is_empty()) {
                    classify_reason("blocked", &mut failures);
                } else {
                    classify_automation_failure(&error, &mut failures);
                }
                simulation_failed_plan_ids.push(plan.id.clone());
                selection.skipped_steps += plan.steps.len();
                push_reason(&mut selection.skipped_reasons, "simulation_failed");
            }
        }

        let generated_plan_ids = generated_plans
            .iter()
            .map(|plan| plan.id.clone())
            .collect::<Vec<_>>();
        let eligible = {
            let _guard = self.state.operation_guard().await;
            let mut current = load_inventory(&self.state.base_dir)?;
            let eligible = mark_auto_eligible_steps(
                &mut current,
                &generated_plan_ids,
                &simulation_failed_plan_ids,
                &mut selection,
                &mut failures,
            );
            save_inventory(&self.state.base_dir, &current)?;
            eligible
        };

        let mut enqueued_steps = 0usize;
        for (plan_id, step_id) in eligible {
            match self
                .approve_consolidation_plan(
                    Some(token),
                    ConsolidationPlanApproveRequest {
                        plan_id: plan_id.clone(),
                        step_ids: vec![step_id.clone()],
                    },
                )
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    classify_automation_failure(&error, &mut failures);
                    selection.skipped_steps += 1;
                    push_reason(&mut selection.skipped_reasons, error_label(&error));
                    continue;
                }
            }
            match self
                .enqueue_consolidation_plan_step(
                    Some(token),
                    PlanEnqueueStepRequest {
                        plan_id,
                        step_id,
                        confirm: true,
                    },
                )
                .await
            {
                Ok(_) => enqueued_steps += 1,
                Err(error) => {
                    classify_automation_failure(&error, &mut failures);
                    selection.skipped_steps += 1;
                    push_reason(&mut selection.skipped_reasons, error_label(&error));
                }
            }
        }

        let generated_steps = generated_plans
            .iter()
            .map(|plan| plan.steps.len())
            .sum::<usize>();
        let summary = TreasuryAutomationRunSummary {
            generated_steps,
            enqueued_steps,
            skipped_steps: selection.skipped_steps,
            skipped_reasons: selection.skipped_reasons,
        };
        if generated_steps + summary.enqueued_steps + summary.skipped_steps > 0 {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryAutomationRun {
                    generated: summary.generated_steps,
                    enqueued: summary.enqueued_steps,
                    skipped: summary.skipped_steps,
                },
            )?;
        }
        Ok(Some(TreasuryAutomationOutcome { summary, failures }))
    }
}

fn load_inventory(base_dir: &std::path::Path) -> ServiceResult<WalletInventoryState> {
    crate::inventory::load_wallet_inventory(base_dir).map_err(|error| {
        ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
    })
}

fn save_inventory(base_dir: &std::path::Path, state: &WalletInventoryState) -> ServiceResult<()> {
    crate::inventory::save_wallet_inventory(base_dir, state).map_err(|error| {
        ServiceError::internal(format!("Failed to save wallet inventory: {error}"))
    })
}

fn load_queue(base_dir: &std::path::Path) -> ServiceResult<QueueState> {
    crate::queue_store::load_queue(base_dir)
        .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))
}

fn select_automation_candidates(
    state: &WalletInventoryState,
    registry: &crate::profiles::ProfileRegistry,
    queue: &QueueState,
) -> CandidateSelection {
    let mut selection = CandidateSelection::default();
    let Some(policy) = state
        .treasury_policy
        .as_ref()
        .filter(|policy| policy.enabled && policy.allow_treasury_automation)
    else {
        return selection;
    };
    let Ok(floor) = decode_quantity_hex(&policy.hot_floor_wei_hex) else {
        return selection;
    };
    let Ok(target) = decode_quantity_hex(&policy.hot_target_wei_hex) else {
        return selection;
    };
    let overflow = policy
        .hot_overflow_wei_hex
        .as_deref()
        .and_then(|value| decode_quantity_hex(value).ok());

    for profile in &registry.eth_seed_wallets {
        let Some(hot_address) = profile.hot_address.as_deref() else {
            continue;
        };
        let Some(treasury_address) = profile.treasury_address.as_deref() else {
            continue;
        };
        let hot_entries = freshest_entries_by_chain(state, &profile.name, hot_address);
        let treasury_entries = freshest_entries_by_chain(state, &profile.name, treasury_address);
        for (chain_id, hot_entry) in hot_entries {
            let Some(hot_balance) = decode_quantity_hex(&hot_entry.native_balance_wei_hex).ok()
            else {
                continue;
            };
            if let Some(reason) = candidate_is_blocked_by_existing_automation(
                state,
                queue,
                &profile.name,
                chain_id,
                hot_entry.last_checked_at_unix,
            ) {
                selection.skipped_steps += 1;
                push_reason(&mut selection.skipped_reasons, reason);
                continue;
            }
            if overflow
                .as_ref()
                .is_some_and(|overflow| compare_u256(&hot_balance, overflow).is_gt())
            {
                if compare_u256(&hot_balance, &target).is_gt() {
                    let amount = subtract_u256(&hot_balance, &target);
                    selection.candidates.push(AutomationCandidate {
                        step: automation_step(&hot_entry, treasury_address, &amount),
                    });
                }
            } else if compare_u256(&hot_balance, &floor).is_lt() {
                let Some(treasury_entry) = treasury_entries.get(&chain_id) else {
                    selection.skipped_steps += 1;
                    push_reason(
                        &mut selection.skipped_reasons,
                        "refill_source_not_inventoried",
                    );
                    continue;
                };
                if compare_u256(&target, &hot_balance).is_gt() {
                    let amount = subtract_u256(&target, &hot_balance);
                    selection.candidates.push(AutomationCandidate {
                        step: automation_step(treasury_entry, hot_address, &amount),
                    });
                }
            }
        }
    }
    selection
}

fn freshest_entries_by_chain(
    state: &WalletInventoryState,
    wallet_profile: &str,
    address: &str,
) -> BTreeMap<u64, WalletInventoryAddress> {
    let mut entries = BTreeMap::<u64, WalletInventoryAddress>::new();
    for entry in state.addresses.iter().filter(|entry| {
        entry.wallet_profile == wallet_profile && entry.address.eq_ignore_ascii_case(address)
    }) {
        match entries.get(&entry.chain_id) {
            Some(existing) if existing.last_checked_at_unix >= entry.last_checked_at_unix => {}
            _ => {
                entries.insert(entry.chain_id, entry.clone());
            }
        }
    }
    entries
}

fn automation_step(
    source: &WalletInventoryAddress,
    destination_address: &str,
    amount: &[u8; 32],
) -> ConsolidationPlanStep {
    ConsolidationPlanStep {
        id: random_id(),
        sequence: 0,
        depends_on: Vec::new(),
        action: WalletPlanStepAction::SweepNative,
        status: WalletPlanStepStatus::ReviewRequired,
        wallet_family: WALLET_FAMILY_ETH_SEED.into(),
        wallet_profile: source.wallet_profile.clone(),
        provider_profile: source.provider_profile.clone(),
        chain_id: source.chain_id,
        address: source.address.clone(),
        derivation_path: source.derivation_path.clone(),
        asset_kind: WalletAssetKind::Native,
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
        amount_hex: encode_quantity_hex(amount),
        destination_address: Some(destination_address.to_string()),
        signer_status: WalletSignerStatus::Available,
        simulation_status: WalletSimulationStatus::Required,
        simulation_evidence: Vec::new(),
        risk_level: "low".into(),
        blockers: Vec::new(),
        linkage_warnings: Vec::new(),
        auto_eligible: false,
        approved: false,
        queued_job_id: None,
    }
}

fn candidate_is_blocked_by_existing_automation(
    state: &WalletInventoryState,
    queue: &QueueState,
    wallet_profile: &str,
    chain_id: u64,
    hot_last_checked_at_unix: u64,
) -> Option<&'static str> {
    for plan in state
        .consolidation_plans
        .iter()
        .filter(|plan| plan.origin.as_deref() == Some(AUTOMATION_ORIGIN))
    {
        for step in plan
            .steps
            .iter()
            .filter(|step| step.wallet_profile == wallet_profile && step.chain_id == chain_id)
        {
            if matches!(
                step.status,
                WalletPlanStepStatus::ReviewRequired
                    | WalletPlanStepStatus::Approved
                    | WalletPlanStepStatus::Blocked
            ) && step.queued_job_id.is_none()
            {
                return Some("automation_step_open");
            }
            let Some(job_id) = step.queued_job_id.as_deref() else {
                continue;
            };
            let Some(job) = queue.jobs.iter().find(|job| job.id == job_id) else {
                return Some("automation_step_open");
            };
            if is_active_queue_state(&job.state) {
                return Some("automation_step_open");
            }
            if job.updated_at_unix >= hot_last_checked_at_unix {
                return Some("balance_not_reobserved");
            }
        }
    }
    None
}

fn persist_generated_plans(
    state: &mut WalletInventoryState,
    candidates: Vec<AutomationCandidate>,
    now: u64,
) -> Vec<ConsolidationPlan> {
    let policy = state.treasury_policy.clone();
    let mut steps_by_chain = BTreeMap::<u64, Vec<ConsolidationPlanStep>>::new();
    for candidate in candidates {
        steps_by_chain
            .entry(candidate.step.chain_id)
            .or_default()
            .push(candidate.step);
    }
    let mut generated = Vec::new();
    for (chain_id, mut steps) in steps_by_chain {
        assign_step_ordering(&mut steps);
        if let Some(policy) = policy.as_ref() {
            for step in &mut steps {
                apply_policy_blockers_to_step(policy, step);
            }
        }
        let policy_violations = policy
            .as_ref()
            .map(|policy| plan_policy_violations(policy, &steps))
            .unwrap_or_default();
        let linkage_findings = analyze_plan_linkage(state, &mut steps);
        if policy
            .as_ref()
            .map(|policy| policy.block_cross_party_linkage)
            .unwrap_or(false)
        {
            apply_linkage_blockers(&mut steps);
        }
        let summary = summarize_plan_steps(&steps);
        let status = if summary.total_steps == 0 {
            WalletPlanStatus::Empty
        } else if summary.blocked_steps > 0 || !policy_violations.is_empty() {
            WalletPlanStatus::Blocked
        } else {
            WalletPlanStatus::ReviewRequired
        };
        generated.push(ConsolidationPlan {
            id: random_id(),
            status,
            chain_id,
            destination_address: None,
            origin: Some(AUTOMATION_ORIGIN.into()),
            created_at_unix: now,
            updated_at_unix: now,
            summary,
            policy_violations,
            linkage_findings,
            steps,
        });
    }
    state.consolidation_plans.extend(generated.iter().cloned());
    generated
}

fn mark_auto_eligible_steps(
    state: &mut WalletInventoryState,
    generated_plan_ids: &[String],
    simulation_failed_plan_ids: &[String],
    selection: &mut CandidateSelection,
    failures: &mut MaintenanceFailureBreakdown,
) -> Vec<(String, String)> {
    let policy = state.treasury_policy.clone();
    let mut eligible = Vec::new();
    let gate_denial = execution_gate_denial(policy.as_ref(), ExecutionFamily::Sweep);
    for plan in state.consolidation_plans.iter_mut().filter(|plan| {
        plan.origin.as_deref() == Some(AUTOMATION_ORIGIN)
            && generated_plan_ids.iter().any(|id| id == &plan.id)
            && !simulation_failed_plan_ids.iter().any(|id| id == &plan.id)
    }) {
        for step in &mut plan.steps {
            if step.queued_job_id.is_some() || step.auto_eligible {
                continue;
            }
            let reason = if !policy
                .as_ref()
                .map(|policy| policy.enabled && policy.allow_treasury_automation)
                .unwrap_or(false)
            {
                Some("automation_disabled")
            } else if !step.blockers.is_empty() {
                Some("blocked")
            } else if step.simulation_status != WalletSimulationStatus::Passed {
                Some("simulation_not_passed")
            } else {
                gate_denial.as_deref()
            };
            if let Some(reason) = reason {
                selection.skipped_steps += 1;
                push_reason(&mut selection.skipped_reasons, reason);
                classify_reason(reason, failures);
                continue;
            }
            step.auto_eligible = true;
            eligible.push((plan.id.clone(), step.id.clone()));
        }
        plan.updated_at_unix = now_unix();
    }
    eligible
}

pub(in crate::service) fn merge_failure_breakdowns(
    left: &mut MaintenanceFailureBreakdown,
    right: &MaintenanceFailureBreakdown,
) {
    left.provider_error += right.provider_error;
    left.policy_block += right.policy_block;
    left.insufficient_gas += right.insufficient_gas;
    left.validation += right.validation;
    left.unknown += right.unknown;
    left.on_chain_revert += right.on_chain_revert;
    left.broadcast_rejected += right.broadcast_rejected;
    left.receipt_timeout += right.receipt_timeout;
}

fn classify_automation_failure(error: &ServiceError, failures: &mut MaintenanceFailureBreakdown) {
    if let Some(action) = error.action() {
        classify_reason(action, failures);
        return;
    }
    if error.status() == StatusCode::FORBIDDEN {
        failures.policy_block += 1;
        return;
    }
    classify_reason(error.message(), failures);
}

fn classify_reason(reason: &str, failures: &mut MaintenanceFailureBreakdown) {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("provider") || reason.contains("rpc") || reason.contains("eth_call") {
        failures.provider_error += 1;
    } else if reason.contains("insufficient_gas") {
        failures.insufficient_gas += 1;
    } else if reason.contains("simulation")
        || reason.contains("validation")
        || reason.contains("unsimulated")
    {
        failures.validation += 1;
    } else if reason.contains("gate")
        || reason.contains("policy")
        || reason.contains("linkage")
        || reason.contains("cap")
        || reason.contains("forbidden")
        || reason.contains("blocked")
    {
        failures.policy_block += 1;
    } else {
        failures.unknown += 1;
    }
}

fn error_label(error: &ServiceError) -> &str {
    error.action().unwrap_or_else(|| error.message())
}

fn push_reason(reasons: &mut Vec<String>, reason: impl AsRef<str>) {
    if reasons.len() >= MAX_SKIPPED_REASONS {
        return;
    }
    let reason = reason.as_ref();
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

fn encode_quantity_hex(value: &[u8; 32]) -> String {
    let Some(start) = value.iter().position(|byte| *byte != 0) else {
        return "0x0".into();
    };
    let mut encoded = String::from("0x");
    let mut rendered = false;
    for byte in &value[start..] {
        if rendered {
            encoded.push_str(&format!("{byte:02x}"));
        } else {
            encoded.push_str(&format!("{byte:x}"));
            rendered = true;
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use sigillum_api::{
        EthSeedWalletProfile, QueueJob, QueueJobPayload, QueueJobReceipt, TreasuryPolicy,
        WalletAddressActivityState,
    };

    use super::*;

    fn policy(enabled: bool, automation: bool) -> TreasuryPolicy {
        TreasuryPolicy {
            enabled,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
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
            hot_floor_wei_hex: "0x5".into(),
            hot_target_wei_hex: "0xa".into(),
            hot_overflow_wei_hex: Some("0x14".into()),
            allow_treasury_automation: automation,
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    fn profile(name: &str) -> EthSeedWalletProfile {
        EthSeedWalletProfile {
            name: name.into(),
            label: None,
            project_account: 0,
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            word_count: 12,
            mnemonic_secret_key: "wallet".into(),
            account_path: "m/44'/60'/0'".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "xpub".into(),
            first_receive_address: "0x0000000000000000000000000000000000000001".into(),
            default_destination_address: None,
            control_xpub: None,
            sponsor_address: None,
            hot_address: Some("0x1111111111111111111111111111111111111111".into()),
            treasury_address: Some("0x2222222222222222222222222222222222222222".into()),
            execution_enabled: true,
        }
    }

    fn address(
        profile: &str,
        chain_id: u64,
        addr: &str,
        balance: &str,
        checked: u64,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: random_id(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: profile.into(),
            provider_profile: format!("provider-{chain_id}"),
            chain_id,
            address: addr.into(),
            derivation_path: "m/44'/60'/0'/1/1".into(),
            derivation_pattern: None,
            account_index: Some(0),
            address_index: 1,
            activity_state: WalletAddressActivityState::Funded,
            native_balance_wei_hex: balance.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: checked,
        }
    }

    fn registry(profile_name: &str) -> crate::profiles::ProfileRegistry {
        crate::profiles::ProfileRegistry {
            eth_seed_wallets: vec![profile(profile_name)],
            ..Default::default()
        }
    }

    fn state_with_policy(policy: TreasuryPolicy) -> WalletInventoryState {
        WalletInventoryState {
            treasury_policy: Some(policy),
            ..Default::default()
        }
    }

    fn queue_job(id: &str, state: &str, updated_at_unix: u64) -> QueueJob {
        QueueJob {
            id: id.into(),
            state: state.into(),
            attempts: 0,
            created_at_unix: 1,
            updated_at_unix,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "stealth".into(),
                stealth_address: "0x3333333333333333333333333333333333333333".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                value_wei_hex: "0x1".into(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
                stealth_hash_convention: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: QueueJobReceipt::default(),
        }
    }

    fn existing_plan(status: WalletPlanStepStatus, job_id: Option<&str>) -> ConsolidationPlan {
        let mut step = automation_step(
            &address(
                "seed-a",
                1,
                "0x1111111111111111111111111111111111111111",
                "0x20",
                100,
            ),
            "0x2222222222222222222222222222222222222222",
            &decode_quantity_hex("0x1").unwrap(),
        );
        step.status = status;
        step.queued_job_id = job_id.map(str::to_string);
        let summary = summarize_plan_steps(&[step.clone()]);
        ConsolidationPlan {
            id: "plan-existing".into(),
            status: WalletPlanStatus::ReviewRequired,
            chain_id: 1,
            destination_address: None,
            origin: Some(AUTOMATION_ORIGIN.into()),
            created_at_unix: 1,
            updated_at_unix: 1,
            summary,
            policy_violations: Vec::new(),
            linkage_findings: Vec::new(),
            steps: vec![step],
        }
    }

    #[test]
    fn candidate_selection_overflow_refill_and_else_exclusion() {
        let reg = registry("seed-a");
        let mut state = state_with_policy(policy(true, true));
        state.addresses = vec![
            address(
                "seed-a",
                1,
                "0x1111111111111111111111111111111111111111",
                "0x1e",
                100,
            ),
            address(
                "seed-a",
                2,
                "0x1111111111111111111111111111111111111111",
                "0x2",
                100,
            ),
            address(
                "seed-a",
                2,
                "0x2222222222222222222222222222222222222222",
                "0x100",
                100,
            ),
        ];

        let selected = select_automation_candidates(&state, &reg, &QueueState::default());
        assert_eq!(selected.candidates.len(), 2);
        assert_eq!(selected.candidates[0].step.chain_id, 1);
        assert_eq!(selected.candidates[0].step.amount_hex, "0x14");
        assert_eq!(
            selected.candidates[0].step.destination_address.as_deref(),
            Some("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(selected.candidates[1].step.chain_id, 2);
        assert_eq!(selected.candidates[1].step.amount_hex, "0x8");
        assert_eq!(
            selected.candidates[1].step.destination_address.as_deref(),
            Some("0x1111111111111111111111111111111111111111")
        );

        let mut hostile = policy(true, true);
        hostile.hot_floor_wei_hex = "0x64".into();
        hostile.hot_target_wei_hex = "0x5".into();
        hostile.hot_overflow_wei_hex = Some("0x1".into());
        let mut hostile_state = state_with_policy(hostile);
        hostile_state.addresses = vec![address(
            "seed-a",
            1,
            "0x1111111111111111111111111111111111111111",
            "0xa",
            100,
        )];
        let selected = select_automation_candidates(&hostile_state, &reg, &QueueState::default());
        assert_eq!(selected.candidates.len(), 1);
        assert_eq!(
            selected.candidates[0].step.destination_address.as_deref(),
            Some("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(selected.candidates[0].step.amount_hex, "0x5");
    }

    #[test]
    fn candidate_blocking_existing_automation_rules() {
        let mut state = WalletInventoryState {
            consolidation_plans: vec![existing_plan(WalletPlanStepStatus::ReviewRequired, None)],
            ..Default::default()
        };
        assert_eq!(
            candidate_is_blocked_by_existing_automation(
                &state,
                &QueueState::default(),
                "seed-a",
                1,
                100
            ),
            Some("automation_step_open")
        );

        state.consolidation_plans =
            vec![existing_plan(WalletPlanStepStatus::Approved, Some("job"))];
        let queue = QueueState {
            jobs: vec![queue_job("job", "queued", 100)],
        };
        assert_eq!(
            candidate_is_blocked_by_existing_automation(&state, &queue, "seed-a", 1, 100),
            Some("automation_step_open")
        );

        let queue = QueueState {
            jobs: vec![queue_job("job", "confirmed", 100)],
        };
        assert_eq!(
            candidate_is_blocked_by_existing_automation(&state, &queue, "seed-a", 1, 100),
            Some("balance_not_reobserved")
        );

        let queue = QueueState {
            jobs: vec![queue_job("job", "confirmed", 90)],
        };
        assert_eq!(
            candidate_is_blocked_by_existing_automation(&state, &queue, "seed-a", 1, 100),
            None
        );
    }

    #[test]
    fn candidate_selection_policy_off_flags_produce_no_candidates() {
        let reg = registry("seed-a");
        for policy in [policy(false, true), policy(true, false)] {
            let mut state = state_with_policy(policy);
            state.addresses = vec![address(
                "seed-a",
                1,
                "0x1111111111111111111111111111111111111111",
                "0x1e",
                100,
            )];
            let selected = select_automation_candidates(&state, &reg, &QueueState::default());
            assert!(selected.candidates.is_empty());
            assert_eq!(selected.skipped_steps, 0);
        }
    }

    #[test]
    fn failure_classification_mapping() {
        let mut failures = MaintenanceFailureBreakdown::default();
        classify_reason(
            "execution_gate: allow_sweep_execution is disabled",
            &mut failures,
        );
        classify_reason("simulation_not_passed", &mut failures);
        classify_reason("rpc provider unavailable", &mut failures);
        classify_reason("unexpected", &mut failures);
        assert_eq!(failures.policy_block, 1);
        assert_eq!(failures.validation, 1);
        assert_eq!(failures.provider_error, 1);
        assert_eq!(failures.unknown, 1);
    }

    #[test]
    fn failure_merge_adds_fieldwise() {
        let mut left = MaintenanceFailureBreakdown {
            provider_error: 1,
            policy_block: 2,
            insufficient_gas: 3,
            validation: 4,
            unknown: 5,
            on_chain_revert: 6,
            broadcast_rejected: 7,
            receipt_timeout: 8,
        };
        let right = MaintenanceFailureBreakdown {
            provider_error: 10,
            policy_block: 20,
            insufficient_gas: 30,
            validation: 40,
            unknown: 50,
            on_chain_revert: 60,
            broadcast_rejected: 70,
            receipt_timeout: 80,
        };
        merge_failure_breakdowns(&mut left, &right);
        assert_eq!(
            left,
            MaintenanceFailureBreakdown {
                provider_error: 11,
                policy_block: 22,
                insufficient_gas: 33,
                validation: 44,
                unknown: 55,
                on_chain_revert: 66,
                broadcast_rejected: 77,
                receipt_timeout: 88,
            }
        );
    }
}
