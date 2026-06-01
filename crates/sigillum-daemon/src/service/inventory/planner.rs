use sigillum_api::{ConsolidationPlanStep, ConsolidationPlanSummary, WalletAssetHolding};
use sigillum_core::decode_quantity_hex;

use crate::service::helpers::random_id;

use super::allowance_discovery::DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE;
use super::claim_discovery::CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1;
use super::nft_approval_discovery::DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE;
use super::permit2_discovery::DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE;
use super::support::quantity_hex_is_nonzero;
use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB};

pub(super) fn signer_status_for_holding(holding: &WalletAssetHolding) -> &'static str {
    match holding.wallet_family.as_str() {
        WALLET_FAMILY_ETH_XPUB => "watch_only",
        WALLET_FAMILY_ETH_SEED => "available",
        _ => "unknown",
    }
}

pub(super) fn plan_step_for_holding(
    holding: &WalletAssetHolding,
    destination_address: Option<String>,
    signer_status: &str,
) -> ConsolidationPlanStep {
    let action = action_for_holding(holding);
    let destination_address = if action.starts_with("sweep") {
        destination_address
    } else {
        None
    };
    let mut blockers = Vec::new();
    if destination_address.is_none() && action.starts_with("sweep") {
        blockers.push("missing_destination".into());
    }
    if signer_status != "available" {
        blockers.push(signer_status.to_string());
    }
    if holding.asset_kind == "approval" {
        if holding.asset_address.is_none() {
            blockers.push("missing_asset_contract".into());
        }
        if holding.counterparty_address.is_none() {
            blockers.push("missing_spender_or_operator".into());
        }
        if action == "revoke_permit2_allowance" && holding.protocol_address.is_none() {
            blockers.push("missing_permit2_contract".into());
        }
        if action == "revoke_approval" {
            blockers.push("unsupported_approval_source".into());
        }
    } else if action == "sweep_nft" {
        if holding.asset_address.is_none() {
            blockers.push("missing_asset_contract".into());
        }
        if holding.token_id_hex.is_none() {
            blockers.push("missing_token_id".into());
        }
        if holding.asset_kind == "erc1155" && !quantity_hex_is_nonzero(&holding.amount_hex) {
            blockers.push("missing_nft_amount".into());
        }
        if !matches!(holding.asset_kind.as_str(), "erc721" | "erc1155") {
            blockers.push("unsupported_nft_standard".into());
        }
    } else if holding.asset_kind == "defi" {
        blockers.push("requires_protocol_adapter".into());
    } else if matches!(holding.asset_kind.as_str(), "airdrop" | "reward") {
        push_claim_reward_blockers(holding, &mut blockers);
    }
    let status = if blockers.is_empty() {
        "review_required"
    } else {
        "blocked"
    };
    let simulation_status = if blockers.is_empty() || claim_reward_is_simulatable(action, &blockers)
    {
        "required"
    } else {
        "not_run"
    };

    ConsolidationPlanStep {
        id: random_id(),
        action: action.into(),
        status: status.into(),
        wallet_family: holding.wallet_family.clone(),
        wallet_profile: holding.wallet_profile.clone(),
        provider_profile: holding.provider_profile.clone(),
        chain_id: holding.chain_id,
        address: holding.address.clone(),
        derivation_path: holding.derivation_path.clone(),
        asset_kind: holding.asset_kind.clone(),
        asset_address: holding.asset_address.clone(),
        token_id_hex: holding.token_id_hex.clone(),
        counterparty_address: holding.counterparty_address.clone(),
        protocol_address: holding.protocol_address.clone(),
        claim_adapter: holding.claim_adapter.clone(),
        claim_index_hex: holding.claim_index_hex.clone(),
        claim_proof: holding.claim_proof.clone(),
        amount_hex: holding.amount_hex.clone(),
        destination_address,
        signer_status: signer_status.into(),
        simulation_status: simulation_status.into(),
        simulation_evidence: Vec::new(),
        risk_level: if blockers.is_empty() {
            risk_level_for_holding(holding).into()
        } else {
            "blocked".into()
        },
        blockers,
        auto_eligible: false,
        approved: false,
    }
}

fn push_claim_reward_blockers(holding: &WalletAssetHolding, blockers: &mut Vec<String>) {
    if holding.protocol_address.is_none() {
        blockers.push("missing_claim_contract".into());
    }
    match holding.claim_adapter.as_deref() {
        Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1) => {
            if holding.claim_index_hex.is_none() {
                blockers.push("missing_claim_index".into());
            }
            if holding.claim_proof.is_empty() {
                blockers.push("missing_claim_proof".into());
            }
            if holding.protocol_address.is_some()
                && holding.claim_index_hex.is_some()
                && !holding.claim_proof.is_empty()
            {
                blockers.push("claim_execution_disabled".into());
            }
        }
        _ => blockers.push("requires_protocol_adapter".into()),
    }
}

fn claim_reward_is_simulatable(action: &str, blockers: &[String]) -> bool {
    action == "claim_reward"
        && !blockers.is_empty()
        && blockers
            .iter()
            .all(|blocker| blocker == "claim_execution_disabled")
}

fn action_for_holding(holding: &WalletAssetHolding) -> &'static str {
    match holding.asset_kind.as_str() {
        "native" => "sweep_native",
        "erc20" => "sweep_erc20",
        "erc721" | "erc1155" | "nft" => "sweep_nft",
        "approval" => match holding.source.as_str() {
            DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE => "revoke_erc20_approval",
            DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE => "revoke_permit2_allowance",
            DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE => "revoke_nft_operator_approval",
            _ => "revoke_approval",
        },
        "defi" => "exit_defi_position",
        "airdrop" | "reward" => "claim_reward",
        _ => "review_asset",
    }
}

fn risk_level_for_holding(holding: &WalletAssetHolding) -> &'static str {
    if holding.asset_kind == "approval" {
        if holding.source == DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE
            || is_very_large_approval(&holding.amount_hex)
        {
            "high"
        } else {
            "medium"
        }
    } else {
        "low"
    }
}

fn is_very_large_approval(amount_hex: &str) -> bool {
    decode_quantity_hex(amount_hex)
        .map(|bytes| bytes[..16].iter().any(|byte| *byte != 0))
        .unwrap_or(false)
}

pub(super) fn summarize_plan_steps(steps: &[ConsolidationPlanStep]) -> ConsolidationPlanSummary {
    ConsolidationPlanSummary {
        total_steps: steps.len(),
        blocked_steps: steps.iter().filter(|step| step.status == "blocked").count(),
        review_required_steps: steps
            .iter()
            .filter(|step| step.status == "review_required")
            .count(),
        approved_steps: steps.iter().filter(|step| step.approved).count(),
        executable_steps: steps
            .iter()
            .filter(|step| {
                step.status == "approved"
                    && step.blockers.is_empty()
                    && step.simulation_status == "passed"
            })
            .count(),
        value_items: steps
            .iter()
            .filter(|step| quantity_hex_is_nonzero(&step.amount_hex))
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_holding(
        asset_kind: &str,
        source: &str,
        counterparty_address: Option<&str>,
    ) -> WalletAssetHolding {
        WalletAssetHolding {
            id: "holding_1".into(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: asset_kind.into(),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
            token_id_hex: None,
            counterparty_address: counterparty_address.map(str::to_string),
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            amount_hex: "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
            source: source.into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    #[test]
    fn approval_holdings_plan_reviewable_specific_revoke_steps() {
        let holding = sample_holding(
            "approval",
            DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE,
            Some("0x3333333333333333333333333333333333333333"),
        );

        let step = plan_step_for_holding(
            &holding,
            Some("0x9999999999999999999999999999999999999999".into()),
            "available",
        );

        assert_eq!(step.action, "revoke_erc20_approval");
        assert_eq!(step.status, "review_required");
        assert_eq!(step.simulation_status, "required");
        assert_eq!(step.destination_address, None);
        assert_eq!(
            step.counterparty_address.as_deref(),
            Some("0x3333333333333333333333333333333333333333")
        );
        assert!(step.blockers.is_empty());
        assert_eq!(step.risk_level, "high");
    }

    #[test]
    fn permit2_and_nft_approvals_get_distinct_actions() {
        let mut permit2_holding = sample_holding(
            "approval",
            DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE,
            Some("0x4444444444444444444444444444444444444444"),
        );
        let permit2_missing_contract = plan_step_for_holding(&permit2_holding, None, "available");
        assert_eq!(permit2_missing_contract.action, "revoke_permit2_allowance");
        assert!(
            permit2_missing_contract
                .blockers
                .iter()
                .any(|blocker| blocker == "missing_permit2_contract")
        );

        permit2_holding.protocol_address =
            Some("0x000000000022d473030f116ddee9f6b43ac78ba3".into());
        let permit2 = plan_step_for_holding(&permit2_holding, None, "available");
        assert_eq!(permit2.action, "revoke_permit2_allowance");
        assert_eq!(
            permit2.protocol_address.as_deref(),
            Some("0x000000000022d473030f116ddee9f6b43ac78ba3")
        );
        assert!(permit2.blockers.is_empty());

        let nft = plan_step_for_holding(
            &sample_holding(
                "approval",
                DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE,
                Some("0x5555555555555555555555555555555555555555"),
            ),
            None,
            "available",
        );
        assert_eq!(nft.action, "revoke_nft_operator_approval");
        assert_eq!(nft.risk_level, "high");
    }

    #[test]
    fn approval_revoke_steps_block_when_counterparty_is_missing_or_watch_only() {
        let step = plan_step_for_holding(
            &sample_holding("approval", DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE, None),
            None,
            "watch_only",
        );

        assert_eq!(step.status, "blocked");
        assert!(
            step.blockers
                .contains(&"missing_spender_or_operator".into())
        );
        assert!(step.blockers.contains(&"watch_only".into()));
    }

    #[test]
    fn approved_steps_are_not_executable_until_simulation_passes() {
        let mut step = plan_step_for_holding(
            &sample_holding(
                "approval",
                DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE,
                Some("0x3333333333333333333333333333333333333333"),
            ),
            None,
            "available",
        );
        step.status = "approved".into();
        let summary = summarize_plan_steps(&[step.clone()]);
        assert_eq!(summary.executable_steps, 0);

        step.simulation_status = "passed".into();
        let summary = summarize_plan_steps(&[step]);
        assert_eq!(summary.executable_steps, 1);
    }

    #[test]
    fn nft_sweeps_require_standard_contract_and_token_id() {
        let mut holding = sample_holding("erc721", "erc721-transfer-log", None);
        holding.amount_hex = "0x1".into();
        holding.token_id_hex = Some("0x7b".into());

        let step = plan_step_for_holding(
            &holding,
            Some("0x9999999999999999999999999999999999999999".into()),
            "available",
        );
        assert_eq!(step.action, "sweep_nft");
        assert_eq!(step.status, "review_required");
        assert!(step.blockers.is_empty());

        holding.token_id_hex = None;
        let missing_token = plan_step_for_holding(
            &holding,
            Some("0x9999999999999999999999999999999999999999".into()),
            "available",
        );
        assert_eq!(missing_token.status, "blocked");
        assert!(missing_token.blockers.contains(&"missing_token_id".into()));

        holding.asset_kind = "nft".into();
        holding.token_id_hex = Some("0x7b".into());
        let generic = plan_step_for_holding(
            &holding,
            Some("0x9999999999999999999999999999999999999999".into()),
            "available",
        );
        assert_eq!(generic.status, "blocked");
        assert!(
            generic
                .blockers
                .contains(&"unsupported_nft_standard".into())
        );
    }

    #[test]
    fn erc1155_sweeps_require_positive_amount() {
        let mut holding = sample_holding("erc1155", "erc1155-transfer-log", None);
        holding.token_id_hex = Some("0x7b".into());
        holding.amount_hex = "0x0".into();

        let step = plan_step_for_holding(
            &holding,
            Some("0x9999999999999999999999999999999999999999".into()),
            "available",
        );

        assert_eq!(step.status, "blocked");
        assert!(step.blockers.contains(&"missing_nft_amount".into()));
    }

    #[test]
    fn merkle_claim_steps_are_simulatable_but_execution_blocked() {
        let mut holding = sample_holding("reward", "claim-candidate:reward:op:list", None);
        holding.protocol_address = Some("0x1111111111111111111111111111111111111111".into());
        holding.claim_adapter = Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into());
        holding.claim_index_hex = Some("0x7".into());
        holding.claim_proof = vec![format!("0x{}", "11".repeat(32))];

        let step = plan_step_for_holding(&holding, None, "available");

        assert_eq!(step.action, "claim_reward");
        assert_eq!(step.status, "blocked");
        assert_eq!(step.simulation_status, "required");
        assert_eq!(
            step.claim_adapter.as_deref(),
            Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1)
        );
        assert!(step.blockers.contains(&"claim_execution_disabled".into()));
        assert!(!step.blockers.contains(&"requires_protocol_adapter".into()));
    }

    #[test]
    fn claim_steps_without_adapter_remain_blocked_before_simulation() {
        let mut holding = sample_holding("airdrop", "claim-candidate:airdrop:op:list", None);
        holding.protocol_address = Some("0x1111111111111111111111111111111111111111".into());

        let step = plan_step_for_holding(&holding, None, "available");

        assert_eq!(step.action, "claim_reward");
        assert_eq!(step.status, "blocked");
        assert_eq!(step.simulation_status, "not_run");
        assert!(step.blockers.contains(&"requires_protocol_adapter".into()));
    }
}
