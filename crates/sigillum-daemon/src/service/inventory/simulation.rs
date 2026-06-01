use sigillum_api::{
    ConsolidationPlanMutationResponse, ConsolidationPlanSimulateRequest, ConsolidationPlanStep,
    ConsolidationPlanSummary, EvmProviderProfile,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::service::evm::{EvmContractCallPreflight, encode_quantity_u256};
use crate::service::helpers::{
    compare_u256, map_wallet_error, multiply_u256_u64, now_unix, subtract_u256,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::planner::summarize_plan_steps;
use super::preflight::{PlanStepPreflight, PlanStepPreflightCall, prepare_plan_step_preflight};
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
        let mut call = match prepare_plan_step_preflight(step) {
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
        let mut evidence = std::mem::take(&mut call.evidence);
        evidence.push(format!("provider_profile={}", provider.name));
        evidence.push(format!("chain_id={}", provider.chain_id));
        evidence.push(format!("from={}", step.address));
        evidence.push(format!("to={}", call.target_address));
        if let Err(outcome) =
            apply_native_sweep_fee_policy(provider, step, &mut call, &mut evidence)
        {
            return outcome;
        }
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

fn apply_native_sweep_fee_policy(
    provider: &EvmProviderProfile,
    step: &ConsolidationPlanStep,
    call: &mut PlanStepPreflightCall,
    evidence: &mut Vec<String>,
) -> Result<(), PlanSimulationOutcome> {
    if step.action != "sweep_native" {
        return Ok(());
    }

    let Some(max_priority_fee_per_gas_hex) = provider.max_priority_fee_per_gas_hex.as_deref()
    else {
        evidence.push("fee_policy=missing".into());
        evidence.push("missing_fee_policy=max_priority_fee_per_gas_hex".into());
        return Err(blocked_simulation(evidence));
    };
    let Some(max_fee_per_gas_hex) = provider.max_fee_per_gas_hex.as_deref() else {
        evidence.push("fee_policy=missing".into());
        evidence.push("missing_fee_policy=max_fee_per_gas_hex".into());
        return Err(blocked_simulation(evidence));
    };

    let gas_limit = provider.native_gas_limit.unwrap_or(21_000);
    let balance = match decode_quantity_hex(&step.amount_hex).map_err(map_wallet_error) {
        Ok(balance) => balance,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_native_amount:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let max_fee = match decode_quantity_hex(max_fee_per_gas_hex).map_err(map_wallet_error) {
        Ok(max_fee) => max_fee,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_max_fee_per_gas:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);

    evidence.push("fee_policy=profile_max_fee".into());
    evidence.push(format!(
        "max_priority_fee_per_gas_hex={max_priority_fee_per_gas_hex}"
    ));
    evidence.push(format!("max_fee_per_gas_hex={max_fee_per_gas_hex}"));
    evidence.push(format!("native_gas_limit={gas_limit}"));
    evidence.push(format!(
        "estimated_gas_cost_wei_hex={}",
        encode_quantity_u256(&gas_cost)
    ));

    if compare_u256(&balance, &gas_cost).is_le() {
        evidence.push("fee_policy_blocker=insufficient_native_balance_after_gas".into());
        return Err(blocked_simulation(evidence));
    }

    let spendable = subtract_u256(&balance, &gas_cost);
    let spendable_hex = encode_quantity_u256(&spendable);
    evidence.push(format!("native_sweep_spendable_amount_hex={spendable_hex}"));
    call.value_hex = Some(spendable_hex);

    Ok(())
}

fn blocked_simulation(evidence: &[String]) -> PlanSimulationOutcome {
    PlanSimulationOutcome {
        status: "blocked".into(),
        blocker: Some("simulation_blocked"),
        evidence: evidence.to_vec(),
    }
}

fn failed_simulation(evidence: &[String]) -> PlanSimulationOutcome {
    PlanSimulationOutcome {
        status: "failed".into(),
        blocker: Some("simulation_failed"),
        evidence: evidence.to_vec(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provider() -> EvmProviderProfile {
        EvmProviderProfile {
            name: "mainnet".into(),
            rpc_url: "http://127.0.0.1:8545".into(),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x1".into()),
            max_fee_per_gas_hex: Some("0x2".into()),
            native_gas_limit: Some(21_000),
            erc20_gas_limit: Some(65_000),
        }
    }

    fn sample_step(amount_hex: &str) -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: "step_1".into(),
            action: "sweep_native".into(),
            status: "review_required".into(),
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
            amount_hex: amount_hex.into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "required".into(),
            simulation_evidence: Vec::new(),
            risk_level: "low".into(),
            blockers: Vec::new(),
            auto_eligible: false,
            approved: false,
        }
    }

    fn sample_call() -> PlanStepPreflightCall {
        PlanStepPreflightCall {
            label: "native.transfer(value)",
            target_address: "0x9999999999999999999999999999999999999999".into(),
            data_hex: "0x".into(),
            value_hex: Some("0x10000".into()),
            evidence: Vec::new(),
        }
    }

    #[test]
    fn native_sweep_fee_policy_reserves_gas_from_transfer_value() {
        let provider = sample_provider();
        let step = sample_step("0x10000");
        let mut call = sample_call();
        let mut evidence = Vec::new();

        apply_native_sweep_fee_policy(&provider, &step, &mut call, &mut evidence).unwrap();

        assert_eq!(call.value_hex, Some("0x5bf0".into()));
        assert!(
            evidence
                .iter()
                .any(|item| item == "estimated_gas_cost_wei_hex=0xa410")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "native_sweep_spendable_amount_hex=0x5bf0")
        );
    }

    #[test]
    fn native_sweep_fee_policy_blocks_when_balance_cannot_pay_gas() {
        let provider = sample_provider();
        let step = sample_step("0xa410");
        let mut call = sample_call();
        let mut evidence = Vec::new();

        let outcome =
            apply_native_sweep_fee_policy(&provider, &step, &mut call, &mut evidence).unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert_eq!(outcome.blocker, Some("simulation_blocked"));
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "fee_policy_blocker=insufficient_native_balance_after_gas")
        );
    }

    #[test]
    fn native_sweep_fee_policy_blocks_when_fee_profile_is_missing() {
        let mut provider = sample_provider();
        provider.max_priority_fee_per_gas_hex = None;
        let step = sample_step("0x10000");
        let mut call = sample_call();
        let mut evidence = Vec::new();

        let outcome =
            apply_native_sweep_fee_policy(&provider, &step, &mut call, &mut evidence).unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "missing_fee_policy=max_priority_fee_per_gas_hex")
        );
    }
}
