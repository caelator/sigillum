use sha3::{Digest, Keccak256};
use sigillum_api::ConsolidationPlanStep;

use crate::service::evm::normalize_address;
use crate::service::{ServiceError, ServiceResult};

const ERC20_APPROVE_SELECTOR: &str = "095ea7b3";
const ERC20_TRANSFER_SELECTOR: &str = "a9059cbb";
const NFT_SET_APPROVAL_FOR_ALL_SELECTOR: &str = "a22cb465";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlanStepPreflightCall {
    pub(super) label: &'static str,
    pub(super) target_address: String,
    pub(super) data_hex: String,
    pub(super) value_hex: Option<String>,
    pub(super) evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PlanStepPreflight {
    Call(PlanStepPreflightCall),
    Unsupported { evidence: Vec<String> },
}

pub(super) fn prepare_plan_step_preflight(
    step: &ConsolidationPlanStep,
) -> ServiceResult<PlanStepPreflight> {
    match step.action.as_str() {
        "sweep_native" => {
            let destination = required_address("destination", step.destination_address.as_deref())?;
            let value = required_quantity("sweep amount", &step.amount_hex)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "native.transfer(value)",
                target_address: destination.clone(),
                data_hex: "0x".into(),
                value_hex: Some(value.clone()),
                evidence: vec![
                    "prepared_call=native.transfer(value)".into(),
                    format!("destination={destination}"),
                    format!("value={value}"),
                    "fee_policy=not_estimated".into(),
                ],
            }))
        }
        "sweep_erc20" => {
            let token = required_address("asset contract", step.asset_address.as_deref())?;
            let destination = required_address("destination", step.destination_address.as_deref())?;
            let amount = required_quantity("sweep amount", &step.amount_hex)?;
            let data_hex = erc20_transfer_call_data(&destination, &amount)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "erc20.transfer(destination,amount)",
                target_address: token,
                data_hex,
                value_hex: None,
                evidence: vec![
                    "prepared_call=erc20.transfer(destination,amount)".into(),
                    format!("destination={destination}"),
                    format!("amount={amount}"),
                ],
            }))
        }
        "revoke_erc20_approval" => {
            let token = required_address("asset contract", step.asset_address.as_deref())?;
            let spender = required_address("spender", step.counterparty_address.as_deref())?;
            let data_hex = erc20_revoke_approval_call_data(&spender)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "erc20.approve(spender,0)",
                target_address: token,
                data_hex,
                value_hex: None,
                evidence: vec![
                    "prepared_call=erc20.approve(spender,0)".into(),
                    format!("spender={spender}"),
                ],
            }))
        }
        "revoke_nft_operator_approval" => {
            let collection = required_address("asset contract", step.asset_address.as_deref())?;
            let operator = required_address("operator", step.counterparty_address.as_deref())?;
            let data_hex = nft_revoke_operator_call_data(&operator)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "nft.setApprovalForAll(operator,false)",
                target_address: collection,
                data_hex,
                value_hex: None,
                evidence: vec![
                    "prepared_call=nft.setApprovalForAll(operator,false)".into(),
                    format!("operator={operator}"),
                ],
            }))
        }
        "revoke_permit2_allowance" => {
            let permit2 = required_address("Permit2 contract", step.protocol_address.as_deref())?;
            let token = required_address("asset contract", step.asset_address.as_deref())?;
            let spender = required_address("spender", step.counterparty_address.as_deref())?;
            let data_hex = permit2_revoke_allowance_call_data(&token, &spender)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "permit2.approve(token,spender,0,0)",
                target_address: permit2,
                data_hex,
                value_hex: None,
                evidence: vec![
                    "prepared_call=permit2.approve(token,spender,0,0)".into(),
                    format!("token={token}"),
                    format!("spender={spender}"),
                ],
            }))
        }
        action => Ok(PlanStepPreflight::Unsupported {
            evidence: vec![
                format!("unsupported_action={action}"),
                "reason=no_local_transaction_builder_for_step".into(),
            ],
        }),
    }
}

fn erc20_transfer_call_data(destination_address: &str, amount_hex: &str) -> ServiceResult<String> {
    Ok(format!(
        "0x{ERC20_TRANSFER_SELECTOR}{}{}",
        encoded_address_arg(destination_address)?,
        encoded_quantity_arg(amount_hex, "sweep amount")?
    ))
}

fn erc20_revoke_approval_call_data(spender_address: &str) -> ServiceResult<String> {
    Ok(format!(
        "0x{ERC20_APPROVE_SELECTOR}{}{}",
        encoded_address_arg(spender_address)?,
        zero_word()
    ))
}

fn nft_revoke_operator_call_data(operator_address: &str) -> ServiceResult<String> {
    Ok(format!(
        "0x{NFT_SET_APPROVAL_FOR_ALL_SELECTOR}{}{}",
        encoded_address_arg(operator_address)?,
        zero_word()
    ))
}

fn permit2_revoke_allowance_call_data(
    token_address: &str,
    spender_address: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}{}{}{}",
        function_selector_hex("approve(address,address,uint160,uint48)"),
        encoded_address_arg(token_address)?,
        encoded_address_arg(spender_address)?,
        zero_word(),
        zero_word()
    ))
}

fn required_address(field: &str, value: Option<&str>) -> ServiceResult<String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServiceError::bad_request(format!("{field} is required for simulation")))?;
    normalize_address(value)
}

fn required_quantity(field: &str, value: &str) -> ServiceResult<String> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.is_empty() || raw.len() > 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(format!(
            "Invalid {field} encoding."
        )));
    }
    let raw = raw.trim_start_matches('0');
    let normalized = if raw.is_empty() { "0" } else { raw };
    Ok(format!("0x{}", normalized.to_ascii_lowercase()))
}

fn encoded_address_arg(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("{}{}", "0".repeat(24), &normalized[2..]))
}

fn encoded_quantity_arg(value: &str, field: &str) -> ServiceResult<String> {
    let normalized = required_quantity(field, value)?;
    let raw = &normalized[2..];
    Ok(format!("{}{}", "0".repeat(64 - raw.len()), raw))
}

fn zero_word() -> String {
    "0".repeat(64)
}

fn function_selector_hex(signature: &str) -> String {
    let digest = Keccak256::digest(signature.as_bytes());
    hex::encode(&digest[..4])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_step(action: &str) -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: "step_1".into(),
            action: action.into(),
            status: "review_required".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: "approval".into(),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
            token_id_hex: None,
            counterparty_address: Some("0x3333333333333333333333333333333333333333".into()),
            protocol_address: Some("0x000000000022d473030f116ddee9f6b43ac78ba3".into()),
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "required".into(),
            simulation_evidence: Vec::new(),
            risk_level: "high".into(),
            blockers: Vec::new(),
            auto_eligible: false,
            approved: false,
        }
    }

    #[test]
    fn prepares_native_sweep_value_call() {
        let prepared = prepare_plan_step_preflight(&sample_step("sweep_native")).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.target_address,
            "0x9999999999999999999999999999999999999999"
        );
        assert_eq!(call.data_hex, "0x");
        assert_eq!(call.value_hex, Some("0x1".into()));
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "fee_policy=not_estimated")
        );
    }

    #[test]
    fn prepares_erc20_transfer_call_data() {
        let mut step = sample_step("sweep_erc20");
        step.amount_hex = "0X000f4240".into();
        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.data_hex,
            format!(
                "0xa9059cbb{}9999999999999999999999999999999999999999{}0f4240",
                "0".repeat(24),
                "0".repeat(58)
            )
        );
        assert_eq!(call.value_hex, None);
        assert!(call.evidence.iter().any(|item| item == "amount=0xf4240"));
    }

    #[test]
    fn prepares_erc20_approve_zero_call_data() {
        let prepared = prepare_plan_step_preflight(&sample_step("revoke_erc20_approval")).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.data_hex,
            format!(
                "0x095ea7b3{}3333333333333333333333333333333333333333{}",
                "0".repeat(24),
                "0".repeat(64)
            )
        );
    }

    #[test]
    fn prepares_nft_set_approval_for_all_false_call_data() {
        let prepared =
            prepare_plan_step_preflight(&sample_step("revoke_nft_operator_approval")).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.data_hex,
            format!(
                "0xa22cb465{}3333333333333333333333333333333333333333{}",
                "0".repeat(24),
                "0".repeat(64)
            )
        );
    }

    #[test]
    fn prepares_permit2_approve_zero_expiration_call_data() {
        let prepared =
            prepare_plan_step_preflight(&sample_step("revoke_permit2_allowance")).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.target_address,
            "0x000000000022d473030f116ddee9f6b43ac78ba3"
        );
        assert!(call.data_hex.starts_with(&format!(
            "0x{}",
            function_selector_hex("approve(address,address,uint160,uint48)")
        )));
        assert!(call.data_hex.ends_with(&"0".repeat(128)));
    }

    #[test]
    fn permit2_revoke_requires_protocol_contract() {
        let mut step = sample_step("revoke_permit2_allowance");
        step.protocol_address = None;
        let error = prepare_plan_step_preflight(&step).unwrap_err();
        assert!(error.to_string().contains("Permit2 contract"));
    }
}
