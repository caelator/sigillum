use sigillum_api::{ConsolidationPlanStep, ConsolidationPlanSummary, WalletAssetHolding};
use sigillum_core::decode_quantity_hex;

use crate::service::helpers::random_id;

use super::allowance_discovery::DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE;
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
        if action == "revoke_approval" {
            blockers.push("unsupported_approval_source".into());
        }
    } else if matches!(holding.asset_kind.as_str(), "defi" | "airdrop" | "reward") {
        blockers.push("requires_protocol_adapter".into());
    }
    let status = if blockers.is_empty() {
        "review_required"
    } else {
        "blocked"
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
        amount_hex: holding.amount_hex.clone(),
        destination_address,
        signer_status: signer_status.into(),
        simulation_status: if blockers.is_empty() {
            "required".into()
        } else {
            "not_run".into()
        },
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
        let permit2 = plan_step_for_holding(
            &sample_holding(
                "approval",
                DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE,
                Some("0x4444444444444444444444444444444444444444"),
            ),
            None,
            "available",
        );
        assert_eq!(permit2.action, "revoke_permit2_allowance");

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
}
