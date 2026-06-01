use sha3::{Digest, Keccak256};
use sigillum_api::ConsolidationPlanStep;

use crate::service::evm::normalize_address;
use crate::service::{ServiceError, ServiceResult};

const ERC20_APPROVE_SELECTOR: &str = "095ea7b3";
const NFT_SET_APPROVAL_FOR_ALL_SELECTOR: &str = "a22cb465";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlanStepPreflightCall {
    pub(super) label: &'static str,
    pub(super) target_address: String,
    pub(super) data_hex: String,
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
        "revoke_erc20_approval" => {
            let token = required_address("asset contract", step.asset_address.as_deref())?;
            let spender = required_address("spender", step.counterparty_address.as_deref())?;
            let data_hex = erc20_revoke_approval_call_data(&spender)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "erc20.approve(spender,0)",
                target_address: token,
                data_hex,
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

fn encoded_address_arg(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("{}{}", "0".repeat(24), &normalized[2..]))
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
            destination_address: None,
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
