use sha3::{Digest, Keccak256};
use sigillum_api::{ConsolidationPlanStep, WalletAssetKind, WalletPlanStepAction};

use crate::service::evm::normalize_address;
use crate::service::{ServiceError, ServiceResult};

use super::claim_discovery::CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1;
use super::defi_adapters::{
    DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW, DEFI_EXIT_ADAPTER_ERC4626_REDEEM,
    DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP,
};

const ERC20_APPROVE_SELECTOR: &str = "095ea7b3";
const ERC20_TRANSFER_SELECTOR: &str = "a9059cbb";
const ERC721_SAFE_TRANSFER_FROM_SELECTOR: &str = "42842e0e";
const ERC1155_SAFE_TRANSFER_FROM_SELECTOR: &str = "f242432a";
const NFT_SET_APPROVAL_FOR_ALL_SELECTOR: &str = "a22cb465";
const CLAIM_PROOF_OFFSET_HEX: &str = "0x80";

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
    match &step.action {
        WalletPlanStepAction::SweepNative => {
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
                    format!("requested_value={value}"),
                ],
            }))
        }
        WalletPlanStepAction::SweepErc20 => {
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
        WalletPlanStepAction::SweepNft => {
            let collection = required_address("asset contract", step.asset_address.as_deref())?;
            let destination = required_address("destination", step.destination_address.as_deref())?;
            let token_id = required_optional_quantity("token id", step.token_id_hex.as_deref())?;
            match &step.asset_kind {
                WalletAssetKind::Erc721 => {
                    let data_hex =
                        erc721_safe_transfer_call_data(&step.address, &destination, &token_id)?;
                    Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                        label: "erc721.safeTransferFrom(owner,destination,tokenId)",
                        target_address: collection,
                        data_hex,
                        value_hex: None,
                        evidence: vec![
                            "prepared_call=erc721.safeTransferFrom(owner,destination,tokenId)"
                                .into(),
                            format!("destination={destination}"),
                            format!("token_id={token_id}"),
                        ],
                    }))
                }
                WalletAssetKind::Erc1155 => {
                    let amount = required_quantity("sweep amount", &step.amount_hex)?;
                    let data_hex = erc1155_safe_transfer_call_data(
                        &step.address,
                        &destination,
                        &token_id,
                        &amount,
                    )?;
                    Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                        label: "erc1155.safeTransferFrom(owner,destination,tokenId,amount,empty)",
                        target_address: collection,
                        data_hex,
                        value_hex: None,
                        evidence: vec![
                            "prepared_call=erc1155.safeTransferFrom(owner,destination,tokenId,amount,empty)".into(),
                            format!("destination={destination}"),
                            format!("token_id={token_id}"),
                            format!("amount={amount}"),
                        ],
                    }))
                }
                kind => Ok(PlanStepPreflight::Unsupported {
                    evidence: vec![
                        format!("unsupported_nft_asset_kind={}", kind.as_str()),
                        "reason=no_local_nft_transfer_builder_for_asset_kind".into(),
                    ],
                }),
            }
        }
        WalletPlanStepAction::RevokeErc20Approval => {
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
        WalletPlanStepAction::RevokeNftOperatorApproval => {
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
        WalletPlanStepAction::RevokePermit2Allowance => {
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
        WalletPlanStepAction::ClaimReward => {
            let adapter = step
                .claim_adapter
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ServiceError::bad_request("claim adapter is required for simulation")
                })?;
            if adapter != CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1 {
                return Ok(PlanStepPreflight::Unsupported {
                    evidence: vec![
                        format!("unsupported_claim_adapter={adapter}"),
                        "reason=no_local_claim_builder_for_adapter".into(),
                    ],
                });
            }
            let claim_contract =
                required_address("claim contract", step.protocol_address.as_deref())?;
            let account = required_address("claim account", Some(step.address.as_str()))?;
            let amount = required_quantity("claim amount", &step.amount_hex)?;
            let index = required_optional_quantity("claim index", step.claim_index_hex.as_deref())?;
            let proof = required_claim_proof(&step.claim_proof)?;
            let data_hex = merkle_claim_call_data(&index, &account, &amount, &proof)?;
            Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                label: "claim.merkleDistributor(index,account,amount,proof)",
                target_address: claim_contract,
                data_hex,
                value_hex: None,
                evidence: vec![
                    "prepared_call=claim.merkle_distributor_v1(index,account,amount,proof)".into(),
                    format!("claim_adapter={CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1}"),
                    format!("claim_index={index}"),
                    format!("claim_account={account}"),
                    format!("amount={amount}"),
                    format!("claim_proof_words={}", proof.len()),
                ],
            }))
        }
        WalletPlanStepAction::ExitDefiPosition => {
            let adapter = step
                .claim_adapter
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ServiceError::bad_request("defi exit adapter is required for simulation")
                })?;
            if adapter == DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW {
                let pool = required_address("Aave V3 pool", step.protocol_address.as_deref())?;
                let asset =
                    required_address("Aave underlying asset", step.asset_address.as_deref())?;
                let amount = required_quantity("withdraw amount", &step.amount_hex)?;
                let recipient =
                    required_address("withdraw recipient", Some(step.address.as_str()))?;
                let data_hex = aave_v3_withdraw_call_data(&asset, &amount, &recipient)?;
                return Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                    label: "aaveV3.withdraw(asset,amount,to)",
                    target_address: pool,
                    data_hex,
                    value_hex: None,
                    evidence: vec![
                        "prepared_call=aave_v3.withdraw(asset,amount,to)".into(),
                        format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW}"),
                        format!("asset={asset}"),
                        format!("recipient={recipient}"),
                        format!("amount={amount}"),
                    ],
                }));
            }
            if adapter == DEFI_EXIT_ADAPTER_ERC4626_REDEEM {
                let vault = required_address("ERC-4626 vault", step.protocol_address.as_deref())?;
                let amount = required_quantity("redeem shares", &step.amount_hex)?;
                let recipient = required_address("redeem receiver", Some(step.address.as_str()))?;
                let owner = required_address("redeem owner", Some(step.address.as_str()))?;
                let data_hex = erc4626_redeem_call_data(&amount, &recipient, &owner)?;
                return Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                    label: "erc4626.redeem(shares,receiver,owner)",
                    target_address: vault.clone(),
                    data_hex,
                    value_hex: None,
                    evidence: vec![
                        "prepared_call=erc4626.redeem(shares,receiver,owner)".into(),
                        format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_ERC4626_REDEEM}"),
                        format!("vault={vault}"),
                        format!("receiver={recipient}"),
                        format!("shares={amount}"),
                    ],
                }));
            }
            if adapter == DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP {
                let wsteth = required_address("wstETH contract", step.protocol_address.as_deref())?;
                let amount = required_quantity("unwrap amount", &step.amount_hex)?;
                let data_hex = lido_wsteth_unwrap_call_data(&amount)?;
                return Ok(PlanStepPreflight::Call(PlanStepPreflightCall {
                    label: "lido_wsteth.unwrap(amount)",
                    target_address: wsteth.clone(),
                    data_hex,
                    value_hex: None,
                    evidence: vec![
                        "prepared_call=lido_wsteth.unwrap(amount)".into(),
                        format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP}"),
                        format!("wsteth={wsteth}"),
                        format!("amount={amount}"),
                        "produces_asset=steth".into(),
                        "steth_withdrawal_queue=out_of_scope_review_asset".into(),
                    ],
                }));
            }
            Ok(PlanStepPreflight::Unsupported {
                evidence: vec![
                    format!("unsupported_defi_exit_adapter={adapter}"),
                    "reason=no_local_defi_exit_builder_for_adapter".into(),
                ],
            })
        }
        action => Ok(PlanStepPreflight::Unsupported {
            evidence: vec![
                format!("unsupported_action={}", action.as_str()),
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

fn erc721_safe_transfer_call_data(
    owner_address: &str,
    destination_address: &str,
    token_id_hex: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{ERC721_SAFE_TRANSFER_FROM_SELECTOR}{}{}{}",
        encoded_address_arg(owner_address)?,
        encoded_address_arg(destination_address)?,
        encoded_quantity_arg(token_id_hex, "token id")?
    ))
}

fn erc1155_safe_transfer_call_data(
    owner_address: &str,
    destination_address: &str,
    token_id_hex: &str,
    amount_hex: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{ERC1155_SAFE_TRANSFER_FROM_SELECTOR}{}{}{}{}{}{}",
        encoded_address_arg(owner_address)?,
        encoded_address_arg(destination_address)?,
        encoded_quantity_arg(token_id_hex, "token id")?,
        encoded_quantity_arg(amount_hex, "sweep amount")?,
        encoded_quantity_arg("0xa0", "bytes offset")?,
        zero_word()
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

fn merkle_claim_call_data(
    index_hex: &str,
    account_address: &str,
    amount_hex: &str,
    proof: &[String],
) -> ServiceResult<String> {
    let proof_words = proof
        .iter()
        .map(|word| required_proof_word(word))
        .collect::<ServiceResult<Vec<_>>>()?
        .join("");
    Ok(format!(
        "0x{}{}{}{}{}{}{}",
        function_selector_hex("claim(uint256,address,uint256,bytes32[])"),
        encoded_quantity_arg(index_hex, "claim index")?,
        encoded_address_arg(account_address)?,
        encoded_quantity_arg(amount_hex, "claim amount")?,
        encoded_quantity_arg(CLAIM_PROOF_OFFSET_HEX, "claim proof offset")?,
        encoded_quantity_arg(&format!("0x{:x}", proof.len()), "claim proof length")?,
        proof_words
    ))
}

fn aave_v3_withdraw_call_data(
    asset_address: &str,
    amount_hex: &str,
    recipient_address: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}{}{}",
        function_selector_hex("withdraw(address,uint256,address)"),
        encoded_address_arg(asset_address)?,
        encoded_quantity_arg(amount_hex, "withdraw amount")?,
        encoded_address_arg(recipient_address)?
    ))
}

fn erc4626_redeem_call_data(
    shares_hex: &str,
    receiver_address: &str,
    owner_address: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}{}{}",
        function_selector_hex("redeem(uint256,address,address)"),
        encoded_quantity_arg(shares_hex, "redeem shares")?,
        encoded_address_arg(receiver_address)?,
        encoded_address_arg(owner_address)?
    ))
}

fn lido_wsteth_unwrap_call_data(amount_hex: &str) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}",
        function_selector_hex("unwrap(uint256)"),
        encoded_quantity_arg(amount_hex, "unwrap amount")?
    ))
}

fn required_optional_quantity(field: &str, value: Option<&str>) -> ServiceResult<String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServiceError::bad_request(format!("{field} is required for simulation")))?;
    required_quantity(field, value)
}

fn required_claim_proof(values: &[String]) -> ServiceResult<Vec<String>> {
    if values.is_empty() {
        return Err(ServiceError::bad_request(
            "claim proof is required for simulation",
        ));
    }
    values
        .iter()
        .map(|value| required_proof_word(value))
        .collect()
}

fn required_proof_word(value: &str) -> ServiceResult<String> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(
            "claim proof words must be 32-byte hex values",
        ));
    }
    Ok(raw.to_ascii_lowercase())
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
            sequence: 0,
            depends_on: Vec::new(),
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
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            amount_hex: "0x1".into(),
            destination_address: Some("0x9999999999999999999999999999999999999999".into()),
            signer_status: "available".into(),
            simulation_status: "required".into(),
            simulation_evidence: Vec::new(),
            risk_level: "high".into(),
            blockers: Vec::new(),
            linkage_warnings: Vec::new(),
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
                .any(|item| item == "requested_value=0x1")
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
    fn prepares_erc721_safe_transfer_call_data() {
        let mut step = sample_step("sweep_nft");
        step.asset_kind = "erc721".into();
        step.token_id_hex =
            Some("0x000000000000000000000000000000000000000000000000000000000000007b".into());

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };

        assert_eq!(
            call.label,
            "erc721.safeTransferFrom(owner,destination,tokenId)"
        );
        assert_eq!(
            call.data_hex,
            format!(
                "0x42842e0e{}1111111111111111111111111111111111111111{}9999999999999999999999999999999999999999{}7b",
                "0".repeat(24),
                "0".repeat(24),
                "0".repeat(62)
            )
        );
        assert!(call.evidence.iter().any(|item| item == "token_id=0x7b"));
    }

    #[test]
    fn prepares_erc1155_safe_transfer_call_data() {
        let mut step = sample_step("sweep_nft");
        step.asset_kind = "erc1155".into();
        step.token_id_hex =
            Some("0x000000000000000000000000000000000000000000000000000000000000007b".into());
        step.amount_hex = "0x2a".into();

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };

        assert_eq!(
            call.label,
            "erc1155.safeTransferFrom(owner,destination,tokenId,amount,empty)"
        );
        assert_eq!(
            call.data_hex,
            format!(
                "0xf242432a{}1111111111111111111111111111111111111111{}9999999999999999999999999999999999999999{}7b{}2a{}a0{}",
                "0".repeat(24),
                "0".repeat(24),
                "0".repeat(62),
                "0".repeat(62),
                "0".repeat(62),
                "0".repeat(64)
            )
        );
        assert!(call.evidence.iter().any(|item| item == "amount=0x2a"));
    }

    #[test]
    fn nft_sweep_requires_token_id() {
        let mut step = sample_step("sweep_nft");
        step.asset_kind = "erc721".into();

        let error = prepare_plan_step_preflight(&step).unwrap_err();

        assert!(error.to_string().contains("token id"));
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

    #[test]
    fn prepares_aave_v3_withdraw_call_data() {
        let mut step = sample_step("exit_defi_position");
        step.asset_kind = "defi".into();
        step.asset_address = Some("0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".into());
        step.protocol_address = Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2".into());
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW.into());
        step.amount_hex = "0xf4240".into();

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };
        assert_eq!(
            call.target_address,
            "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2"
        );
        assert!(call.data_hex.starts_with(&format!(
            "0x{}",
            function_selector_hex("withdraw(address,uint256,address)")
        )));
        assert!(call.evidence.iter().any(|item| {
            item == &format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_AAVE_V3_WITHDRAW}")
        }));
    }

    #[test]
    fn prepares_erc4626_redeem_call_data() {
        let mut step = sample_step("exit_defi_position");
        step.asset_kind = "defi".into();
        step.asset_address = Some("0xdead111100000000000000000000000000000001".into());
        step.protocol_address = Some("0xdead4626000000000000000000000000000000aa".into());
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_ERC4626_REDEEM.into());
        step.amount_hex = "0x000e8480".into();

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };

        assert_eq!(call.label, "erc4626.redeem(shares,receiver,owner)");
        assert_eq!(
            call.target_address,
            "0xdead4626000000000000000000000000000000aa"
        );
        assert_eq!(
            call.data_hex,
            format!(
                "0x{}{}e8480{}1111111111111111111111111111111111111111{}1111111111111111111111111111111111111111",
                function_selector_hex("redeem(uint256,address,address)"),
                "0".repeat(59),
                "0".repeat(24),
                "0".repeat(24)
            )
        );
        assert!(call.evidence.iter().any(|item| {
            item == &format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_ERC4626_REDEEM}")
        }));
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "prepared_call=erc4626.redeem(shares,receiver,owner)")
        );
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "vault=0xdead4626000000000000000000000000000000aa")
        );
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "receiver=0x1111111111111111111111111111111111111111")
        );
        assert!(call.evidence.iter().any(|item| item == "shares=0xe8480"));
    }

    #[test]
    fn prepares_lido_wsteth_unwrap_call_data() {
        let mut step = sample_step("exit_defi_position");
        step.asset_kind = "defi".into();
        step.asset_address = Some("0xdead4d57e7570000000000000000000000000000".into());
        step.protocol_address = Some("0xdead4d57e7570000000000000000000000000000".into());
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP.into());
        step.amount_hex = "0x000f4240".into();

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };

        assert_eq!(call.label, "lido_wsteth.unwrap(amount)");
        assert_eq!(
            call.target_address,
            "0xdead4d57e7570000000000000000000000000000"
        );
        assert_eq!(call.data_hex, format!("0xde0e9a3e{}f4240", "0".repeat(59)));
        assert!(call.evidence.iter().any(|item| {
            item == &format!("defi_exit_adapter={DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP}")
        }));
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "prepared_call=lido_wsteth.unwrap(amount)")
        );
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "wsteth=0xdead4d57e7570000000000000000000000000000")
        );
        assert!(call.evidence.iter().any(|item| item == "amount=0xf4240"));
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "produces_asset=steth")
        );
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "steth_withdrawal_queue=out_of_scope_review_asset")
        );
    }

    #[test]
    fn lido_wsteth_unwrap_requires_contract() {
        let mut step = sample_step("exit_defi_position");
        step.asset_kind = "defi".into();
        step.protocol_address = None;
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_LIDO_WSTETH_UNWRAP.into());

        let error = prepare_plan_step_preflight(&step).unwrap_err();

        assert!(error.to_string().contains("wstETH contract"));
    }

    #[test]
    fn erc4626_redeem_requires_vault() {
        let mut step = sample_step("exit_defi_position");
        step.asset_kind = "defi".into();
        step.protocol_address = None;
        step.claim_adapter = Some(DEFI_EXIT_ADAPTER_ERC4626_REDEEM.into());

        let error = prepare_plan_step_preflight(&step).unwrap_err();

        assert!(error.to_string().contains("ERC-4626 vault"));
    }

    #[test]
    fn prepares_merkle_claim_call_data() {
        let mut step = sample_step("claim_reward");
        step.asset_kind = "reward".into();
        step.address = "0x9858effd232b4033e47d90003d41ec34ecaeda94".into();
        step.protocol_address = Some("0x1111111111111111111111111111111111111111".into());
        step.claim_adapter = Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into());
        step.claim_index_hex = Some("0x7".into());
        step.amount_hex = "0xf4240".into();
        step.claim_proof = vec![
            format!("0x{}", "11".repeat(32)),
            format!("0x{}", "22".repeat(32)),
        ];

        let prepared = prepare_plan_step_preflight(&step).unwrap();
        let PlanStepPreflight::Call(call) = prepared else {
            panic!("expected call");
        };

        assert_eq!(
            call.target_address,
            "0x1111111111111111111111111111111111111111"
        );
        assert!(call.data_hex.starts_with(&format!(
            "0x{}",
            function_selector_hex("claim(uint256,address,uint256,bytes32[])")
        )));
        assert!(
            call.data_hex
                .contains("9858effd232b4033e47d90003d41ec34ecaeda94")
        );
        assert!(
            call.data_hex
                .ends_with(&format!("{}{}", "11".repeat(32), "22".repeat(32)))
        );
        assert!(call.evidence.iter().any(|item| {
            item == "prepared_call=claim.merkle_distributor_v1(index,account,amount,proof)"
        }));
        assert!(
            call.evidence
                .iter()
                .any(|item| item == "claim_proof_words=2")
        );
    }

    #[test]
    fn merkle_claim_requires_proof_evidence() {
        let mut step = sample_step("claim_reward");
        step.protocol_address = Some("0x1111111111111111111111111111111111111111".into());
        step.claim_adapter = Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into());
        step.claim_index_hex = Some("0x7".into());
        step.claim_proof = Vec::new();

        let error = prepare_plan_step_preflight(&step).unwrap_err();

        assert!(error.to_string().contains("claim proof"));
    }
}
