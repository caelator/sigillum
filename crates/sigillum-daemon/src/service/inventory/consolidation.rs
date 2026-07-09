use std::collections::BTreeMap;

use sigillum_api::{
    ConsolidationPlan, ConsolidationPlanApproveRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanListResponse, ConsolidationPlanMutationResponse, WalletPlanStatus,
    WalletPlanStepStatus,
};

use crate::audit_log::AuditEventSpec;
use crate::inventory::WalletInventoryState;

use super::super::helpers::{now_unix, random_id};
use super::super::{ServiceError, ServiceResult, SigillumService};
use super::planner::{
    analyze_plan_linkage, apply_linkage_blockers, apply_policy_blockers_to_step,
    assign_step_ordering, build_plan_steps, plan_policy_violations, summarize_plan_steps,
};
use super::support::{load_inventory_state, save_inventory_state, trimmed_optional};

impl SigillumService {
    pub(crate) fn list_consolidation_plans(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<ConsolidationPlanListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(ConsolidationPlanListResponse {
            plans: state.consolidation_plans,
        })
    }

    pub(crate) async fn generate_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanGenerateRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
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
            let linkage_findings = analyze_plan_linkage(&state, &mut steps);
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
            let plan = ConsolidationPlan {
                id: random_id(),
                status,
                chain_id,
                destination_address: destination_address.clone(),
                created_at_unix: now,
                updated_at_unix: now,
                summary,
                policy_violations,
                linkage_findings,
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
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let policy = state.treasury_policy.clone();
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
            let _ = analyze_plan_linkage(linkage_state, &mut plan.steps);
            apply_linkage_blockers(&mut plan.steps);
        }
        let approve_all = body.step_ids.is_empty();
        for step in &mut plan.steps {
            if step.status == WalletPlanStepStatus::ReviewRequired
                && (approve_all || body.step_ids.iter().any(|id| id == &step.id))
            {
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
            }
        }
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = if plan.summary.blocked_steps > 0 || !plan.policy_violations.is_empty() {
            WalletPlanStatus::Blocked
        } else if plan.summary.review_required_steps > 0 {
            WalletPlanStatus::ReviewRequired
        } else {
            WalletPlanStatus::Approved
        };
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
