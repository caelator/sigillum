use sigillum_api::{
    ConsolidationPlanMutationResponse, ConsolidationPlanSimulateRequest, ConsolidationPlanStep,
    ConsolidationPlanSummary, EvmProviderProfile,
};

use crate::audit_log::AuditEventSpec;
use crate::service::evm::EvmContractCallPreflight;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::planner::summarize_plan_steps;
use super::preflight::{PlanStepPreflight, prepare_plan_step_preflight};
use super::support::{load_inventory_state, save_inventory_state};

impl SigillumService {
    pub(crate) async fn simulate_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanSimulateRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let plan_index = state
            .consolidation_plans
            .iter()
            .position(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        let simulate_all = body.step_ids.is_empty();
        let step_indexes = state.consolidation_plans[plan_index]
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| simulate_all || body.step_ids.iter().any(|id| id == &step.id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if step_indexes.is_empty() {
            return Err(ServiceError::bad_request(
                "No matching consolidation plan steps found.",
            ));
        }

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut unsupported = 0usize;
        for step_index in step_indexes {
            let step = state.consolidation_plans[plan_index].steps[step_index].clone();
            let outcome = if let Some(blockers) = non_simulation_blockers(&step) {
                PlanSimulationOutcome {
                    status: "blocked".into(),
                    blocker: None,
                    evidence: vec![format!("blocked_by={}", blockers.join(","))],
                }
            } else {
                self.simulate_consolidation_step_preflight(&registry.evm_providers, &step)
                    .await
            };
            match outcome.status.as_str() {
                "passed" => passed += 1,
                "unsupported" => unsupported += 1,
                "failed" => failed += 1,
                _ => {}
            }
            apply_simulation_outcome(
                &mut state.consolidation_plans[plan_index].steps[step_index],
                outcome,
            );
        }

        let plan = &mut state.consolidation_plans[plan_index];
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = plan_status_for_summary(&plan.summary).into();
        let plan = plan.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanSimulate {
                id: plan.id.clone(),
                passed,
                failed,
                unsupported,
            },
        )?;

        Ok(ConsolidationPlanMutationResponse {
            status: "simulated".into(),
            plan,
        })
    }

    async fn simulate_consolidation_step_preflight(
        &self,
        providers: &[EvmProviderProfile],
        step: &ConsolidationPlanStep,
    ) -> PlanSimulationOutcome {
        let provider = match providers.iter().find(|provider| {
            provider.name == step.provider_profile && provider.chain_id == step.chain_id
        }) {
            Some(provider) => provider,
            None => {
                return PlanSimulationOutcome {
                    status: "failed".into(),
                    blocker: Some("simulation_failed"),
                    evidence: vec![format!(
                        "provider_not_found={} chain_id={}",
                        step.provider_profile, step.chain_id
                    )],
                };
            }
        };
        let call = match prepare_plan_step_preflight(step) {
            Ok(PlanStepPreflight::Call(call)) => call,
            Ok(PlanStepPreflight::Unsupported { evidence }) => {
                return PlanSimulationOutcome {
                    status: "unsupported".into(),
                    blocker: Some("simulation_unsupported"),
                    evidence,
                };
            }
            Err(error) => {
                return PlanSimulationOutcome {
                    status: "failed".into(),
                    blocker: Some("simulation_failed"),
                    evidence: vec![format!("preflight_prepare_error={error}")],
                };
            }
        };
        let mut evidence = call.evidence;
        evidence.push(format!("provider_profile={}", provider.name));
        evidence.push(format!("chain_id={}", provider.chain_id));
        evidence.push(format!("from={}", step.address));
        evidence.push(format!("to={}", call.target_address));
        if let Some(value_hex) = call.value_hex.as_deref() {
            evidence.push(format!("value={value_hex}"));
        }
        evidence.push(format!("call={}", call.label));
        evidence.push("rpc_method=eth_call".into());
        match self
            .evm_contract_call_preflight_for_provider(
                provider.compartment_id,
                provider,
                EvmContractCallPreflight {
                    from_address: &step.address,
                    target_address: &call.target_address,
                    data_hex: &call.data_hex,
                    value_hex: call.value_hex.as_deref(),
                    block_tag: "latest",
                },
            )
            .await
        {
            Ok(result) => {
                evidence.push(format!("eth_call_result={result}"));
                PlanSimulationOutcome {
                    status: "passed".into(),
                    blocker: None,
                    evidence,
                }
            }
            Err(error) => {
                evidence.push(format!("eth_call_error={error}"));
                PlanSimulationOutcome {
                    status: "failed".into(),
                    blocker: Some("simulation_failed"),
                    evidence,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct PlanSimulationOutcome {
    status: String,
    blocker: Option<&'static str>,
    evidence: Vec<String>,
}

fn non_simulation_blockers(step: &ConsolidationPlanStep) -> Option<Vec<String>> {
    let blockers = step
        .blockers
        .iter()
        .filter(|blocker| !is_simulation_blocker(blocker))
        .cloned()
        .collect::<Vec<_>>();
    if blockers.is_empty() {
        None
    } else {
        Some(blockers)
    }
}

fn apply_simulation_outcome(step: &mut ConsolidationPlanStep, outcome: PlanSimulationOutcome) {
    step.blockers
        .retain(|blocker| !is_simulation_blocker(blocker));
    if let Some(blocker) = outcome.blocker {
        push_unique_blocker(&mut step.blockers, blocker);
    }
    step.simulation_status = outcome.status;
    step.simulation_evidence = outcome.evidence;
    if step.blockers.is_empty() {
        step.status = if step.approved {
            "approved".into()
        } else {
            "review_required".into()
        };
    } else {
        step.status = "blocked".into();
    }
}

fn push_unique_blocker(blockers: &mut Vec<String>, blocker: &str) {
    if !blockers.iter().any(|existing| existing == blocker) {
        blockers.push(blocker.into());
    }
}

fn is_simulation_blocker(blocker: &str) -> bool {
    matches!(
        blocker,
        "simulation_failed" | "simulation_unsupported" | "simulation_blocked"
    )
}

fn plan_status_for_summary(summary: &ConsolidationPlanSummary) -> &'static str {
    if summary.total_steps == 0 {
        "empty"
    } else if summary.blocked_steps > 0 {
        "blocked"
    } else if summary.review_required_steps > 0 {
        "review_required"
    } else {
        "approved"
    }
}
