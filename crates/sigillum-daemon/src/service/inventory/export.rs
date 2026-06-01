use sigillum_api::{
    ConsolidationPlanExportBundle, ConsolidationPlanExportCall, ConsolidationPlanExportRequest,
    ConsolidationPlanExportResponse, ConsolidationPlanExportSkippedStep, ConsolidationPlanStep,
    SafeTransactionBuilderBatch, SafeTransactionBuilderMeta, SafeTransactionBuilderTransaction,
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
        let export_all = body.step_ids.is_empty();
        let selected_steps = plan
            .steps
            .iter()
            .filter(|step| export_all || body.step_ids.iter().any(|id| id == &step.id))
            .collect::<Vec<_>>();
        if selected_steps.is_empty() {
            return Err(ServiceError::bad_request(
                "No matching consolidation plan steps found.",
            ));
        }

        let mut bundles = Vec::new();
        let mut skipped_steps = Vec::new();
        for step in selected_steps {
            if let Some(reason) = step_export_skip_reason(step) {
                skipped_steps.push(skipped_step(step, reason, step.blockers.clone()));
                continue;
            }
            if format == EXPORT_FORMAT_SAFE_TX_BUILDER
                && safe_address
                    .as_deref()
                    .is_some_and(|address| !address.eq_ignore_ascii_case(&step.address))
            {
                skipped_steps.push(skipped_step(step, "safe_address_mismatch", Vec::new()));
                continue;
            }

            let call = match prepare_plan_step_preflight(step) {
                Ok(PlanStepPreflight::Call(call)) => call,
                Ok(PlanStepPreflight::Unsupported { evidence }) => {
                    skipped_steps.push(skipped_step(step, "preflight_unsupported", evidence));
                    continue;
                }
                Err(error) => {
                    skipped_steps.push(skipped_step(
                        step,
                        "preflight_failed",
                        vec![error.to_string()],
                    ));
                    continue;
                }
            };
            let export_call = match export_call_for_step(step, call) {
                Ok(call) => call,
                Err(error) => {
                    skipped_steps.push(skipped_step(
                        step,
                        "export_call_failed",
                        vec![error.to_string()],
                    ));
                    continue;
                }
            };
            push_export_call(&mut bundles, &format, safe_address.as_deref(), export_call)?;
        }

        if format == EXPORT_FORMAT_SAFE_TX_BUILDER {
            attach_safe_batches(&mut bundles, safe_address.as_deref().unwrap())?;
        }

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
    if step.simulation_status != "passed" {
        return Some("simulation_not_passed");
    }
    if step.status != "approved" || !step.approved {
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
    if step.action == "sweep_native" {
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
    fn export_skip_reason_requires_approved_passed_unblocked_steps() {
        let mut step = ConsolidationPlanStep {
            id: "step_1".into(),
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
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "required".into(),
            simulation_evidence: Vec::new(),
            risk_level: "low".into(),
            blockers: Vec::new(),
            auto_eligible: false,
            approved: false,
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
