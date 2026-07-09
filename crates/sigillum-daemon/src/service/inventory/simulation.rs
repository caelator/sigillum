use sigillum_api::{
    ConsolidationPlanMutationResponse, ConsolidationPlanSimulateRequest, ConsolidationPlanStep,
    ConsolidationPlanSummary, EvmProviderProfile, WalletInventoryAddress, WalletPlanStatus,
    WalletPlanStepAction, WalletPlanStepStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::service::evm::{EvmContractCallPreflight, encode_quantity_u256};
use crate::service::helpers::{
    compare_u256, map_wallet_error, multiply_u256_u64, now_unix, subtract_u256,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::claim_gate::refresh_claim_execution_blocker;
use super::defi_adapters::{
    DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW, DEFI_EXIT_ADAPTER_ERC4626_REDEEM,
    DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP, DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY,
};
use super::planner::summarize_plan_steps;
use super::preflight::{PlanStepPreflight, PlanStepPreflightCall, prepare_plan_step_preflight};
use super::support::{load_inventory_state, save_inventory_state};
use super::treasury::add_u256;

const DEFAULT_TOKEN_TRANSACTION_GAS_LIMIT: u64 = 65_000;
const DEFAULT_NFT_SWEEP_GAS_LIMIT: u64 = 100_000;
const DEFAULT_CLAIM_TRANSACTION_GAS_LIMIT: u64 = 120_000;

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
        let inventory_addresses = state.addresses.clone();
        let plan_steps = state.consolidation_plans[plan_index].steps.clone();
        let policy = state.treasury_policy.clone();
        let risk_catalog = state.risk_catalog.clone();
        for step_index in step_indexes {
            let step = state.consolidation_plans[plan_index].steps[step_index].clone();
            let mut outcome = if let Some(blockers) = non_simulation_blockers(&step) {
                PlanSimulationOutcome {
                    status: WalletSimulationStatus::Blocked,
                    blocker: None,
                    evidence: vec![format!("blocked_by={}", blockers.join(","))],
                }
            } else {
                self.simulate_consolidation_step_preflight(
                    &registry.evm_providers,
                    &inventory_addresses,
                    &step,
                    &plan_steps,
                )
                .await
            };
            outcome
                .evidence
                .push(format!("simulated_at_unix={}", now_unix()));
            match &outcome.status {
                WalletSimulationStatus::Passed => passed += 1,
                WalletSimulationStatus::Unsupported => unsupported += 1,
                WalletSimulationStatus::Failed => failed += 1,
                _ => {}
            }
            apply_simulation_outcome(
                &mut state.consolidation_plans[plan_index].steps[step_index],
                outcome,
            );
            refresh_claim_execution_blocker(
                policy.as_ref(),
                &risk_catalog,
                &mut state.consolidation_plans[plan_index].steps[step_index],
            );
        }

        let plan = &mut state.consolidation_plans[plan_index];
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = plan_status_for_summary(&plan.summary);
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
            plans: Vec::new(),
        })
    }

    async fn simulate_consolidation_step_preflight(
        &self,
        providers: &[EvmProviderProfile],
        inventory_addresses: &[WalletInventoryAddress],
        step: &ConsolidationPlanStep,
        plan_steps: &[ConsolidationPlanStep],
    ) -> PlanSimulationOutcome {
        let provider = match providers.iter().find(|provider| {
            provider.name == step.provider_profile && provider.chain_id == step.chain_id
        }) {
            Some(provider) => provider,
            None => {
                return PlanSimulationOutcome {
                    status: WalletSimulationStatus::Failed,
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
                    status: WalletSimulationStatus::Unsupported,
                    blocker: Some("simulation_unsupported"),
                    evidence,
                };
            }
            Err(error) => {
                return PlanSimulationOutcome {
                    status: WalletSimulationStatus::Failed,
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
        let fee_basis = if matches!(
            step.action,
            WalletPlanStepAction::SweepNative | WalletPlanStepAction::FundGas
        ) {
            let gas_limit = provider.native_gas_limit.unwrap_or(21_000);
            Some(
                match self
                    .resolve_fee_basis_for_provider_profile(provider, gas_limit)
                    .await
                {
                    Ok(fee_basis) => fee_basis,
                    Err(error) => {
                        evidence.push(format!("fee_estimation_error={error}"));
                        return failed_simulation(&evidence);
                    }
                },
            )
        } else if call.value_hex.is_none() {
            let gas_limit = zero_value_transaction_gas_limit(provider, step);
            Some(
                match self
                    .resolve_fee_basis_for_provider_profile(provider, gas_limit)
                    .await
                {
                    Ok(fee_basis) => fee_basis,
                    Err(error) => {
                        evidence.push(format!("fee_estimation_error={error}"));
                        return failed_simulation(&evidence);
                    }
                },
            )
        } else {
            None
        };
        let pending_gas_topup_credit = pending_gas_topup_credit_for_step(step, plan_steps);
        if let Err(outcome) = apply_native_sweep_fee_policy(
            provider,
            step,
            &mut call,
            &mut evidence,
            fee_basis.as_ref(),
            pending_gas_topup_credit,
        ) {
            return outcome;
        }
        if let Err(outcome) = apply_fund_gas_fee_policy(
            provider,
            inventory_addresses,
            step,
            &call,
            &mut evidence,
            fee_basis.as_ref(),
        ) {
            return outcome;
        }
        if let Err(outcome) = apply_zero_value_transaction_gas_policy(
            provider,
            inventory_addresses,
            step,
            &call,
            &mut evidence,
            fee_basis.as_ref(),
            pending_gas_topup_credit,
        ) {
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
                for expected_output in defi_expected_output_evidence(step, &result) {
                    evidence.push(expected_output);
                }
                PlanSimulationOutcome {
                    status: WalletSimulationStatus::Passed,
                    blocker: None,
                    evidence,
                }
            }
            Err(error) => {
                evidence.push(format!("eth_call_error={error}"));
                PlanSimulationOutcome {
                    status: WalletSimulationStatus::Failed,
                    blocker: Some("simulation_failed"),
                    evidence,
                }
            }
        }
    }

    pub(super) async fn resolve_fee_basis_for_provider_profile(
        &self,
        provider: &EvmProviderProfile,
        gas_limit: u64,
    ) -> ServiceResult<FeeBasisResolution> {
        if !provider.fee_estimation_enabled {
            return Ok(static_fee_basis_for_provider(provider));
        }

        let response = self
            .evm_estimate_fees_for_provider(provider.compartment_id, provider, gas_limit)
            .await?;
        Ok(FeeBasisResolution::Resolved(ResolvedFeeBasis {
            basis: "estimated",
            max_priority_fee_per_gas_hex: response.fees.max_priority_fee_per_gas_hex,
            max_fee_per_gas_hex: response.fees.max_fee_per_gas_hex,
            resolved_at_unix: now_unix(),
        }))
    }
}

#[derive(Clone, Debug)]
struct PlanSimulationOutcome {
    status: WalletSimulationStatus,
    blocker: Option<&'static str>,
    evidence: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedFeeBasis {
    pub(super) basis: &'static str,
    pub(super) max_priority_fee_per_gas_hex: String,
    pub(super) max_fee_per_gas_hex: String,
    pub(super) resolved_at_unix: u64,
}

#[derive(Clone, Debug)]
pub(super) enum FeeBasisResolution {
    Resolved(ResolvedFeeBasis),
    MissingStatic { missing_field: &'static str },
}

fn static_fee_basis_for_provider(provider: &EvmProviderProfile) -> FeeBasisResolution {
    let Some(max_priority_fee_per_gas_hex) = provider.max_priority_fee_per_gas_hex.clone() else {
        return FeeBasisResolution::MissingStatic {
            missing_field: "max_priority_fee_per_gas_hex",
        };
    };
    let Some(max_fee_per_gas_hex) = provider.max_fee_per_gas_hex.clone() else {
        return FeeBasisResolution::MissingStatic {
            missing_field: "max_fee_per_gas_hex",
        };
    };
    FeeBasisResolution::Resolved(ResolvedFeeBasis {
        basis: "static_profile",
        max_priority_fee_per_gas_hex,
        max_fee_per_gas_hex,
        resolved_at_unix: now_unix(),
    })
}

fn apply_zero_value_transaction_gas_policy(
    provider: &EvmProviderProfile,
    inventory_addresses: &[WalletInventoryAddress],
    step: &ConsolidationPlanStep,
    call: &PlanStepPreflightCall,
    evidence: &mut Vec<String>,
    fee_basis: Option<&FeeBasisResolution>,
    pending_gas_topup_credit: Option<[u8; 32]>,
) -> Result<(), PlanSimulationOutcome> {
    if step.action == WalletPlanStepAction::SweepNative || call.value_hex.is_some() {
        return Ok(());
    }

    let basis = match fee_basis {
        Some(FeeBasisResolution::Resolved(basis)) => basis,
        Some(FeeBasisResolution::MissingStatic { missing_field }) => {
            evidence.push("gas_policy=missing".into());
            evidence.push(format!("missing_gas_policy={missing_field}"));
            return Err(blocked_simulation(evidence));
        }
        None => {
            evidence.push("gas_policy=missing".into());
            evidence.push("missing_gas_policy=max_priority_fee_per_gas_hex".into());
            return Err(blocked_simulation(evidence));
        }
    };

    let gas_limit = zero_value_transaction_gas_limit(provider, step);
    let native_balance_hex = match inventory_native_balance_hex_for_step(inventory_addresses, step)
    {
        Some(balance) => balance,
        None => {
            evidence.push("gas_policy=inventory_native_balance".into());
            evidence.push("gas_policy_blocker=missing_inventory_address".into());
            return Err(blocked_simulation(evidence));
        }
    };
    let native_balance = match decode_quantity_hex(native_balance_hex).map_err(map_wallet_error) {
        Ok(balance) => balance,
        Err(error) => {
            evidence.push(format!("gas_policy_error=invalid_native_balance:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let max_fee = match decode_quantity_hex(&basis.max_fee_per_gas_hex).map_err(map_wallet_error) {
        Ok(max_fee) => max_fee,
        Err(error) => {
            evidence.push(format!("gas_policy_error=invalid_max_fee_per_gas:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    let available = pending_gas_topup_credit
        .map(|credit| add_u256(&native_balance, &credit))
        .unwrap_or(native_balance);

    evidence.push(format!("fee_basis={}", basis.basis));
    evidence.push(format!(
        "fee_basis_resolved_at_unix={}",
        basis.resolved_at_unix
    ));
    evidence.push("gas_policy=profile_max_fee".into());
    evidence.push(format!(
        "max_priority_fee_per_gas_hex={}",
        basis.max_priority_fee_per_gas_hex
    ));
    evidence.push(format!("max_fee_per_gas_hex={}", basis.max_fee_per_gas_hex));
    evidence.push(format!("transaction_gas_limit={gas_limit}"));
    evidence.push(format!("native_balance_wei_hex={native_balance_hex}"));
    if let Some(credit) = pending_gas_topup_credit {
        evidence.push(format!(
            "pending_gas_topup_wei_hex={}",
            encode_quantity_u256(&credit)
        ));
    }
    evidence.push(format!(
        "estimated_gas_cost_wei_hex={}",
        encode_quantity_u256(&gas_cost)
    ));

    if compare_u256(&available, &gas_cost).is_lt() {
        evidence.push("gas_policy_blocker=insufficient_native_gas".into());
        return Err(blocked_simulation(evidence));
    }

    Ok(())
}

pub(super) fn zero_value_transaction_gas_limit(
    provider: &EvmProviderProfile,
    step: &ConsolidationPlanStep,
) -> u64 {
    match &step.action {
        WalletPlanStepAction::SweepErc20
        | WalletPlanStepAction::ApproveErc20
        | WalletPlanStepAction::RevokeErc20Approval
        | WalletPlanStepAction::RevokePermit2Allowance => provider
            .erc20_gas_limit
            .unwrap_or(DEFAULT_TOKEN_TRANSACTION_GAS_LIMIT),
        WalletPlanStepAction::SweepNft => provider
            .erc20_gas_limit
            .unwrap_or(DEFAULT_NFT_SWEEP_GAS_LIMIT)
            .max(DEFAULT_NFT_SWEEP_GAS_LIMIT),
        WalletPlanStepAction::RevokeNftOperatorApproval => provider
            .erc20_gas_limit
            .unwrap_or(DEFAULT_TOKEN_TRANSACTION_GAS_LIMIT),
        WalletPlanStepAction::ClaimReward => provider
            .erc20_gas_limit
            .unwrap_or(DEFAULT_CLAIM_TRANSACTION_GAS_LIMIT)
            .max(DEFAULT_CLAIM_TRANSACTION_GAS_LIMIT),
        _ => provider
            .erc20_gas_limit
            .unwrap_or(DEFAULT_TOKEN_TRANSACTION_GAS_LIMIT),
    }
}

pub(super) fn inventory_native_balance_hex_for_step<'a>(
    inventory_addresses: &'a [WalletInventoryAddress],
    step: &ConsolidationPlanStep,
) -> Option<&'a str> {
    inventory_addresses
        .iter()
        .find(|address| {
            address.wallet_family == step.wallet_family
                && address.wallet_profile == step.wallet_profile
                && address.provider_profile == step.provider_profile
                && address.chain_id == step.chain_id
                && address.address.eq_ignore_ascii_case(&step.address)
        })
        .map(|address| address.native_balance_wei_hex.as_str())
}

fn apply_native_sweep_fee_policy(
    provider: &EvmProviderProfile,
    step: &ConsolidationPlanStep,
    call: &mut PlanStepPreflightCall,
    evidence: &mut Vec<String>,
    fee_basis: Option<&FeeBasisResolution>,
    pending_gas_topup_credit: Option<[u8; 32]>,
) -> Result<(), PlanSimulationOutcome> {
    if step.action != WalletPlanStepAction::SweepNative {
        return Ok(());
    }

    let basis = match fee_basis {
        Some(FeeBasisResolution::Resolved(basis)) => basis,
        Some(FeeBasisResolution::MissingStatic { missing_field }) => {
            evidence.push("fee_policy=missing".into());
            evidence.push(format!("missing_fee_policy={missing_field}"));
            return Err(blocked_simulation(evidence));
        }
        None => {
            evidence.push("fee_policy=missing".into());
            evidence.push("missing_fee_policy=max_priority_fee_per_gas_hex".into());
            return Err(blocked_simulation(evidence));
        }
    };

    let gas_limit = provider.native_gas_limit.unwrap_or(21_000);
    let balance = match decode_quantity_hex(&step.amount_hex).map_err(map_wallet_error) {
        Ok(balance) => balance,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_native_amount:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let max_fee = match decode_quantity_hex(&basis.max_fee_per_gas_hex).map_err(map_wallet_error) {
        Ok(max_fee) => max_fee,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_max_fee_per_gas:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    let effective_balance = pending_gas_topup_credit
        .map(|credit| add_u256(&balance, &credit))
        .unwrap_or(balance);

    evidence.push(format!("fee_basis={}", basis.basis));
    evidence.push(format!(
        "fee_basis_resolved_at_unix={}",
        basis.resolved_at_unix
    ));
    evidence.push("fee_policy=profile_max_fee".into());
    evidence.push(format!(
        "max_priority_fee_per_gas_hex={}",
        basis.max_priority_fee_per_gas_hex
    ));
    evidence.push(format!("max_fee_per_gas_hex={}", basis.max_fee_per_gas_hex));
    evidence.push(format!("native_gas_limit={gas_limit}"));
    evidence.push(format!(
        "estimated_gas_cost_wei_hex={}",
        encode_quantity_u256(&gas_cost)
    ));
    if let Some(credit) = pending_gas_topup_credit {
        evidence.push(format!(
            "pending_gas_topup_wei_hex={}",
            encode_quantity_u256(&credit)
        ));
    }

    if compare_u256(&effective_balance, &gas_cost).is_le() {
        evidence.push("fee_policy_blocker=insufficient_native_balance_after_gas".into());
        return Err(blocked_simulation(evidence));
    }

    let spendable = subtract_u256(&effective_balance, &gas_cost);
    let spendable_hex = encode_quantity_u256(&spendable);
    evidence.push(format!("native_sweep_spendable_amount_hex={spendable_hex}"));
    call.value_hex = Some(spendable_hex);

    Ok(())
}

fn apply_fund_gas_fee_policy(
    provider: &EvmProviderProfile,
    inventory_addresses: &[WalletInventoryAddress],
    step: &ConsolidationPlanStep,
    _call: &PlanStepPreflightCall,
    evidence: &mut Vec<String>,
    fee_basis: Option<&FeeBasisResolution>,
) -> Result<(), PlanSimulationOutcome> {
    if step.action != WalletPlanStepAction::FundGas {
        return Ok(());
    }

    let basis = match fee_basis {
        Some(FeeBasisResolution::Resolved(basis)) => basis,
        Some(FeeBasisResolution::MissingStatic { missing_field }) => {
            evidence.push("fee_policy=missing".into());
            evidence.push(format!("missing_fee_policy={missing_field}"));
            return Err(blocked_simulation(evidence));
        }
        None => {
            evidence.push("fee_policy=missing".into());
            evidence.push("missing_fee_policy=max_priority_fee_per_gas_hex".into());
            return Err(blocked_simulation(evidence));
        }
    };

    let amount = match decode_quantity_hex(&step.amount_hex).map_err(map_wallet_error) {
        Ok(amount) => amount,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_fund_gas_amount:{error}"));
            return Err(failed_simulation(evidence));
        }
    };
    let sponsor_balance_hex = match inventory_native_balance_hex_for_step(inventory_addresses, step)
    {
        Some(balance) => balance,
        None => {
            evidence.push("fee_policy=inventory_native_balance".into());
            evidence.push("fee_policy_blocker=missing_sponsor_inventory_address".into());
            return Err(blocked_simulation(evidence));
        }
    };
    let sponsor_balance = match decode_quantity_hex(sponsor_balance_hex).map_err(map_wallet_error) {
        Ok(balance) => balance,
        Err(error) => {
            evidence.push(format!(
                "fee_policy_error=invalid_sponsor_native_balance:{error}"
            ));
            return Err(failed_simulation(evidence));
        }
    };
    let max_fee = match decode_quantity_hex(&basis.max_fee_per_gas_hex).map_err(map_wallet_error) {
        Ok(max_fee) => max_fee,
        Err(error) => {
            evidence.push(format!("fee_policy_error=invalid_max_fee_per_gas:{error}"));
            return Err(failed_simulation(evidence));
        }
    };

    let gas_limit = provider.native_gas_limit.unwrap_or(21_000);
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    let required = add_u256(&amount, &gas_cost);

    evidence.push(format!("fee_basis={}", basis.basis));
    evidence.push(format!(
        "fee_basis_resolved_at_unix={}",
        basis.resolved_at_unix
    ));
    evidence.push("fee_policy=profile_max_fee".into());
    evidence.push(format!(
        "max_priority_fee_per_gas_hex={}",
        basis.max_priority_fee_per_gas_hex
    ));
    evidence.push(format!("max_fee_per_gas_hex={}", basis.max_fee_per_gas_hex));
    evidence.push(format!("native_gas_limit={gas_limit}"));
    evidence.push(format!(
        "estimated_gas_cost_wei_hex={}",
        encode_quantity_u256(&gas_cost)
    ));
    evidence.push(format!(
        "sponsor_native_balance_wei_hex={sponsor_balance_hex}"
    ));
    evidence.push(format!(
        "gas_topup_amount_wei_hex={}",
        encode_quantity_u256(&amount)
    ));

    if compare_u256(&sponsor_balance, &required).is_lt() {
        evidence.push("fee_policy_blocker=insufficient_sponsor_native_for_topup".into());
        return Err(blocked_simulation(evidence));
    }

    Ok(())
}

fn pending_gas_topup_credit_for_step(
    step: &ConsolidationPlanStep,
    plan_steps: &[ConsolidationPlanStep],
) -> Option<[u8; 32]> {
    let mut credit = None::<[u8; 32]>;
    for fund_step in plan_steps.iter().filter(|candidate| {
        candidate.action == WalletPlanStepAction::FundGas
            && candidate.blockers.is_empty()
            && step.depends_on.iter().any(|id| id == &candidate.id)
            && candidate
                .destination_address
                .as_deref()
                .is_some_and(|destination| destination.eq_ignore_ascii_case(&step.address))
    }) {
        let Ok(amount) = decode_quantity_hex(&fund_step.amount_hex) else {
            continue;
        };
        credit = Some(match credit {
            Some(existing) => add_u256(&existing, &amount),
            None => amount,
        });
    }
    credit
}

fn blocked_simulation(evidence: &[String]) -> PlanSimulationOutcome {
    PlanSimulationOutcome {
        status: WalletSimulationStatus::Blocked,
        blocker: Some("simulation_blocked"),
        evidence: evidence.to_vec(),
    }
}

fn failed_simulation(evidence: &[String]) -> PlanSimulationOutcome {
    PlanSimulationOutcome {
        status: WalletSimulationStatus::Failed,
        blocker: Some("simulation_failed"),
        evidence: evidence.to_vec(),
    }
}

fn non_simulation_blockers(step: &ConsolidationPlanStep) -> Option<Vec<String>> {
    let blockers = step
        .blockers
        .iter()
        .filter(|blocker| blocks_simulation(blocker))
        .cloned()
        .collect::<Vec<_>>();
    if blockers.is_empty() {
        None
    } else {
        Some(blockers)
    }
}

fn defi_expected_output_evidence(step: &ConsolidationPlanStep, result_hex: &str) -> Vec<String> {
    if step.action != WalletPlanStepAction::ExitDefiPosition {
        return Vec::new();
    }
    let Some(adapter) = step.claim_adapter.as_deref() else {
        return Vec::new();
    };
    match adapter {
        DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW
        | DEFI_EXIT_ADAPTER_ERC4626_REDEEM
        | DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP => {
            canonical_quantity_hex_from_single_word(result_hex)
                .map(|value| vec![format!("expected_assets_out_hex={value}")])
                .unwrap_or_default()
        }
        DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY => {
            let Some(words) = strict_words_hex(result_hex, 2) else {
                return Vec::new();
            };
            vec![
                format!(
                    "expected_amount0_out_hex={}",
                    canonical_quantity_hex_from_word(&words[0])
                ),
                format!(
                    "expected_amount1_out_hex={}",
                    canonical_quantity_hex_from_word(&words[1])
                ),
            ]
        }
        _ => Vec::new(),
    }
}

fn canonical_quantity_hex_from_single_word(value: &str) -> Option<String> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let raw = raw.trim_start_matches('0');
    if raw.is_empty() {
        Some("0x0".into())
    } else {
        Some(format!("0x{}", raw.to_ascii_lowercase()))
    }
}

fn strict_words_hex(value: &str, expected_words: usize) -> Option<Vec<String>> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != expected_words.checked_mul(64)?
        || !raw.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    raw.as_bytes()
        .chunks_exact(64)
        .map(|chunk| Some(std::str::from_utf8(chunk).ok()?.to_ascii_lowercase()))
        .collect::<Option<Vec<_>>>()
}

fn canonical_quantity_hex_from_word(word: &str) -> String {
    let raw = word.trim_start_matches('0');
    if raw.is_empty() {
        "0x0".into()
    } else {
        format!("0x{}", raw.to_ascii_lowercase())
    }
}

fn blocks_simulation(blocker: &str) -> bool {
    !is_simulation_blocker(blocker) && blocker != "claim_execution_disabled"
}

pub(super) fn parse_simulated_at_unix(evidence: &[String]) -> Option<u64> {
    evidence
        .iter()
        .rev()
        .find_map(|item| item.strip_prefix("simulated_at_unix="))
        .and_then(|value| value.parse().ok())
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
            WalletPlanStepStatus::Approved
        } else {
            WalletPlanStepStatus::ReviewRequired
        };
    } else {
        step.status = WalletPlanStepStatus::Blocked;
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

fn plan_status_for_summary(summary: &ConsolidationPlanSummary) -> WalletPlanStatus {
    if summary.total_steps == 0 {
        WalletPlanStatus::Empty
    } else if summary.blocked_steps > 0 {
        WalletPlanStatus::Blocked
    } else if summary.review_required_steps > 0 {
        WalletPlanStatus::ReviewRequired
    } else {
        WalletPlanStatus::Approved
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
            fee_estimation_enabled: false,
        }
    }

    fn static_fee_basis() -> FeeBasisResolution {
        FeeBasisResolution::Resolved(ResolvedFeeBasis {
            basis: "static_profile",
            max_priority_fee_per_gas_hex: "0x1".into(),
            max_fee_per_gas_hex: "0x2".into(),
            resolved_at_unix: 1,
        })
    }

    fn estimated_fee_basis() -> FeeBasisResolution {
        FeeBasisResolution::Resolved(ResolvedFeeBasis {
            basis: "estimated",
            max_priority_fee_per_gas_hex: "0x1".into(),
            max_fee_per_gas_hex: "0x3".into(),
            resolved_at_unix: 2,
        })
    }

    fn sample_step(amount_hex: &str) -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: "step_1".into(),
            sequence: 0,
            depends_on: Vec::new(),
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
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            exit_token0_address: None,
            exit_token1_address: None,
            exit_amount0_min_hex: None,
            exit_amount1_min_hex: None,
            exit_deadline_unix: None,
            amount_hex: amount_hex.into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "required".into(),
            simulation_evidence: Vec::new(),
            risk_level: "low".into(),
            blockers: Vec::new(),
            linkage_warnings: Vec::new(),
            auto_eligible: false,
            approved: false,
            queued_job_id: None,
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

    fn sample_fund_gas_step(amount_hex: &str) -> ConsolidationPlanStep {
        let mut step = sample_step(amount_hex);
        step.action = WalletPlanStepAction::FundGas;
        step.asset_kind = "native".into();
        step.asset_address = None;
        step.address = "0x4444444444444444444444444444444444444444".into();
        step.destination_address = Some("0x1111111111111111111111111111111111111111".into());
        step
    }

    fn sample_fund_gas_call(amount_hex: &str) -> PlanStepPreflightCall {
        PlanStepPreflightCall {
            label: "native.transfer(gas_topup)",
            target_address: "0x1111111111111111111111111111111111111111".into(),
            data_hex: "0x".into(),
            value_hex: Some(amount_hex.into()),
            evidence: Vec::new(),
        }
    }

    fn sample_erc20_step() -> ConsolidationPlanStep {
        let mut step = sample_step("0xf4240");
        step.action = "sweep_erc20".into();
        step.asset_kind = "erc20".into();
        step.asset_address = Some("0x2222222222222222222222222222222222222222".into());
        step
    }

    fn sample_erc20_call() -> PlanStepPreflightCall {
        PlanStepPreflightCall {
            label: "erc20.transfer(destination,amount)",
            target_address: "0x2222222222222222222222222222222222222222".into(),
            data_hex: "0xa9059cbb".into(),
            value_hex: None,
            evidence: Vec::new(),
        }
    }

    fn sample_nft_step() -> ConsolidationPlanStep {
        let mut step = sample_step("0x1");
        step.action = "sweep_nft".into();
        step.asset_kind = "erc721".into();
        step.asset_address = Some("0x2222222222222222222222222222222222222222".into());
        step.token_id_hex = Some("0x7b".into());
        step
    }

    fn sample_nft_call() -> PlanStepPreflightCall {
        PlanStepPreflightCall {
            label: "erc721.safeTransferFrom(owner,destination,tokenId)",
            target_address: "0x2222222222222222222222222222222222222222".into(),
            data_hex: "0x42842e0e".into(),
            value_hex: None,
            evidence: Vec::new(),
        }
    }

    fn sample_inventory_address_at(
        address: &str,
        native_balance_wei_hex: &str,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: "addr_1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: native_balance_wei_hex.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: vec!["signer_available".into()],
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_inventory_address(native_balance_wei_hex: &str) -> WalletInventoryAddress {
        sample_inventory_address_at(
            "0x1111111111111111111111111111111111111111",
            native_balance_wei_hex,
        )
    }

    fn sample_sponsor_inventory_address(native_balance_wei_hex: &str) -> WalletInventoryAddress {
        sample_inventory_address_at(
            "0x4444444444444444444444444444444444444444",
            native_balance_wei_hex,
        )
    }

    #[test]
    fn native_sweep_fee_policy_reserves_gas_from_transfer_value() {
        let provider = sample_provider();
        let step = sample_step("0x10000");
        let mut call = sample_call();
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap();

        assert_eq!(call.value_hex, Some("0x5bf0".into()));
        assert!(
            evidence
                .iter()
                .any(|item| item == "fee_basis=static_profile")
        );
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
        let fee_basis = static_fee_basis();

        let outcome = apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

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
        let provider = sample_provider();
        let step = sample_step("0x10000");
        let mut call = sample_call();
        let mut evidence = Vec::new();
        let fee_basis = FeeBasisResolution::MissingStatic {
            missing_field: "max_priority_fee_per_gas_hex",
        };

        let outcome = apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "missing_fee_policy=max_priority_fee_per_gas_hex")
        );
    }

    #[test]
    fn estimated_fee_basis_values_drive_native_sweep_math_and_evidence() {
        let provider = sample_provider();
        let step = sample_step("0x10000");
        let mut call = sample_call();
        let mut evidence = Vec::new();
        let fee_basis = estimated_fee_basis();

        apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap();

        assert_eq!(call.value_hex, Some("0x9e8".into()));
        assert!(evidence.iter().any(|item| item == "fee_basis=estimated"));
        assert!(
            evidence
                .iter()
                .any(|item| item == "fee_basis_resolved_at_unix=2")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "max_fee_per_gas_hex=0x3")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "estimated_gas_cost_wei_hex=0xf618")
        );
    }

    #[test]
    fn static_fee_basis_resolution_preserves_existing_blockers_and_evidence() {
        let provider = sample_provider();
        let step = sample_step("0xa410");
        let mut call = sample_call();
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis_for_provider(&provider);

        let outcome = apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert_eq!(outcome.blocker, Some("simulation_blocked"));
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "fee_basis=static_profile")
        );
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "fee_policy=profile_max_fee")
        );
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "max_fee_per_gas_hex=0x2")
        );
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "fee_policy_blocker=insufficient_native_balance_after_gas")
        );
    }

    #[test]
    fn fund_gas_fee_policy_records_fee_basis_and_passes_with_funded_sponsor() {
        let provider = sample_provider();
        let step = sample_fund_gas_step("0x2f9b8");
        let call = sample_fund_gas_call("0x2f9b8");
        let addresses = vec![sample_sponsor_inventory_address("0x39dc8")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        apply_fund_gas_fee_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
        )
        .unwrap();

        assert_eq!(call.value_hex, Some("0x2f9b8".into()));
        assert!(
            evidence
                .iter()
                .any(|item| item == "fee_basis=static_profile")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "gas_topup_amount_wei_hex=0x2f9b8")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "sponsor_native_balance_wei_hex=0x39dc8")
        );
        assert!(evidence.iter().any(|item| item == "native_gas_limit=21000"));
    }

    #[test]
    fn fund_gas_fee_policy_blocks_insufficient_sponsor() {
        let provider = sample_provider();
        let step = sample_fund_gas_step("0x2f9b8");
        let call = sample_fund_gas_call("0x2f9b8");
        let addresses = vec![sample_sponsor_inventory_address("0x39dc7")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        let outcome = apply_fund_gas_fee_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
        )
        .unwrap_err();

        assert_eq!(outcome.status, WalletSimulationStatus::Blocked);
        assert_eq!(
            outcome.evidence.last().map(String::as_str),
            Some("fee_policy_blocker=insufficient_sponsor_native_for_topup")
        );
    }

    #[test]
    fn fund_gas_fee_policy_blocks_missing_sponsor_inventory() {
        let provider = sample_provider();
        let step = sample_fund_gas_step("0x2f9b8");
        let call = sample_fund_gas_call("0x2f9b8");
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        let outcome = apply_fund_gas_fee_policy(
            &provider,
            &[],
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
        )
        .unwrap_err();

        assert_eq!(outcome.status, WalletSimulationStatus::Blocked);
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "fee_policy_blocker=missing_sponsor_inventory_address")
        );
    }

    #[test]
    fn pending_gas_topup_credit_for_step_only_credits_matching_unblocked_deps() {
        let funded_address = "0xaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaA";
        let funded_address_lower = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut dependent = sample_erc20_step();
        dependent.depends_on = vec!["fund_1".into()];
        dependent.address = funded_address.into();

        let mut matching_fund_step = sample_fund_gas_step("0x2f9b8");
        matching_fund_step.id = "fund_1".into();
        matching_fund_step.destination_address = Some(funded_address_lower.into());
        let credit = pending_gas_topup_credit_for_step(
            &dependent,
            std::slice::from_ref(&matching_fund_step),
        )
        .expect("matching unblocked fund_gas dependency should credit the step");
        assert_eq!(encode_quantity_u256(&credit), "0x2f9b8");

        let mut blocked_fund_step = matching_fund_step.clone();
        blocked_fund_step.blockers = vec!["cross_party_linkage".into()];
        assert!(pending_gas_topup_credit_for_step(&dependent, &[blocked_fund_step]).is_none());

        let mut unreferenced_fund_step = matching_fund_step.clone();
        unreferenced_fund_step.id = "fund_2".into();
        assert!(pending_gas_topup_credit_for_step(&dependent, &[unreferenced_fund_step]).is_none());

        let mut different_destination_fund_step = matching_fund_step;
        different_destination_fund_step.destination_address =
            Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
        assert!(
            pending_gas_topup_credit_for_step(&dependent, &[different_destination_fund_step])
                .is_none()
        );
    }

    #[test]
    fn zero_value_gas_policy_accepts_inventory_balance_that_covers_gas() {
        let provider = sample_provider();
        let step = sample_erc20_step();
        let call = sample_erc20_call();
        let addresses = vec![sample_inventory_address("0x20000")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap();

        assert!(
            evidence
                .iter()
                .any(|item| item == "gas_policy=profile_max_fee")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "fee_basis=static_profile")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "estimated_gas_cost_wei_hex=0x1fbd0")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "native_balance_wei_hex=0x20000")
        );
    }

    #[test]
    fn zero_value_gas_policy_blocks_when_inventory_balance_cannot_pay_gas() {
        let provider = sample_provider();
        let step = sample_erc20_step();
        let call = sample_erc20_call();
        let addresses = vec![sample_inventory_address("0x1")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        let outcome = apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert_eq!(outcome.blocker, Some("simulation_blocked"));
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "gas_policy_blocker=insufficient_native_gas")
        );
    }

    #[test]
    fn zero_value_gas_policy_credits_pending_fund_gas_topup() {
        let provider = sample_provider();
        let step = sample_erc20_step();
        let call = sample_erc20_call();
        let addresses = vec![sample_inventory_address("0x1")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();
        let credit = decode_quantity_hex("0x1fbd0").unwrap();

        apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            Some(credit),
        )
        .unwrap();

        assert!(
            evidence
                .iter()
                .any(|item| item == "pending_gas_topup_wei_hex=0x1fbd0")
        );
    }

    #[test]
    fn zero_value_gas_policy_without_credit_is_byte_identical_regression() {
        let provider = sample_provider();
        let step = sample_erc20_step();
        let call = sample_erc20_call();
        let addresses = vec![sample_inventory_address("0x1")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        let outcome = apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

        assert_eq!(outcome.status, WalletSimulationStatus::Blocked);
        assert_eq!(outcome.blocker, Some("simulation_blocked"));
        assert_eq!(
            outcome.evidence,
            vec![
                "fee_basis=static_profile".to_string(),
                "fee_basis_resolved_at_unix=1".to_string(),
                "gas_policy=profile_max_fee".to_string(),
                "max_priority_fee_per_gas_hex=0x1".to_string(),
                "max_fee_per_gas_hex=0x2".to_string(),
                "transaction_gas_limit=65000".to_string(),
                "native_balance_wei_hex=0x1".to_string(),
                "estimated_gas_cost_wei_hex=0x1fbd0".to_string(),
                "gas_policy_blocker=insufficient_native_gas".to_string(),
            ]
        );
    }

    #[test]
    fn zero_value_gas_policy_blocks_when_inventory_address_is_missing() {
        let provider = sample_provider();
        let step = sample_erc20_step();
        let call = sample_erc20_call();
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        let outcome = apply_zero_value_transaction_gas_policy(
            &provider,
            &[],
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap_err();

        assert_eq!(outcome.status, "blocked");
        assert!(
            outcome
                .evidence
                .iter()
                .any(|item| item == "gas_policy_blocker=missing_inventory_address")
        );
    }

    #[test]
    fn native_sweep_fee_policy_credits_pending_topup_into_spendable() {
        let provider = sample_provider();
        let step = sample_step("0x1");
        let mut call = sample_call();
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();
        let credit = decode_quantity_hex("0x10000").unwrap();

        apply_native_sweep_fee_policy(
            &provider,
            &step,
            &mut call,
            &mut evidence,
            Some(&fee_basis),
            Some(credit),
        )
        .unwrap();

        assert_eq!(call.value_hex, Some("0x5bf1".into()));
        assert!(
            evidence
                .iter()
                .any(|item| item == "pending_gas_topup_wei_hex=0x10000")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "native_sweep_spendable_amount_hex=0x5bf1")
        );
    }

    #[test]
    fn nft_sweep_gas_policy_uses_conservative_floor() {
        let provider = sample_provider();
        let step = sample_nft_step();
        let call = sample_nft_call();
        let addresses = vec![sample_inventory_address("0x40000")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap();

        assert!(
            evidence
                .iter()
                .any(|item| item == "transaction_gas_limit=100000")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "estimated_gas_cost_wei_hex=0x30d40")
        );
    }

    #[test]
    fn claim_steps_keep_execution_blocker_but_allow_simulation() {
        let mut step = sample_step("0xf4240");
        step.action = "claim_reward".into();
        step.blockers = vec!["claim_execution_disabled".into()];

        assert!(non_simulation_blockers(&step).is_none());

        apply_simulation_outcome(
            &mut step,
            PlanSimulationOutcome {
                status: "passed".into(),
                blocker: None,
                evidence: vec!["prepared_call=claim.merkle_distributor_v1".into()],
            },
        );

        assert_eq!(step.simulation_status, "passed");
        assert_eq!(step.status, "blocked");
        assert!(step.blockers.contains(&"claim_execution_disabled".into()));
    }

    #[test]
    fn claim_gas_policy_uses_conservative_floor() {
        let provider = sample_provider();
        let mut step = sample_erc20_step();
        step.action = "claim_reward".into();
        let call = sample_erc20_call();
        let addresses = vec![sample_inventory_address("0x40000")];
        let mut evidence = Vec::new();
        let fee_basis = static_fee_basis();

        apply_zero_value_transaction_gas_policy(
            &provider,
            &addresses,
            &step,
            &call,
            &mut evidence,
            Some(&fee_basis),
            None,
        )
        .unwrap();

        assert!(
            evidence
                .iter()
                .any(|item| item == "transaction_gas_limit=120000")
        );
        assert!(
            evidence
                .iter()
                .any(|item| item == "estimated_gas_cost_wei_hex=0x3a980")
        );
    }

    #[test]
    fn defi_expected_assets_evidence_records_aave_single_word() {
        let mut step = sample_step("0xf4240");
        step.action = "exit_defi_position".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW.into());

        let evidence = defi_expected_output_evidence(
            &step,
            "0x00000000000000000000000000000000000000000000000000000000000f0000",
        );

        assert_eq!(evidence, vec!["expected_assets_out_hex=0xf0000"]);
    }

    #[test]
    fn defi_expected_assets_evidence_records_erc4626_zero_word() {
        let mut step = sample_step("0xf4240");
        step.action = "exit_defi_position".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_ERC4626_REDEEM.into());

        let evidence = defi_expected_output_evidence(
            &step,
            "0x0000000000000000000000000000000000000000000000000000000000000000",
        );

        assert_eq!(evidence, vec!["expected_assets_out_hex=0x0"]);
    }

    #[test]
    fn defi_expected_assets_evidence_records_lido_wsteth_unwrap_single_word() {
        let mut step = sample_step("0xf4240");
        step.action = "exit_defi_position".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP.into());

        let evidence = defi_expected_output_evidence(
            &step,
            "0x00000000000000000000000000000000000000000000000000000000000f5000",
        );

        assert_eq!(evidence, vec!["expected_assets_out_hex=0xf5000"]);
    }

    #[test]
    fn defi_expected_assets_evidence_records_uniswap_v2_two_words() {
        let mut step = sample_step("0xf4240");
        step.action = "exit_defi_position".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_UNISWAP_V2_REMOVE_LIQUIDITY.into());

        let evidence = defi_expected_output_evidence(
            &step,
            &format!(
                "{}{}",
                abi_word_without_prefix(0x16e360),
                abi_word_without_prefix(0x2dc6c0)
            ),
        );

        assert_eq!(
            evidence,
            vec![
                "expected_amount0_out_hex=0x16e360",
                "expected_amount1_out_hex=0x2dc6c0"
            ]
        );
    }

    #[test]
    fn defi_expected_assets_evidence_ignores_malformed_or_unsupported_results() {
        let mut step = sample_step("0xf4240");
        step.action = "exit_defi_position".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_ERC4626_REDEEM.into());
        assert!(defi_expected_output_evidence(&step, "0xf0000").is_empty());
        assert_eq!(
            defi_expected_output_evidence(
                &step,
                "0x00000000000000000000000000000000000000000000000000000000000f000g",
            ),
            Vec::<String>::new()
        );

        step.claim_adapter = Some("unsupported-adapter".into());
        assert_eq!(
            defi_expected_output_evidence(
                &step,
                "0x00000000000000000000000000000000000000000000000000000000000f0000",
            ),
            Vec::<String>::new()
        );

        step.action = "sweep_erc20".into();
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW.into());
        assert_eq!(
            defi_expected_output_evidence(
                &step,
                "0x00000000000000000000000000000000000000000000000000000000000f0000",
            ),
            Vec::<String>::new()
        );
    }

    fn abi_word_without_prefix(value: u64) -> String {
        format!("{value:064x}")
    }

    #[test]
    fn parse_simulated_at_unix_returns_last_timestamp() {
        let evidence = vec![
            "simulated_at_unix=10".to_string(),
            "fee_basis=static_profile".to_string(),
            "simulated_at_unix=20".to_string(),
        ];
        assert_eq!(parse_simulated_at_unix(&evidence), Some(20));

        let invalid_last = vec![
            "simulated_at_unix=10".to_string(),
            "simulated_at_unix=not-a-number".to_string(),
        ];
        assert_eq!(parse_simulated_at_unix(&invalid_last), None);
        assert_eq!(
            parse_simulated_at_unix(&["fee_basis=estimated".into()]),
            None
        );
    }
}
