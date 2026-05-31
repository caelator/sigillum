use sigillum_api::{ConsolidationPlanStep, ConsolidationPlanSummary, WalletAssetHolding};

use crate::service::helpers::random_id;

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
    let action = match holding.asset_kind.as_str() {
        "native" => "sweep_native",
        "erc20" => "sweep_erc20",
        "erc721" | "erc1155" | "nft" => "sweep_nft",
        "approval" => "revoke_approval",
        "defi" => "exit_defi_position",
        "airdrop" | "reward" => "claim_reward",
        _ => "review_asset",
    };
    let mut blockers = Vec::new();
    if destination_address.is_none() && action.starts_with("sweep") {
        blockers.push("missing_destination".into());
    }
    if signer_status != "available" {
        blockers.push(signer_status.to_string());
    }
    let blocked_by_kind = matches!(
        holding.asset_kind.as_str(),
        "approval" | "defi" | "airdrop" | "reward"
    );
    if blocked_by_kind {
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
        amount_hex: holding.amount_hex.clone(),
        destination_address,
        signer_status: signer_status.into(),
        simulation_status: "not_run".into(),
        risk_level: if blockers.is_empty() {
            "low".into()
        } else {
            "blocked".into()
        },
        blockers,
        auto_eligible: false,
        approved: false,
    }
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
            .filter(|step| step.status == "approved" && step.blockers.is_empty())
            .count(),
        value_items: steps
            .iter()
            .filter(|step| quantity_hex_is_nonzero(&step.amount_hex))
            .count(),
    }
}
