use std::collections::BTreeMap;

use sigillum_api::{
    ConsolidationPlan, ConsolidationPlanApproveRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanListResponse, ConsolidationPlanMutationResponse, WalletPlanStepAction,
    WalletPlanStepStatus, WalletSimulationStatus,
};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;

use super::super::helpers::{now_unix, random_id};
use super::super::list_query::{
    self, CreatedUpdatedSort, PLAN_STATUSES, SortOrder, effective_order, paginate, validated_value,
};
use super::super::{ServiceError, ServiceResult, SessionOperationContext, SigillumService};
use super::claim_gate::{claim_execution_gate_satisfied, refresh_claim_execution_blocker};
use super::planner::{
    analyze_plan_linkage, apply_linkage_blockers, apply_policy_blockers_to_step,
    assign_step_ordering, build_plan_steps, plan_policy_violations, plan_status,
    summarize_plan_steps,
};
use super::simulation::{DEFAULT_SIMULATION_FRESHNESS_SECS, simulation_is_stale};
use super::support::{load_inventory_state, save_inventory_state, trimmed_optional};

impl SigillumService {
    pub(crate) fn list_consolidation_plans(
        &self,
        token: Option<&str>,
        query: list_query::ConsolidationPlanListQuery,
    ) -> ServiceResult<ConsolidationPlanListResponse> {
        let _ = self.require_session(token)?;
        let status = query
            .status
            .map(|value| validated_value("status", value, &PLAN_STATUSES))
            .transpose()?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let mut plans = state.consolidation_plans;
        if let Some(status) = status.as_deref() {
            plans.retain(|plan| plan.status.as_str() == status);
        }
        if let Some(sort) = query.sort {
            let order = effective_order(query.sort.as_ref(), query.order);
            let key = |plan: &ConsolidationPlan| match sort {
                CreatedUpdatedSort::Created => plan.created_at_unix,
                CreatedUpdatedSort::Updated => plan.updated_at_unix,
            };
            match order {
                SortOrder::Asc => plans.sort_by_key(&key),
                SortOrder::Desc => plans.sort_by_key(|plan| std::cmp::Reverse(key(plan))),
            }
        }
        let (plans, pagination) = paginate(plans, query.page);
        Ok(ConsolidationPlanListResponse { plans, pagination })
    }

    pub(crate) async fn generate_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanGenerateRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let now = now_unix();
        let destination_address = body.destination_address.clone().and_then(trimmed_optional);
        let policy = state.treasury_policy.clone();
        if body.chain_id == Some(0) {
            return Err(ServiceError::bad_request("chain_id must be greater than 0"));
        }
        let steps = build_plan_steps(&state, &registry, &body, &destination_address);
        let steps = self
            .expand_defi_exit_steps(&registry.evm_providers, &state.chain_profiles, steps)
            .await;
        let steps = self
            .expand_gas_topup_steps(&registry.evm_providers, &registry, &state, steps)
            .await;
        let mut steps_by_chain = BTreeMap::<u64, Vec<_>>::new();
        for step in steps {
            steps_by_chain.entry(step.chain_id).or_default().push(step);
        }
        if steps_by_chain.is_empty() {
            steps_by_chain.insert(body.chain_id.unwrap_or(1), Vec::new());
        }

        let mut generated_plans = Vec::new();
        for (chain_id, mut steps) in steps_by_chain {
            assign_step_ordering(&mut steps);
            // Policy runs after planning so planner blockers and policy verdicts
            // are both visible on each step, then the summary reflects the final
            // step statuses.
            if let Some(policy) = policy.as_ref() {
                for step in &mut steps {
                    apply_policy_blockers_to_step(policy, step);
                }
            }
            let policy_violations = policy
                .as_ref()
                .map(|policy| plan_policy_violations(policy, &steps))
                .unwrap_or_default();
            let linkage_analysis = analyze_plan_linkage(&state, &mut steps);
            if policy
                .as_ref()
                .map(|policy| policy.block_cross_party_linkage)
                .unwrap_or(false)
            {
                apply_linkage_blockers(&mut steps);
            }
            let summary = summarize_plan_steps(&steps);
            let status = plan_status(&summary, &policy_violations);
            let plan = ConsolidationPlan {
                id: random_id(),
                status,
                chain_id,
                destination_address: destination_address.clone(),
                origin: None,
                created_at_unix: now,
                updated_at_unix: now,
                summary,
                policy_violations,
                linkage_findings: linkage_analysis.findings,
                risk_findings: linkage_analysis.risk_findings,
                steps,
            };
            generated_plans.push(plan);
        }

        state
            .consolidation_plans
            .extend(generated_plans.iter().cloned());
        save_inventory_state(&self.state.base_dir, &state)?;

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

        let plan = generated_plans
            .first()
            .cloned()
            .ok_or_else(|| ServiceError::internal("No consolidation plan was generated."))?;
        Ok(ConsolidationPlanMutationResponse {
            status: "generated".into(),
            plan,
            plans: generated_plans,
        })
    }

    pub(crate) async fn approve_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanApproveRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        self.approve_consolidation_plan_with_context(&session_context, body)
            .await
    }

    pub(in crate::service) async fn approve_consolidation_plan_with_context(
        &self,
        session_context: &SessionOperationContext,
        body: ConsolidationPlanApproveRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let _guard = self.acquire_session_operation(session_context).await?;
        let token = session_context.token.as_str();
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let policy = state.treasury_policy.clone();
        let risk_catalog = state.risk_catalog.clone();
        let linkage_state = if policy
            .as_ref()
            .map(|policy| policy.block_cross_party_linkage)
            .unwrap_or(false)
        {
            Some(WalletInventoryState {
                receive_allocations: state.receive_allocations.clone(),
                parties: state.parties.clone(),
                ..Default::default()
            })
        } else {
            None
        };
        let plan = state
            .consolidation_plans
            .iter_mut()
            .find(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        if let Some(linkage_state) = linkage_state.as_ref() {
            let analysis = analyze_plan_linkage(linkage_state, &mut plan.steps);
            plan.risk_findings = analysis.risk_findings;
            apply_linkage_blockers(&mut plan.steps);
        }
        let freshness_secs = policy
            .as_ref()
            .map(|policy| policy.simulation_freshness_secs)
            .unwrap_or(DEFAULT_SIMULATION_FRESHNESS_SECS);
        let now = now_unix();
        for step in &mut plan.steps {
            if step.simulation_status == WalletSimulationStatus::Passed
                && simulation_is_stale(&step.simulation_evidence, freshness_secs, now)
            {
                // Stale or unprovable simulation evidence must be re-run before execution (fail closed).
                step.simulation_status = WalletSimulationStatus::Required;
            }
        }
        let approve_all = body.step_ids.is_empty();
        for step in &mut plan.steps {
            let requested = approve_all || body.step_ids.iter().any(|id| id == &step.id);
            if step.status == WalletPlanStepStatus::ReviewRequired && requested {
                // Approval is the last review gate, so candidates are
                // re-checked against the CURRENT policy: a step planned
                // before a policy change must not slip through approval.
                if let Some(policy) = policy.as_ref() {
                    apply_policy_blockers_to_step(policy, step);
                    if step.status == WalletPlanStepStatus::Blocked {
                        continue;
                    }
                }
                step.approved = true;
                step.status = WalletPlanStepStatus::Approved;
            } else if step.status == WalletPlanStepStatus::Blocked
                && requested
                && step.action == WalletPlanStepAction::ClaimReward
                && !step.blockers.is_empty()
                && step
                    .blockers
                    .iter()
                    .all(|blocker| blocker == "claim_execution_disabled")
            {
                // W7.3: reverted claims become operator_action_required; see claim_gate.rs.
                let mut approval_candidate = step.clone();
                approval_candidate.approved = true;
                if claim_execution_gate_satisfied(
                    policy.as_ref(),
                    &risk_catalog,
                    &approval_candidate,
                ) {
                    step.approved = true;
                    refresh_claim_execution_blocker(policy.as_ref(), &risk_catalog, step);
                }
            }
        }
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = plan_status(&plan.summary, &plan.policy_violations);
        let plan = plan.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanApprove {
                id: plan.id.clone(),
                approved: plan.summary.approved_steps,
            },
        )?;

        Ok(ConsolidationPlanMutationResponse {
            status: "approved".into(),
            plan,
            plans: Vec::new(),
        })
    }
}
