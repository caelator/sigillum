use std::collections::{HashMap, HashSet};

use sigillum_api::{
    ConsolidationPlan, ConsolidationPlanExportBundle, ConsolidationPlanExportCall,
    ConsolidationPlanExportRequest, ConsolidationPlanExportResponse,
    ConsolidationPlanExportSkippedStep, ConsolidationPlanStep, SafeTransactionBuilderBatch,
    SafeTransactionBuilderMeta, SafeTransactionBuilderTransaction, WalletPlanStepAction,
    WalletPlanStepStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::service::evm::normalize_address;
use crate::service::helpers::map_wallet_error;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::preflight::{PlanStepPreflight, PlanStepPreflightCall, prepare_plan_step_preflight};
use super::support::load_inventory_state;

const EXPORT_FORMAT_CALL_MANIFEST: &str = "call_manifest";
const EXPORT_FORMAT_SAFE_TX_BUILDER: &str = "safe_tx_builder";

impl SigillumService {
    pub(crate) fn export_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanExportRequest,
    ) -> ServiceResult<ConsolidationPlanExportResponse> {
        let token = self.require_session(token)?;
        let format = normalized_export_format(body.format.as_deref())?;
        let safe_address = body
            .safe_address
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(normalize_address)
            .transpose()?;
        if format == EXPORT_FORMAT_SAFE_TX_BUILDER && safe_address.is_none() {
            return Err(ServiceError::bad_request(
                "safe_address is required for safe_tx_builder export",
            ));
        }

        let state = load_inventory_state(&self.state.base_dir)?;
        let plan = state
            .consolidation_plans
            .iter()
            .find(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        if selected_step_indexes(plan, &body.step_ids).is_empty() {
            return Err(ServiceError::bad_request(
                "No matching consolidation plan steps found.",
            ));
        }

        let (bundles, skipped_steps) =
            build_export(plan, &body.step_ids, &format, safe_address.as_deref())?;
        let exported_steps = bundles.iter().map(|bundle| bundle.calls.len()).sum();
        let response = ConsolidationPlanExportResponse {
            status: "exported".into(),
            plan_id: plan.id.clone(),
            format,
            exported_steps,
            skipped_steps,
            bundles,
        };

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanExport {
                id: response.plan_id.clone(),
                format: response.format.clone(),
                exported: response.exported_steps,
                skipped: response.skipped_steps.len(),
            },
        )?;

        Ok(response)
    }
}

fn build_export(
    plan: &ConsolidationPlan,
    step_ids: &[String],
    format: &str,
    safe_address: Option<&str>,
) -> ServiceResult<(
    Vec<ConsolidationPlanExportBundle>,
    Vec<ConsolidationPlanExportSkippedStep>,
)> {
    if dependencies_contain_cycle(plan) {
        return Err(ServiceError::bad_request(
            "Consolidation plan step dependencies contain a cycle.",
        ));
    }

    let selected_indexes = selected_step_indexes(plan, step_ids);
    let ordered_indexes = stable_topological_selected_indexes(plan, &selected_indexes);
    let plan_step_ids = plan
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<HashSet<_>>();
    let selected_step_ids = selected_indexes
        .iter()
        .map(|index| plan.steps[*index].id.as_str())
        .collect::<HashSet<_>>();
    let mut exported_step_ids = HashSet::<String>::new();
    let mut skipped_reasons = HashMap::<String, String>::new();
    let mut bundles = Vec::new();
    let mut skipped_steps = Vec::new();

    for step_index in ordered_indexes {
        let step = &plan.steps[step_index];
        if let Some(reason) = step_export_skip_reason(step) {
            record_skipped_step(
                &mut skipped_steps,
                &mut skipped_reasons,
                step,
                reason,
                step.blockers.clone(),
            );
            continue;
        }
        if let Some(reason) = dependency_skip_reason(
            step,
            &plan_step_ids,
            &selected_step_ids,
            &exported_step_ids,
            &skipped_reasons,
        ) {
            record_skipped_step(
                &mut skipped_steps,
                &mut skipped_reasons,
                step,
                &reason,
                Vec::new(),
            );
            continue;
        }
        if format == EXPORT_FORMAT_SAFE_TX_BUILDER
            && safe_address.is_some_and(|address| !address.eq_ignore_ascii_case(&step.address))
        {
            record_skipped_step(
                &mut skipped_steps,
                &mut skipped_reasons,
                step,
                "safe_address_mismatch",
                Vec::new(),
            );
            continue;
        }

        let call = match prepare_plan_step_preflight(step) {
            Ok(PlanStepPreflight::Call(call)) => call,
            Ok(PlanStepPreflight::Unsupported { evidence }) => {
                record_skipped_step(
                    &mut skipped_steps,
                    &mut skipped_reasons,
                    step,
                    "preflight_unsupported",
                    evidence,
                );
                continue;
            }
            Err(error) => {
                record_skipped_step(
                    &mut skipped_steps,
                    &mut skipped_reasons,
                    step,
                    "preflight_failed",
                    vec![error.to_string()],
                );
                continue;
            }
        };
        let export_call = match export_call_for_step(step, call) {
            Ok(call) => call,
            Err(error) => {
                record_skipped_step(
                    &mut skipped_steps,
                    &mut skipped_reasons,
                    step,
                    "export_call_failed",
                    vec![error.to_string()],
                );
                continue;
            }
        };
        push_export_call(&mut bundles, format, safe_address, export_call)?;
        exported_step_ids.insert(step.id.clone());
    }

    if format == EXPORT_FORMAT_SAFE_TX_BUILDER {
        let safe_address = safe_address.ok_or_else(|| {
            ServiceError::bad_request("safe_address is required for safe_tx_builder export")
        })?;
        attach_safe_batches(&mut bundles, safe_address)?;
    }

    Ok((bundles, skipped_steps))
}

pub(super) fn selected_step_indexes(plan: &ConsolidationPlan, step_ids: &[String]) -> Vec<usize> {
    let export_all = step_ids.is_empty();
    plan.steps
        .iter()
        .enumerate()
        .filter(|(_, step)| export_all || step_ids.iter().any(|id| id == &step.id))
        .map(|(index, _)| index)
        .collect()
}

pub(super) fn stable_topological_selected_indexes(
    plan: &ConsolidationPlan,
    selected_indexes: &[usize],
) -> Vec<usize> {
    let index_by_id = plan_step_index_by_id(plan);
    let selected_step_ids = selected_indexes
        .iter()
        .map(|index| plan.steps[*index].id.as_str())
        .collect::<HashSet<_>>();
    let mut indegrees = vec![0usize; plan.steps.len()];
    let mut dependents = HashMap::<usize, Vec<usize>>::new();
    for &step_index in selected_indexes {
        for dep_id in &plan.steps[step_index].depends_on {
            if let Some(&dep_index) = index_by_id.get(dep_id.as_str()) {
                if selected_step_ids.contains(dep_id.as_str()) {
                    indegrees[step_index] += 1;
                    dependents.entry(dep_index).or_default().push(step_index);
                }
            }
        }
    }

    let mut ready = selected_indexes
        .iter()
        .copied()
        .filter(|index| indegrees[*index] == 0)
        .collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(selected_indexes.len());
    while let Some((position, _)) = ready
        .iter()
        .enumerate()
        .min_by_key(|(_, index)| (plan.steps[**index].sequence, **index))
    {
        let step_index = ready.swap_remove(position);
        ordered.push(step_index);
        if let Some(dependents) = dependents.get(&step_index) {
            for dependent_index in dependents {
                indegrees[*dependent_index] -= 1;
                if indegrees[*dependent_index] == 0 {
                    ready.push(*dependent_index);
                }
            }
        }
    }
    debug_assert_eq!(ordered.len(), selected_indexes.len());
    ordered
}

pub(super) fn dependencies_contain_cycle(plan: &ConsolidationPlan) -> bool {
    let index_by_id = plan_step_index_by_id(plan);
    let mut states = vec![DependencyVisitState::Unvisited; plan.steps.len()];
    (0..plan.steps.len()).any(|step_index| {
        dependency_visit_contains_cycle(step_index, plan, &index_by_id, &mut states)
    })
}

fn dependency_visit_contains_cycle(
    step_index: usize,
    plan: &ConsolidationPlan,
    index_by_id: &HashMap<&str, usize>,
    states: &mut [DependencyVisitState],
) -> bool {
    match states[step_index] {
        DependencyVisitState::Visiting => return true,
        DependencyVisitState::Visited => return false,
        DependencyVisitState::Unvisited => {}
    }
    states[step_index] = DependencyVisitState::Visiting;
    for dep_id in &plan.steps[step_index].depends_on {
        if let Some(&dep_index) = index_by_id.get(dep_id.as_str()) {
            if dependency_visit_contains_cycle(dep_index, plan, index_by_id, states) {
                return true;
            }
        }
    }
    states[step_index] = DependencyVisitState::Visited;
    false
}

fn plan_step_index_by_id(plan: &ConsolidationPlan) -> HashMap<&str, usize> {
    plan.steps
        .iter()
        .enumerate()
        .map(|(index, step)| (step.id.as_str(), index))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DependencyVisitState {
    Unvisited,
    Visiting,
    Visited,
}

fn dependency_skip_reason(
    step: &ConsolidationPlanStep,
    plan_step_ids: &HashSet<&str>,
    selected_step_ids: &HashSet<&str>,
    exported_step_ids: &HashSet<String>,
    skipped_reasons: &HashMap<String, String>,
) -> Option<String> {
    for dep_id in &step.depends_on {
        if !plan_step_ids.contains(dep_id.as_str()) {
            return Some(format!("dependency_missing:{dep_id}"));
        }
        if let Some(reason) = skipped_reasons.get(dep_id.as_str()) {
            return if reason == "blocked" {
                Some(format!("dependency_blocked:{dep_id}"))
            } else {
                Some(format!("dependency_skipped:{dep_id}"))
            };
        }
        if !selected_step_ids.contains(dep_id.as_str()) {
            return Some(format!("dependency_not_exported:{dep_id}"));
        }
        if !exported_step_ids.contains(dep_id.as_str()) {
            return Some(format!("dependency_not_exported:{dep_id}"));
        }
    }
    None
}

fn record_skipped_step(
    skipped_steps: &mut Vec<ConsolidationPlanExportSkippedStep>,
    skipped_reasons: &mut HashMap<String, String>,
    step: &ConsolidationPlanStep,
    reason: &str,
    blockers: Vec<String>,
) {
    skipped_steps.push(skipped_step(step, reason, blockers));
    skipped_reasons.insert(step.id.clone(), reason.to_string());
}

fn normalized_export_format(value: Option<&str>) -> ServiceResult<String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(EXPORT_FORMAT_CALL_MANIFEST)
        .replace('-', "_")
        .to_ascii_lowercase();
    match value.as_str() {
        EXPORT_FORMAT_CALL_MANIFEST | EXPORT_FORMAT_SAFE_TX_BUILDER => Ok(value),
        _ => Err(ServiceError::bad_request(
            "format must be call_manifest or safe_tx_builder",
        )),
    }
}

fn step_export_skip_reason(step: &ConsolidationPlanStep) -> Option<&'static str> {
    if !step.blockers.is_empty() {
        return Some("blocked");
    }
    if step.simulation_status != WalletSimulationStatus::Passed {
        return Some("simulation_not_passed");
    }
    if step.status != WalletPlanStepStatus::Approved || !step.approved {
        return Some("not_approved");
    }
    None
}

fn skipped_step(
    step: &ConsolidationPlanStep,
    reason: &str,
    blockers: Vec<String>,
) -> ConsolidationPlanExportSkippedStep {
    ConsolidationPlanExportSkippedStep {
        step_id: step.id.clone(),
        action: step.action.clone(),
        reason: reason.into(),
        blockers,
    }
}

fn export_call_for_step(
    step: &ConsolidationPlanStep,
    call: PlanStepPreflightCall,
) -> ServiceResult<ConsolidationPlanExportCall> {
    let value_wei_hex = export_value_hex(step, &call)?;
    let mut evidence = call.evidence;
    evidence.extend(step.simulation_evidence.clone());
    Ok(ConsolidationPlanExportCall {
        step_id: step.id.clone(),
        action: step.action.clone(),
        from_address: step.address.clone(),
        to_address: call.target_address,
        value_wei_hex,
        data_hex: call.data_hex,
        operation: 0,
        chain_id: step.chain_id,
        provider_profile: step.provider_profile.clone(),
        asset_kind: step.asset_kind.clone(),
        amount_hex: step.amount_hex.clone(),
        evidence,
    })
}

fn export_value_hex(
    step: &ConsolidationPlanStep,
    call: &PlanStepPreflightCall,
) -> ServiceResult<String> {
    if step.action == WalletPlanStepAction::SweepNative {
        return step
            .simulation_evidence
            .iter()
            .find_map(|item| item.strip_prefix("native_sweep_spendable_amount_hex="))
            .map(str::to_string)
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "native sweep export requires passed simulation spendable evidence",
                )
            });
    }
    Ok(call.value_hex.clone().unwrap_or_else(|| "0x0".into()))
}

fn push_export_call(
    bundles: &mut Vec<ConsolidationPlanExportBundle>,
    format: &str,
    safe_address: Option<&str>,
    call: ConsolidationPlanExportCall,
) -> ServiceResult<()> {
    let source_address = if format == EXPORT_FORMAT_SAFE_TX_BUILDER {
        None
    } else {
        Some(call.from_address.clone())
    };
    let safe_address = safe_address.map(str::to_string);
    if let Some(bundle) = bundles.iter_mut().find(|bundle| {
        bundle.chain_id == call.chain_id
            && bundle.provider_profile == call.provider_profile
            && bundle.source_address == source_address
            && bundle.safe_address == safe_address
    }) {
        bundle.calls.push(call);
        return Ok(());
    }

    bundles.push(ConsolidationPlanExportBundle {
        chain_id: call.chain_id,
        provider_profile: call.provider_profile.clone(),
        source_address,
        safe_address,
        calls: vec![call],
        safe_transaction_builder: None,
    });
    Ok(())
}

fn attach_safe_batches(
    bundles: &mut [ConsolidationPlanExportBundle],
    safe_address: &str,
) -> ServiceResult<()> {
    for bundle in bundles {
        let transactions = bundle
            .calls
            .iter()
            .map(|call| {
                Ok(SafeTransactionBuilderTransaction {
                    to: call.to_address.clone(),
                    value: quantity_hex_to_decimal_string(&call.value_wei_hex)?,
                    data: call.data_hex.clone(),
                    operation: call.operation,
                })
            })
            .collect::<ServiceResult<Vec<_>>>()?;
        bundle.safe_transaction_builder = Some(SafeTransactionBuilderBatch {
            version: "1.0".into(),
            chain_id: bundle.chain_id.to_string(),
            meta: SafeTransactionBuilderMeta {
                name: "Sigillum consolidation export".into(),
                description: "Approved Sigillum consolidation plan calls".into(),
                tx_builder_version: "1.0".into(),
                created_from_safe_address: Some(safe_address.into()),
            },
            transactions,
        });
    }
    Ok(())
}

fn quantity_hex_to_decimal_string(value: &str) -> ServiceResult<String> {
    let bytes = decode_quantity_hex(value).map_err(map_wallet_error)?;
    let mut digits = vec![0u8];
    for byte in bytes {
        let mut carry = byte as u32;
        for digit in digits.iter_mut().rev() {
            let value = (*digit as u32) * 256 + carry;
            *digit = (value % 10) as u8;
            carry = value / 10;
        }
        while carry > 0 {
            digits.insert(0, (carry % 10) as u8);
            carry /= 10;
        }
    }
    Ok(digits
        .into_iter()
        .skip_while(|digit| *digit == 0)
        .map(|digit| char::from(b'0' + digit))
        .collect::<String>()
        .if_empty_zero())
}

trait EmptyZero {
    fn if_empty_zero(self) -> String;
}

impl EmptyZero for String {
    fn if_empty_zero(self) -> String {
        if self.is_empty() { "0".into() } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exportable_step(id: &str, sequence: u32, depends_on: Vec<String>) -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: id.into(),
            sequence,
            depends_on,
            action: "sweep_erc20".into(),
            status: "approved".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: "erc20".into(),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
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
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "passed".into(),
            simulation_evidence: Vec::new(),
            risk_level: "low".into(),
            blockers: Vec::new(),
            linkage_warnings: Vec::new(),
            auto_eligible: false,
            approved: true,
            queued_job_id: None,
        }
    }

    fn synthetic_plan(steps: Vec<ConsolidationPlanStep>) -> ConsolidationPlan {
        let total_steps = steps.len();
        ConsolidationPlan {
            id: "plan_1".into(),
            status: sigillum_api::WalletPlanStatus::Approved,
            chain_id: 1,
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            origin: None,
            created_at_unix: 1,
            updated_at_unix: 1,
            summary: sigillum_api::ConsolidationPlanSummary {
                total_steps,
                blocked_steps: 0,
                review_required_steps: 0,
                approved_steps: total_steps,
                executable_steps: total_steps,
                value_items: total_steps,
            },
            policy_violations: Vec::new(),
            linkage_findings: Vec::new(),
            steps,
        }
    }

    fn exported_step_ids(bundles: &[ConsolidationPlanExportBundle]) -> Vec<String> {
        bundles
            .iter()
            .flat_map(|bundle| bundle.calls.iter().map(|call| call.step_id.clone()))
            .collect()
    }

    fn skipped_reason<'a>(
        skipped_steps: &'a [ConsolidationPlanExportSkippedStep],
        step_id: &str,
    ) -> Option<&'a str> {
        skipped_steps
            .iter()
            .find(|step| step.step_id == step_id)
            .map(|step| step.reason.as_str())
    }

    #[test]
    fn quantity_hex_to_decimal_string_handles_large_values() {
        assert_eq!(quantity_hex_to_decimal_string("0x0").unwrap(), "0");
        assert_eq!(
            quantity_hex_to_decimal_string("0xf4240").unwrap(),
            "1000000"
        );
        assert_eq!(
            quantity_hex_to_decimal_string("0xde0b6b3a7640000").unwrap(),
            "1000000000000000000"
        );
    }

    #[test]
    fn export_orders_steps_by_dependency() {
        let step_a = exportable_step("step_a", 1, Vec::new());
        let step_b = exportable_step("step_b", 0, vec!["step_a".into()]);
        let plan = synthetic_plan(vec![step_b, step_a]);

        let (bundles, skipped_steps) =
            build_export(&plan, &[], EXPORT_FORMAT_CALL_MANIFEST, None).unwrap();

        assert!(skipped_steps.is_empty());
        assert_eq!(
            exported_step_ids(&bundles),
            vec!["step_a".to_string(), "step_b".to_string()]
        );
    }

    #[test]
    fn export_skips_dependents_of_blocked_step() {
        let mut step_a = exportable_step("step_a", 0, Vec::new());
        step_a.status = "blocked".into();
        step_a.blockers = vec!["watch_only".into()];
        let step_b = exportable_step("step_b", 1, vec!["step_a".into()]);
        let step_c = exportable_step("step_c", 2, vec!["step_b".into()]);
        let plan = synthetic_plan(vec![step_a, step_b, step_c]);

        let (bundles, skipped_steps) =
            build_export(&plan, &[], EXPORT_FORMAT_CALL_MANIFEST, None).unwrap();

        assert!(exported_step_ids(&bundles).is_empty());
        assert_eq!(skipped_reason(&skipped_steps, "step_a"), Some("blocked"));
        assert_eq!(
            skipped_reason(&skipped_steps, "step_b"),
            Some("dependency_blocked:step_a")
        );
        assert_eq!(
            skipped_reason(&skipped_steps, "step_c"),
            Some("dependency_skipped:step_b")
        );
    }

    #[test]
    fn fund_gas_export_orders_topup_before_dependent_and_carries_amount() {
        let mut fund_step = exportable_step("fund_1", 0, Vec::new());
        fund_step.action = WalletPlanStepAction::FundGas;
        fund_step.asset_kind = "native".into();
        fund_step.asset_address = None;
        fund_step.amount_hex = "0xb71b0".into();
        fund_step.address = "0x4444444444444444444444444444444444444444".into();
        fund_step.destination_address = Some("0x1111111111111111111111111111111111111111".into());
        let mut dependent = exportable_step("step_b", 1, vec!["fund_1".into()]);
        dependent.address = "0x1111111111111111111111111111111111111111".into();
        let plan = synthetic_plan(vec![dependent, fund_step]);

        let (bundles, skipped_steps) =
            build_export(&plan, &[], EXPORT_FORMAT_CALL_MANIFEST, None).unwrap();

        assert!(skipped_steps.is_empty());
        assert_eq!(
            exported_step_ids(&bundles),
            vec!["fund_1".to_string(), "step_b".to_string()]
        );
        let fund_call = bundles
            .iter()
            .flat_map(|bundle| &bundle.calls)
            .find(|call| call.step_id == "fund_1")
            .unwrap();
        assert_eq!(
            fund_call.to_address,
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(fund_call.value_wei_hex, "0xb71b0");
        assert_eq!(fund_call.data_hex, "0x");
        assert_eq!(fund_call.action, WalletPlanStepAction::FundGas);
    }

    #[test]
    fn fund_gas_export_blocked_topup_skips_dependent() {
        let mut fund_step = exportable_step("fund_1", 0, Vec::new());
        fund_step.action = WalletPlanStepAction::FundGas;
        fund_step.asset_kind = "native".into();
        fund_step.asset_address = None;
        fund_step.amount_hex = "0xb71b0".into();
        fund_step.address = "0x4444444444444444444444444444444444444444".into();
        fund_step.destination_address = Some("0x1111111111111111111111111111111111111111".into());
        fund_step.status = WalletPlanStepStatus::Blocked;
        fund_step.blockers = vec!["cross_party_linkage".into()];
        let mut dependent = exportable_step("step_b", 1, vec!["fund_1".into()]);
        dependent.address = "0x1111111111111111111111111111111111111111".into();
        let plan = synthetic_plan(vec![fund_step, dependent]);

        let (bundles, skipped_steps) =
            build_export(&plan, &[], EXPORT_FORMAT_CALL_MANIFEST, None).unwrap();

        assert!(exported_step_ids(&bundles).is_empty());
        assert_eq!(skipped_reason(&skipped_steps, "fund_1"), Some("blocked"));
        assert_eq!(
            skipped_reason(&skipped_steps, "step_b"),
            Some("dependency_blocked:fund_1")
        );
    }

    #[test]
    fn export_rejects_dependency_cycle() {
        let step_a = exportable_step("step_a", 0, vec!["step_b".into()]);
        let step_b = exportable_step("step_b", 1, vec!["step_a".into()]);
        let plan = synthetic_plan(vec![step_a, step_b]);

        let error = build_export(&plan, &[], EXPORT_FORMAT_CALL_MANIFEST, None).unwrap_err();

        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(
            error.message(),
            "Consolidation plan step dependencies contain a cycle."
        );
    }

    #[test]
    fn export_skips_step_when_dependency_not_selected() {
        let step_a = exportable_step("step_a", 0, Vec::new());
        let step_b = exportable_step("step_b", 1, vec!["step_a".into()]);
        let plan = synthetic_plan(vec![step_a, step_b]);
        let step_ids = vec!["step_b".to_string()];

        let (bundles, skipped_steps) =
            build_export(&plan, &step_ids, EXPORT_FORMAT_CALL_MANIFEST, None).unwrap();

        assert!(exported_step_ids(&bundles).is_empty());
        assert_eq!(
            skipped_reason(&skipped_steps, "step_b"),
            Some("dependency_not_exported:step_a")
        );
    }

    #[test]
    fn export_skip_reason_requires_approved_passed_unblocked_steps() {
        let mut step = ConsolidationPlanStep {
            id: "step_1".into(),
            sequence: 0,
            depends_on: Vec::new(),
            action: "sweep_erc20".into(),
            status: "review_required".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: "erc20".into(),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
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
            amount_hex: "0x1".into(),
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
        };

        assert_eq!(
            step_export_skip_reason(&step),
            Some("simulation_not_passed")
        );
        step.simulation_status = "passed".into();
        assert_eq!(step_export_skip_reason(&step), Some("not_approved"));
        step.status = "approved".into();
        step.approved = true;
        assert_eq!(step_export_skip_reason(&step), None);
        step.blockers.push("watch_only".into());
        assert_eq!(step_export_skip_reason(&step), Some("blocked"));
    }
}
