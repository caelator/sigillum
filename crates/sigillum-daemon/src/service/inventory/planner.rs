use std::collections::BTreeMap;

use sigillum_api::{
    ConsolidationPlanGenerateRequest, ConsolidationPlanStep, ConsolidationPlanSummary,
    TreasuryPolicy, WalletAssetHolding, WalletAssetKind, WalletPlanStepAction,
    WalletPlanStepStatus, WalletSignerStatus, WalletSimulationStatus,
};
use sigillum_core::decode_quantity_hex;

use crate::inventory::WalletInventoryState;
use crate::profiles::ProfileRegistry;
use crate::service::helpers::{compare_u256, random_id};

use super::allowance_discovery::DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE;
use super::claim_discovery::CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1;
use super::defi_adapters::supported_defi_exit_adapter;
use super::nft_approval_discovery::DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE;
use super::permit2_discovery::DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE;
use super::support::quantity_hex_is_nonzero;
use super::treasury::{add_u256, policy_blockers_for_step};
use super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH, WALLET_FAMILY_ETH_XPUB};

const DEFAULT_HOT_FLOOR_WEI_HEX: &str = "0xde0b6b3a7640000";

pub(super) fn signer_status_for_holding(holding: &WalletAssetHolding) -> WalletSignerStatus {
    match holding.wallet_family.as_str() {
        WALLET_FAMILY_ETH_XPUB | WALLET_FAMILY_ETH_WATCH => WalletSignerStatus::WatchOnly,
        WALLET_FAMILY_ETH_SEED => WalletSignerStatus::Available,
        _ => WalletSignerStatus::Unknown,
    }
}

pub(super) fn plan_step_for_holding(
    holding: &WalletAssetHolding,
    destination_address: Option<String>,
    signer_status: impl Into<WalletSignerStatus>,
) -> ConsolidationPlanStep {
    let signer_status = signer_status.into();
    let action = action_for_holding(holding);
    let destination_address = if action.as_str().starts_with("sweep") {
        destination_address
    } else {
        None
    };
    let mut blockers = Vec::new();
    if destination_address.is_none() && action.as_str().starts_with("sweep") {
        blockers.push("missing_destination".into());
    }
    if signer_status != WalletSignerStatus::Available {
        blockers.push(signer_status.as_str().to_string());
    }
    if holding.asset_kind == WalletAssetKind::Approval {
        if holding.asset_address.is_none() {
            blockers.push("missing_asset_contract".into());
        }
        if holding.counterparty_address.is_none() {
            blockers.push("missing_spender_or_operator".into());
        }
        if action == WalletPlanStepAction::RevokePermit2Allowance
            && holding.protocol_address.is_none()
        {
            blockers.push("missing_permit2_contract".into());
        }
        if action == WalletPlanStepAction::RevokeApproval {
            blockers.push("unsupported_approval_source".into());
        }
    } else if action == WalletPlanStepAction::SweepNft {
        if holding.asset_address.is_none() {
            blockers.push("missing_asset_contract".into());
        }
        if holding.token_id_hex.is_none() {
            blockers.push("missing_token_id".into());
        }
        if holding.asset_kind == WalletAssetKind::Erc1155
            && !quantity_hex_is_nonzero(&holding.amount_hex)
        {
            blockers.push("missing_nft_amount".into());
        }
        if !matches!(
            holding.asset_kind,
            WalletAssetKind::Erc721 | WalletAssetKind::Erc1155
        ) {
            blockers.push("unsupported_nft_standard".into());
        }
    } else if holding.asset_kind == WalletAssetKind::Defi {
        if holding.asset_address.is_none() {
            blockers.push("missing_asset_contract".into());
        }
        if holding.protocol_address.is_none() {
            blockers.push("missing_protocol_contract".into());
        }
        match holding.claim_adapter.as_deref() {
            Some(adapter) if supported_defi_exit_adapter(adapter) => {}
            Some(_) => blockers.push("unsupported_protocol_adapter".into()),
            None => blockers.push("requires_protocol_adapter".into()),
        }
    } else if matches!(
        holding.asset_kind,
        WalletAssetKind::Airdrop | WalletAssetKind::Reward
    ) {
        push_claim_reward_blockers(holding, &mut blockers);
    }
    let status = if blockers.is_empty() {
        WalletPlanStepStatus::ReviewRequired
    } else {
        WalletPlanStepStatus::Blocked
    };
    let simulation_status =
        if blockers.is_empty() || claim_reward_is_simulatable(&action, &blockers) {
            WalletSimulationStatus::Required
        } else {
            WalletSimulationStatus::NotRun
        };

    ConsolidationPlanStep {
        id: random_id(),
        sequence: 0,
        depends_on: Vec::new(),
        action,
        status,
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
        exit_token0_address: None,
        exit_token1_address: None,
        exit_amount0_min_hex: None,
        exit_amount1_min_hex: None,
        exit_deadline_unix: None,
        amount_hex: holding.amount_hex.clone(),
        destination_address,
        signer_status,
        simulation_status,
        simulation_evidence: Vec::new(),
        risk_level: if blockers.is_empty() {
            risk_level_for_holding(holding).into()
        } else {
            "blocked".into()
        },
        blockers,
        linkage_warnings: Vec::new(),
        auto_eligible: false,
        approved: false,
        queued_job_id: None,
    }
}

pub(in crate::service) fn assign_step_ordering(steps: &mut [ConsolidationPlanStep]) {
    for (index, step) in steps.iter_mut().enumerate() {
        step.sequence = index as u32;
    }
}

fn push_claim_reward_blockers(holding: &WalletAssetHolding, blockers: &mut Vec<String>) {
    // Claim execution starts disabled at generation: fresh steps are never
    // simulated or approved, so W5 cannot pass here. claim_gate::refresh_claim_execution_blocker
    // is the only code allowed to remove this blocker. W7.3 queue execution
    // must mark reverted claims operator_action_required and never auto-retry.
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

fn claim_reward_is_simulatable(action: &WalletPlanStepAction, blockers: &[String]) -> bool {
    action == &WalletPlanStepAction::ClaimReward
        && !blockers.is_empty()
        && blockers
            .iter()
            .all(|blocker| blocker == "claim_execution_disabled")
}

fn action_for_holding(holding: &WalletAssetHolding) -> WalletPlanStepAction {
    match &holding.asset_kind {
        WalletAssetKind::Native => WalletPlanStepAction::SweepNative,
        WalletAssetKind::Erc20 => WalletPlanStepAction::SweepErc20,
        WalletAssetKind::Erc721 | WalletAssetKind::Erc1155 | WalletAssetKind::Nft => {
            WalletPlanStepAction::SweepNft
        }
        WalletAssetKind::Approval => match holding.source.as_str() {
            DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE => WalletPlanStepAction::RevokeErc20Approval,
            DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE => {
                WalletPlanStepAction::RevokePermit2Allowance
            }
            DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE => {
                WalletPlanStepAction::RevokeNftOperatorApproval
            }
            _ => WalletPlanStepAction::RevokeApproval,
        },
        WalletAssetKind::Defi => WalletPlanStepAction::ExitDefiPosition,
        WalletAssetKind::Airdrop | WalletAssetKind::Reward => WalletPlanStepAction::ClaimReward,
        WalletAssetKind::Other(_) => WalletPlanStepAction::ReviewAsset,
    }
}

fn risk_level_for_holding(holding: &WalletAssetHolding) -> &'static str {
    if holding.asset_kind == WalletAssetKind::Approval {
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

pub(in crate::service) fn summarize_plan_steps(
    steps: &[ConsolidationPlanStep],
) -> ConsolidationPlanSummary {
    ConsolidationPlanSummary {
        total_steps: steps.len(),
        blocked_steps: steps
            .iter()
            .filter(|step| step.status == WalletPlanStepStatus::Blocked)
            .count(),
        review_required_steps: steps
            .iter()
            .filter(|step| step.status == WalletPlanStepStatus::ReviewRequired)
            .count(),
        approved_steps: steps.iter().filter(|step| step.approved).count(),
        executable_steps: steps
            .iter()
            .filter(|step| {
                step.status == WalletPlanStepStatus::Approved
                    && step.blockers.is_empty()
                    && step.simulation_status == WalletSimulationStatus::Passed
            })
            .count(),
        value_items: steps
            .iter()
            .filter(|step| quantity_hex_is_nonzero(&step.amount_hex))
            .count(),
    }
}

pub(super) fn build_plan_steps(
    state: &WalletInventoryState,
    registry: &ProfileRegistry,
    body: &ConsolidationPlanGenerateRequest,
    destination_address: &Option<String>,
) -> Vec<ConsolidationPlanStep> {
    let mut steps = Vec::new();
    let per_party = body.routing_strategy.as_deref().map(str::trim) == Some("per_party");
    let address_to_party: BTreeMap<String, String> = if per_party {
        state
            .receive_allocations
            .iter()
            .filter_map(|allocation| {
                allocation.counterparty_id.as_ref().map(|counterparty_id| {
                    (
                        normalize_linkage_address(&allocation.address),
                        counterparty_id.clone(),
                    )
                })
            })
            .collect()
    } else {
        BTreeMap::new()
    };
    let party_to_destination: BTreeMap<String, String> = if per_party {
        body.party_destinations
            .iter()
            .filter_map(|destination| {
                let destination_address = destination.destination_address.trim();
                if destination_address.is_empty() {
                    None
                } else {
                    Some((
                        destination.counterparty_id.trim().to_string(),
                        destination_address.to_string(),
                    ))
                }
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    for holding in state
        .holdings
        .iter()
        .filter(|holding| quantity_hex_is_nonzero(&holding.amount_hex))
        .filter(|holding| {
            body.wallet_family
                .as_deref()
                .is_none_or(|family| family == holding.wallet_family)
        })
        .filter(|holding| {
            body.wallet_profile
                .as_deref()
                .is_none_or(|profile| profile == holding.wallet_profile)
        })
        .filter(|holding| {
            body.provider_profile
                .as_deref()
                .is_none_or(|profile| profile == holding.provider_profile)
        })
        .filter(|holding| {
            body.chain_id
                .is_none_or(|chain_id| chain_id == holding.chain_id)
        })
        .filter(|holding| !is_seed_control_reserve_holding(registry, holding))
    {
        let signer_status = signer_status_for_holding(holding);
        if signer_status == WalletSignerStatus::WatchOnly && body.include_watch_only != Some(true) {
            continue;
        }
        let (step_destination, missing_party_destination) = if per_party {
            let party = address_to_party.get(&normalize_linkage_address(&holding.address));
            if let Some(party_id) = party {
                match party_to_destination.get(party_id) {
                    Some(destination) => (Some(destination.clone()), false),
                    None => (None, true),
                }
            } else {
                (
                    resolve_default_destination(state, registry, holding, destination_address),
                    false,
                )
            }
        } else {
            (
                resolve_default_destination(state, registry, holding, destination_address),
                false,
            )
        };
        let mut step = plan_step_for_holding(holding, step_destination, signer_status);
        if missing_party_destination && step.action.as_str().starts_with("sweep") {
            if !step
                .blockers
                .iter()
                .any(|blocker| blocker == "missing_party_destination")
            {
                step.blockers.push("missing_party_destination".into());
            }
            step.status = WalletPlanStepStatus::Blocked;
            step.risk_level = "blocked".into();
        }
        steps.push(step);
    }

    steps
}

fn is_seed_control_reserve_holding(
    registry: &ProfileRegistry,
    holding: &WalletAssetHolding,
) -> bool {
    if holding.wallet_family != WALLET_FAMILY_ETH_SEED
        || holding.asset_kind != WalletAssetKind::Native
    {
        return false;
    }
    let Some(profile) = registry
        .eth_seed_wallets
        .iter()
        .find(|profile| profile.name == holding.wallet_profile)
    else {
        return false;
    };

    [
        profile.sponsor_address.as_deref(),
        profile.hot_address.as_deref(),
        profile.treasury_address.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|address| address.eq_ignore_ascii_case(&holding.address))
}

fn resolve_default_destination(
    state: &WalletInventoryState,
    registry: &ProfileRegistry,
    holding: &WalletAssetHolding,
    destination_address: &Option<String>,
) -> Option<String> {
    if destination_address.is_some() {
        destination_address.clone()
    } else if holding.wallet_family == WALLET_FAMILY_ETH_SEED {
        if let Some(seed_profile) = registry
            .eth_seed_wallets
            .iter()
            .find(|p| p.name == holding.wallet_profile)
        {
            if seed_profile.hot_address.is_some() && seed_profile.treasury_address.is_some() {
                let hot_addr = seed_profile.hot_address.as_ref().unwrap();
                let treasury_addr = seed_profile.treasury_address.as_ref().unwrap();
                let hot_balance = state
                    .addresses
                    .iter()
                    .find(|addr| {
                        addr.wallet_profile == holding.wallet_profile && addr.address == *hot_addr
                    })
                    .and_then(|addr| decode_quantity_hex(&addr.native_balance_wei_hex).ok())
                    .unwrap_or([0u8; 32]);
                let floor = state
                    .treasury_policy
                    .as_ref()
                    .and_then(|policy| decode_quantity_hex(&policy.hot_floor_wei_hex).ok())
                    .unwrap_or_else(|| decode_quantity_hex(DEFAULT_HOT_FLOOR_WEI_HEX).unwrap());
                // hot_target_wei_hex is the execution refill ceiling, not a
                // routing trigger: only balance below the floor routes hot.
                if compare_u256(&hot_balance, &floor).is_lt() {
                    Some(hot_addr.clone())
                } else {
                    Some(treasury_addr.clone())
                }
            } else {
                seed_profile.default_destination_address.clone()
            }
        } else {
            None
        }
    } else {
        None
    }
}

/// Extend a step with treasury policy blockers, mirroring planner semantics:
/// any blocker forces blocked status and blocked risk level. Blockers are
/// deduped because approval re-evaluates steps that generation already marked.
pub(in crate::service) fn apply_policy_blockers_to_step(
    policy: &TreasuryPolicy,
    step: &mut ConsolidationPlanStep,
) {
    let policy_blockers = policy_blockers_for_step(
        policy,
        step.action.as_str(),
        step.destination_address.as_deref(),
        step.asset_kind.as_str(),
        &step.amount_hex,
    );
    if policy_blockers.is_empty() {
        return;
    }
    for blocker in policy_blockers {
        if !step.blockers.contains(&blocker) {
            step.blockers.push(blocker);
        }
    }
    step.status = WalletPlanStepStatus::Blocked;
    step.risk_level = "blocked".into();
}

/// Fail-closed pass: convert every step that the linkage analyzer warned on
/// into a hard block, mirroring apply_policy_blockers_to_step semantics.
pub(in crate::service) fn apply_linkage_blockers(steps: &mut [ConsolidationPlanStep]) {
    for step in steps.iter_mut() {
        if step.linkage_warnings.is_empty() {
            continue;
        }
        if !step.blockers.iter().any(|b| b == "cross_party_linkage") {
            step.blockers.push("cross_party_linkage".into());
        }
        step.status = WalletPlanStepStatus::Blocked;
        step.risk_level = "blocked".into();
    }
}

/// Plan-level policy violations: currently only the native plan cap, summed
/// over steps that can still move value. Blocked steps cannot execute, so
/// they do not consume cap budget.
pub(in crate::service) fn plan_policy_violations(
    policy: &TreasuryPolicy,
    steps: &[ConsolidationPlanStep],
) -> Vec<String> {
    let mut violations = Vec::new();
    if !policy.enabled {
        return violations;
    }
    if let Some(cap_hex) = policy.max_plan_native_wei_hex.as_deref() {
        if let Ok(cap) = decode_quantity_hex(cap_hex) {
            let mut total = [0u8; 32];
            for step in steps.iter().filter(|step| {
                step.status != WalletPlanStepStatus::Blocked
                    && step.asset_kind == WalletAssetKind::Native
                    && step.action.as_str().starts_with("sweep")
            }) {
                total = add_u256(
                    &total,
                    &decode_quantity_hex(&step.amount_hex).unwrap_or([0u8; 32]),
                );
            }
            if compare_u256(&total, &cap).is_gt() {
                violations.push("exceeds_policy_plan_cap".into());
            }
        }
    }
    violations
}

#[derive(Clone, Debug)]
struct LinkageIdentity {
    key: String,
    label: String,
}

/// Detect common-recipient privacy linkage without changing plan execution.
pub(in crate::service) fn analyze_plan_linkage(
    state: &WalletInventoryState,
    steps: &mut [ConsolidationPlanStep],
) -> Vec<String> {
    let mut counterparty_by_address = BTreeMap::new();
    for allocation in &state.receive_allocations {
        if let Some(counterparty_id) = allocation.counterparty_id.as_deref() {
            counterparty_by_address.insert(
                normalize_linkage_address(&allocation.address),
                counterparty_id.to_string(),
            );
        }
    }

    let party_name_by_id: BTreeMap<String, String> = state
        .parties
        .iter()
        .map(|party| (party.id.clone(), party.name.clone()))
        .collect();

    let mut steps_by_destination: BTreeMap<String, Vec<(usize, LinkageIdentity)>> = BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        if !step.action.as_str().starts_with("sweep") {
            continue;
        }
        let Some(destination_address) = step.destination_address.as_deref() else {
            continue;
        };
        let destination_key = normalize_linkage_address(destination_address);
        if destination_key.is_empty() {
            continue;
        }
        steps_by_destination
            .entry(destination_key)
            .or_default()
            .push((
                index,
                linkage_identity_for_step(step, &counterparty_by_address, &party_name_by_id),
            ));
    }

    let mut findings = Vec::new();
    for (destination, entries) in steps_by_destination {
        let mut labels_by_identity = BTreeMap::new();
        for (_, identity) in &entries {
            labels_by_identity
                .entry(identity.key.clone())
                .or_insert_with(|| identity.label.clone());
        }
        if labels_by_identity.len() < 2 {
            continue;
        }

        let mut all_labels: Vec<String> = labels_by_identity.values().cloned().collect();
        all_labels.sort();
        findings.push(format!(
            "Destination {} links {} payers: {}",
            short_form(&destination, 10),
            labels_by_identity.len(),
            all_labels.join(", ")
        ));

        for (index, identity) in &entries {
            let mut others: Vec<String> = labels_by_identity
                .iter()
                .filter_map(|(key, label)| {
                    if key == &identity.key {
                        None
                    } else {
                        Some(label.clone())
                    }
                })
                .collect();
            others.sort();
            let warning = format!(
                "shared destination links this payer with: {}",
                others.join(", ")
            );
            if !steps[*index].linkage_warnings.contains(&warning) {
                steps[*index].linkage_warnings.push(warning);
            }
        }
    }

    let mut steps_by_funder: BTreeMap<String, Vec<(usize, LinkageIdentity)>> = BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        if step.action != WalletPlanStepAction::FundGas {
            continue;
        }
        let funder_key = normalize_linkage_address(&step.address);
        if funder_key.is_empty() {
            continue;
        }
        let Some(destination_address) = step.destination_address.as_deref() else {
            continue;
        };
        steps_by_funder.entry(funder_key).or_default().push((
            index,
            linkage_identity_for_address(
                destination_address,
                &counterparty_by_address,
                &party_name_by_id,
            ),
        ));
    }

    for (funder, entries) in steps_by_funder {
        let mut labels_by_identity = BTreeMap::new();
        for (_, identity) in &entries {
            labels_by_identity
                .entry(identity.key.clone())
                .or_insert_with(|| identity.label.clone());
        }
        if labels_by_identity.len() < 2 {
            continue;
        }

        let mut all_labels: Vec<String> = labels_by_identity.values().cloned().collect();
        all_labels.sort();
        findings.push(format!(
            "Sponsor {} funds {} parties: {}",
            short_form(&funder, 10),
            labels_by_identity.len(),
            all_labels.join(", ")
        ));

        for (index, identity) in &entries {
            let mut others: Vec<String> = labels_by_identity
                .iter()
                .filter_map(|(key, label)| {
                    if key == &identity.key {
                        None
                    } else {
                        Some(label.clone())
                    }
                })
                .collect();
            others.sort();
            let warning = format!(
                "shared gas sponsor links this party with: {}",
                others.join(", ")
            );
            if !steps[*index].linkage_warnings.contains(&warning) {
                steps[*index].linkage_warnings.push(warning);
            }
        }
    }

    findings
}

fn linkage_identity_for_step(
    step: &ConsolidationPlanStep,
    counterparty_by_address: &BTreeMap<String, String>,
    party_name_by_id: &BTreeMap<String, String>,
) -> LinkageIdentity {
    linkage_identity_for_address(&step.address, counterparty_by_address, party_name_by_id)
}

fn linkage_identity_for_address(
    address: &str,
    counterparty_by_address: &BTreeMap<String, String>,
    party_name_by_id: &BTreeMap<String, String>,
) -> LinkageIdentity {
    let address = normalize_linkage_address(address);
    if let Some(counterparty_id) = counterparty_by_address.get(&address) {
        return LinkageIdentity {
            key: format!("counterparty:{counterparty_id}"),
            label: party_name_by_id
                .get(counterparty_id)
                .cloned()
                .unwrap_or_else(|| short_form(counterparty_id, 12)),
        };
    }

    LinkageIdentity {
        key: format!("unattributed:{address}"),
        label: format!("unattributed ({})", short_form(&address, 10)),
    }
}

fn normalize_linkage_address(address: &str) -> String {
    address.trim().to_lowercase()
}

fn short_form(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    let mut chars = value.chars();
    let short: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{short}...")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sigillum_api::{
        Counterparty, EthSeedWalletProfile, PartyDestination, TreasuryReceiveAllocation,
        WalletInventoryAddress,
    };

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
            metadata_uri: None,
            metadata_name: None,
            spam_label: None,
            amount_hex: "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
            source: source.into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_receive_allocation(
        address: &str,
        counterparty_id: Option<&str>,
    ) -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: format!("alloc_{}", short_form(address, 6)),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/5".into(),
            address_index: 5,
            purpose: "counterparty".into(),
            label: None,
            status: "active".into(),
            created_at_unix: 1,
            retired_at_unix: None,
            counterparty_id: counterparty_id.map(str::to_string),
        }
    }

    fn sample_party(id: &str, name: &str) -> Counterparty {
        Counterparty {
            id: id.into(),
            name: name.into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 1,
        }
    }

    fn sample_sweep_step(address: &str, destination_address: &str) -> ConsolidationPlanStep {
        let mut holding = sample_holding("native", "native-balance", None);
        holding.address = address.into();
        holding.asset_address = None;
        holding.amount_hex = "0x1".into();
        plan_step_for_holding(&holding, Some(destination_address.into()), "available")
    }

    fn sample_fund_gas_step(
        id: &str,
        sponsor_address: &str,
        destination_address: &str,
    ) -> ConsolidationPlanStep {
        let mut step = sample_sweep_step(sponsor_address, destination_address);
        step.id = id.into();
        step.action = WalletPlanStepAction::FundGas;
        step.amount_hex = "0x100".into();
        step
    }

    #[test]
    fn normalize_linkage_address_folds_case_and_whitespace() {
        assert_eq!(
            normalize_linkage_address("  0xABCDef0000000000000000000000000000000001  "),
            "0xabcdef0000000000000000000000000000000001"
        );
        assert_eq!(
            normalize_linkage_address("  0xABCDef0000000000000000000000000000000001  "),
            normalize_linkage_address("0xABCDEF0000000000000000000000000000000001")
        );
    }

    #[test]
    fn planner_assign_step_ordering_sets_contiguous_sequence() {
        let destination = "0x9999999999999999999999999999999999999999";
        let mut steps = vec![
            sample_sweep_step("0x1111111111111111111111111111111111111111", destination),
            sample_sweep_step("0x2222222222222222222222222222222222222222", destination),
            sample_sweep_step("0x3333333333333333333333333333333333333333", destination),
        ];

        assign_step_ordering(&mut steps);

        assert_eq!(steps[0].sequence, 0);
        assert_eq!(steps[1].sequence, 1);
        assert_eq!(steps[2].sequence, 2);
        assert!(steps.iter().all(|step| step.depends_on.is_empty()));
    }

    fn sample_native_holding_at(address: &str) -> WalletAssetHolding {
        let mut holding = sample_holding("native", "native-balance", None);
        holding.address = address.into();
        holding.asset_address = None;
        holding.amount_hex = "0x1".into();
        holding
    }

    fn sample_plan_request(
        destination_address: Option<&str>,
        party_destinations: Vec<PartyDestination>,
    ) -> ConsolidationPlanGenerateRequest {
        ConsolidationPlanGenerateRequest {
            destination_address: destination_address.map(str::to_string),
            wallet_family: None,
            wallet_profile: None,
            provider_profile: None,
            chain_id: None,
            include_watch_only: None,
            auto_queue_low_risk: None,
            routing_strategy: Some("per_party".into()),
            party_destinations,
        }
    }

    fn sample_seed_registry() -> ProfileRegistry {
        ProfileRegistry {
            eth_seed_wallets: vec![EthSeedWalletProfile {
                name: "seed-main".into(),
                label: Some("Seed main".into()),
                project_account: 0,
                provider_profile: "mainnet".into(),
                compartment_id: 0,
                chain_id: Some(1),
                word_count: 12,
                mnemonic_secret_key: "wallet.seed.seed-main.mnemonic".into(),
                account_path: "m/44'/60'/0'".into(),
                receive_path: "m/44'/60'/0'/0".into(),
                receive_xpub: "xpub661MyMwAqRbcFexample".into(),
                first_receive_address: "0x1111111111111111111111111111111111111111".into(),
                default_destination_address: None,
                control_xpub: None,
                sponsor_address: None,
                hot_address: Some("0x2222222222222222222222222222222222222222".into()),
                treasury_address: Some("0x3333333333333333333333333333333333333333".into()),
                execution_enabled: false,
            }],
            ..ProfileRegistry::default()
        }
    }

    fn sample_hot_address(balance_hex: &str) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: "addr_hot".into(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x2222222222222222222222222222222222222222".into(),
            derivation_path: "m/44'/60'/0'/0/1".into(),
            derivation_pattern: Some("hot".into()),
            account_index: Some(0),
            address_index: 1,
            activity_state: "funded".into(),
            native_balance_wei_hex: balance_hex.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "test".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_routing_policy(floor_hex: &str, target_hex: &str) -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            allow_claim_execution: false,
            allow_gas_topups: false,
            max_gas_topup_wei_hex: None,
            allow_plan_execution: false,
            allow_sweep_execution: false,
            allow_revoke_execution: false,
            allow_exit_execution: false,
            execution_paused: false,
            max_fee_per_gas_cap_hex: None,
            simulation_freshness_secs: 900,
            hot_floor_wei_hex: floor_hex.into(),
            hot_target_wei_hex: target_hex.into(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    fn default_destination_for_hot_balance(
        hot_balance_hex: &str,
        treasury_policy: Option<TreasuryPolicy>,
    ) -> Option<String> {
        let state = WalletInventoryState {
            addresses: vec![sample_hot_address(hot_balance_hex)],
            treasury_policy,
            ..WalletInventoryState::default()
        };
        let registry = sample_seed_registry();
        let holding = sample_native_holding_at("0x1111111111111111111111111111111111111111");
        resolve_default_destination(&state, &registry, &holding, &None)
    }

    #[test]
    fn build_plan_steps_skips_seed_control_reserve_native_holdings() {
        let mut registry = sample_seed_registry();
        registry.eth_seed_wallets[0].sponsor_address =
            Some("0x4444444444444444444444444444444444444444".into());
        let state = WalletInventoryState {
            holdings: vec![
                sample_native_holding_at("0x1111111111111111111111111111111111111111"),
                sample_native_holding_at("0x2222222222222222222222222222222222222222"),
                sample_native_holding_at("0x3333333333333333333333333333333333333333"),
                sample_native_holding_at("0x4444444444444444444444444444444444444444"),
            ],
            ..WalletInventoryState::default()
        };
        let body = sample_plan_request(
            Some("0x5555555555555555555555555555555555555555"),
            Vec::new(),
        );

        let steps = build_plan_steps(
            &state,
            &registry,
            &body,
            &Some("0x5555555555555555555555555555555555555555".into()),
        );

        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].address,
            "0x1111111111111111111111111111111111111111"
        );
    }

    #[test]
    fn legacy_default_routing_is_byte_identical_to_one_eth_hardcode() {
        let cases = [
            (
                "0xde0b6b3a763ffff",
                "0x2222222222222222222222222222222222222222",
            ),
            (
                DEFAULT_HOT_FLOOR_WEI_HEX,
                "0x3333333333333333333333333333333333333333",
            ),
            (
                "0xde0b6b3a7640001",
                "0x3333333333333333333333333333333333333333",
            ),
        ];

        for (hot_balance, expected_destination) in cases {
            assert_eq!(
                default_destination_for_hot_balance(hot_balance, None).as_deref(),
                Some(expected_destination)
            );
            assert_eq!(
                default_destination_for_hot_balance(
                    hot_balance,
                    Some(sample_routing_policy(
                        DEFAULT_HOT_FLOOR_WEI_HEX,
                        DEFAULT_HOT_FLOOR_WEI_HEX,
                    )),
                )
                .as_deref(),
                Some(expected_destination)
            );
        }
    }

    #[test]
    fn policy_floor_overrides_default() {
        let policy = sample_routing_policy("0x2", "0x2");

        assert_eq!(
            default_destination_for_hot_balance("0x1", Some(policy.clone())).as_deref(),
            Some("0x2222222222222222222222222222222222222222")
        );
        assert_eq!(
            default_destination_for_hot_balance("0x2", Some(policy)).as_deref(),
            Some("0x3333333333333333333333333333333333333333")
        );
    }

    #[test]
    fn target_does_not_trigger_refill() {
        let policy = sample_routing_policy("0x1", DEFAULT_HOT_FLOOR_WEI_HEX);

        assert_eq!(
            default_destination_for_hot_balance("0x1", Some(policy)).as_deref(),
            Some("0x3333333333333333333333333333333333333333")
        );
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
    fn eligible_merkle_claim_without_policy_optin_keeps_exact_execution_blocker() {
        let mut holding = sample_holding("airdrop", "claim-candidate:airdrop:op:list", None);
        holding.protocol_address = Some("0x1111111111111111111111111111111111111111".into());
        holding.claim_adapter = Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into());
        holding.claim_index_hex = Some("0x7".into());
        holding.claim_proof = vec![
            format!("0x{}", "11".repeat(32)),
            format!("0x{}", "22".repeat(32)),
        ];

        let step = plan_step_for_holding(&holding, None, "available");

        assert_eq!(step.blockers, vec!["claim_execution_disabled".to_string()]);
        assert_eq!(step.status, WalletPlanStepStatus::Blocked);
        assert_eq!(step.simulation_status, WalletSimulationStatus::Required);
        assert_eq!(step.risk_level, "blocked");
        assert!(!step.approved);
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

    #[test]
    fn build_plan_steps_per_party_routes_party_with_mapping_to_its_destination() {
        let source_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let destination = "0xdddddddddddddddddddddddddddddddddddddddd";
        let state = WalletInventoryState {
            holdings: vec![sample_native_holding_at(source_address)],
            receive_allocations: vec![sample_receive_allocation(
                source_address,
                Some("party_acme"),
            )],
            parties: vec![sample_party("party_acme", "Acme")],
            ..Default::default()
        };
        let body = sample_plan_request(
            None,
            vec![PartyDestination {
                counterparty_id: "party_acme".into(),
                destination_address: destination.into(),
            }],
        );

        let steps = build_plan_steps(&state, &ProfileRegistry::default(), &body, &None);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, "sweep_native");
        assert_eq!(steps[0].destination_address.as_deref(), Some(destination));
        assert_eq!(steps[0].status, "review_required");
        assert!(steps[0].blockers.is_empty());
    }

    #[test]
    fn build_plan_steps_per_party_blocks_party_without_mapping() {
        let source_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let state = WalletInventoryState {
            holdings: vec![sample_native_holding_at(source_address)],
            receive_allocations: vec![sample_receive_allocation(
                source_address,
                Some("party_acme"),
            )],
            parties: vec![sample_party("party_acme", "Acme")],
            ..Default::default()
        };
        let body = sample_plan_request(None, Vec::new());

        let steps = build_plan_steps(&state, &ProfileRegistry::default(), &body, &None);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].action, "sweep_native");
        assert_eq!(steps[0].destination_address, None);
        assert!(steps[0].blockers.contains(&"missing_destination".into()));
        assert!(
            steps[0]
                .blockers
                .contains(&"missing_party_destination".into())
        );
        assert_eq!(steps[0].status, "blocked");
        assert_eq!(steps[0].risk_level, "blocked");
    }

    #[test]
    fn build_plan_steps_per_party_unattributed_uses_global_destination() {
        let source_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState {
            holdings: vec![sample_native_holding_at(source_address)],
            receive_allocations: Vec::new(),
            ..Default::default()
        };
        let body = sample_plan_request(Some(destination), Vec::new());
        let destination_address = body.destination_address.clone();

        let steps = build_plan_steps(
            &state,
            &ProfileRegistry::default(),
            &body,
            &destination_address,
        );

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].destination_address.as_deref(), Some(destination));
        assert_eq!(steps[0].status, "review_required");
    }

    #[test]
    fn per_party_distinct_destinations_have_no_linkage_findings() {
        let acme_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let acme_destination = "0x1111111111111111111111111111111111111111";
        let bob_destination = "0x2222222222222222222222222222222222222222";
        let state = WalletInventoryState {
            holdings: vec![
                sample_native_holding_at(acme_address),
                sample_native_holding_at(bob_address),
            ],
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            ..Default::default()
        };
        let body = sample_plan_request(
            None,
            vec![
                PartyDestination {
                    counterparty_id: "party_acme".into(),
                    destination_address: acme_destination.into(),
                },
                PartyDestination {
                    counterparty_id: "party_bob".into(),
                    destination_address: bob_destination.into(),
                },
            ],
        );
        let mut steps = build_plan_steps(&state, &ProfileRegistry::default(), &body, &None);

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert!(findings.is_empty());
        assert!(steps.iter().all(|step| step.linkage_warnings.is_empty()));
    }

    #[test]
    fn per_party_same_destination_for_two_parties_yields_linkage_findings() {
        let acme_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState {
            holdings: vec![
                sample_native_holding_at(acme_address),
                sample_native_holding_at(bob_address),
            ],
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            ..Default::default()
        };
        let body = sample_plan_request(
            None,
            vec![
                PartyDestination {
                    counterparty_id: "party_acme".into(),
                    destination_address: destination.into(),
                },
                PartyDestination {
                    counterparty_id: "party_bob".into(),
                    destination_address: destination.into(),
                },
            ],
        );
        let mut steps = build_plan_steps(&state, &ProfileRegistry::default(), &body, &None);

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert!(!findings.is_empty());
        assert!(steps.iter().all(|step| !step.linkage_warnings.is_empty()));
    }

    #[test]
    fn apply_linkage_blockers_hard_blocks_warned_steps() {
        let acme_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let unrelated_address = "0xcccccccccccccccccccccccccccccccccccccccc";
        let shared_destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState {
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_sweep_step(acme_address, shared_destination),
            sample_sweep_step(bob_address, shared_destination),
            sample_sweep_step(
                unrelated_address,
                "0x8888888888888888888888888888888888888888",
            ),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);
        apply_linkage_blockers(&mut steps);

        assert!(!findings.is_empty());
        for step in &steps[..2] {
            assert!(step.blockers.contains(&"cross_party_linkage".into()));
            assert_eq!(step.status, "blocked");
            assert_eq!(step.risk_level, "blocked");
        }
        assert!(steps[2].linkage_warnings.is_empty());
        assert!(!steps[2].blockers.contains(&"cross_party_linkage".into()));
        assert_eq!(steps[2].status, "review_required");
        assert_eq!(steps[2].risk_level, "low");
    }

    #[test]
    fn analyze_plan_linkage_warns_for_two_distinct_parties_to_one_destination() {
        let acme_address = "0xaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaA";
        let bob_address = "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState {
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_sweep_step(acme_address, destination),
            sample_sweep_step(bob_address, destination),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("Destination 0x99999999... links 2 payers"));
        assert!(findings[0].contains("Acme"));
        assert!(findings[0].contains("Bob"));
        assert_eq!(steps[0].linkage_warnings.len(), 1);
        assert!(steps[0].linkage_warnings[0].contains("Bob"));
        assert!(!steps[0].linkage_warnings[0].contains("Acme"));
        assert_eq!(steps[1].linkage_warnings.len(), 1);
        assert!(steps[1].linkage_warnings[0].contains("Acme"));
        assert!(!steps[1].linkage_warnings[0].contains("Bob"));
    }

    #[test]
    fn analyze_plan_linkage_clusters_case_variant_destinations() {
        let acme_address = "0xaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaA";
        let bob_address = "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB";
        let destination_lower = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
        let destination_upper = "0xABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";
        let state = WalletInventoryState {
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_sweep_step(acme_address, destination_lower),
            sample_sweep_step(bob_address, destination_upper),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("Destination 0xabcdefab... links 2 payers"));
        assert!(findings[0].contains("Acme"));
        assert!(findings[0].contains("Bob"));
        assert_eq!(steps[0].linkage_warnings.len(), 1);
        assert!(steps[0].linkage_warnings[0].contains("Bob"));
        assert!(!steps[0].linkage_warnings[0].contains("Acme"));
        assert_eq!(steps[1].linkage_warnings.len(), 1);
        assert!(steps[1].linkage_warnings[0].contains("Acme"));
        assert!(!steps[1].linkage_warnings[0].contains("Bob"));
    }

    #[test]
    fn analyze_plan_linkage_does_not_warn_for_one_party_with_multiple_sources() {
        let first_address = "0x1111111111111111111111111111111111111111";
        let second_address = "0x2222222222222222222222222222222222222222";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState {
            parties: vec![sample_party("party_acme", "Acme")],
            receive_allocations: vec![
                sample_receive_allocation(first_address, Some("party_acme")),
                sample_receive_allocation(second_address, Some("party_acme")),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_sweep_step(first_address, destination),
            sample_sweep_step(second_address, destination),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert!(findings.is_empty());
        assert!(steps.iter().all(|step| step.linkage_warnings.is_empty()));
    }

    #[test]
    fn analyze_plan_linkage_treats_unattributed_addresses_as_distinct_identities() {
        let first_address = "0x3333333333333333333333333333333333333333";
        let second_address = "0x4444444444444444444444444444444444444444";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState::default();
        let mut steps = vec![
            sample_sweep_step(first_address, destination),
            sample_sweep_step(second_address, destination),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("links 2 payers"));
        assert!(findings[0].contains("unattributed (0x33333333...)"));
        assert!(findings[0].contains("unattributed (0x44444444...)"));
        assert_eq!(steps[0].linkage_warnings.len(), 1);
        assert!(steps[0].linkage_warnings[0].contains("0x44444444..."));
        assert_eq!(steps[1].linkage_warnings.len(), 1);
        assert!(steps[1].linkage_warnings[0].contains("0x33333333..."));
    }

    #[test]
    fn analyze_plan_linkage_does_not_warn_for_same_unattributed_source_twice() {
        let address = "0x5555555555555555555555555555555555555555";
        let destination = "0x9999999999999999999999999999999999999999";
        let state = WalletInventoryState::default();
        let mut steps = vec![
            sample_sweep_step(address, destination),
            sample_sweep_step(address, destination),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert!(findings.is_empty());
        assert!(steps.iter().all(|step| step.linkage_warnings.is_empty()));
    }

    #[test]
    fn fund_gas_cross_party_sponsor_funding_warns_both_steps() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let acme_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let state = WalletInventoryState {
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_fund_gas_step("fund_acme", sponsor, acme_address),
            sample_fund_gas_step("fund_bob", sponsor, bob_address),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("Sponsor"));
        assert!(findings[0].contains("funds 2 parties"));
        assert!(findings[0].contains("Acme"));
        assert!(findings[0].contains("Bob"));
        assert_eq!(steps[0].linkage_warnings.len(), 1);
        assert!(steps[0].linkage_warnings[0].contains("Bob"));
        assert!(!steps[0].linkage_warnings[0].contains("Acme"));
        assert_eq!(steps[1].linkage_warnings.len(), 1);
        assert!(steps[1].linkage_warnings[0].contains("Acme"));
        assert!(!steps[1].linkage_warnings[0].contains("Bob"));
    }

    #[test]
    fn fund_gas_same_party_topups_do_not_warn() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let first_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let state = WalletInventoryState {
            receive_allocations: vec![
                sample_receive_allocation(first_address, Some("party_acme")),
                sample_receive_allocation(second_address, Some("party_acme")),
            ],
            parties: vec![sample_party("party_acme", "Acme")],
            ..Default::default()
        };
        let mut steps = vec![
            sample_fund_gas_step("fund_1", sponsor, first_address),
            sample_fund_gas_step("fund_2", sponsor, second_address),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert!(findings.is_empty());
        assert!(steps.iter().all(|step| step.linkage_warnings.is_empty()));
    }

    #[test]
    fn fund_gas_unattributed_destinations_are_distinct_identities() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let first_address = "0x1111111111111111111111111111111111111111";
        let second_address = "0x2222222222222222222222222222222222222222";
        let state = WalletInventoryState::default();
        let mut steps = vec![
            sample_fund_gas_step("fund_1", sponsor, first_address),
            sample_fund_gas_step("fund_2", sponsor, second_address),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("funds 2 parties"));
        assert!(findings[0].contains("unattributed (0x11111111...)"));
        assert!(findings[0].contains("unattributed (0x22222222...)"));
        assert_eq!(steps[0].linkage_warnings.len(), 1);
        assert!(steps[0].linkage_warnings[0].contains("0x22222222..."));
        assert_eq!(steps[1].linkage_warnings.len(), 1);
        assert!(steps[1].linkage_warnings[0].contains("0x11111111..."));
    }

    #[test]
    fn fund_gas_linkage_hard_blocks_when_policy_blocks_cross_party() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let acme_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let bob_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let state = WalletInventoryState {
            receive_allocations: vec![
                sample_receive_allocation(acme_address, Some("party_acme")),
                sample_receive_allocation(bob_address, Some("party_bob")),
            ],
            parties: vec![
                sample_party("party_acme", "Acme"),
                sample_party("party_bob", "Bob"),
            ],
            ..Default::default()
        };
        let mut steps = vec![
            sample_fund_gas_step("fund_acme", sponsor, acme_address),
            sample_fund_gas_step("fund_bob", sponsor, bob_address),
            sample_sweep_step(
                "0xcccccccccccccccccccccccccccccccccccccccc",
                "0x9999999999999999999999999999999999999999",
            ),
        ];

        let findings = analyze_plan_linkage(&state, &mut steps);
        apply_linkage_blockers(&mut steps);

        assert_eq!(findings.len(), 1);
        for step in &steps[..2] {
            assert!(step.blockers.contains(&"cross_party_linkage".into()));
            assert_eq!(step.status, WalletPlanStepStatus::Blocked);
            assert_eq!(step.risk_level, "blocked");
        }
        assert!(steps[2].linkage_warnings.is_empty());
        assert!(!steps[2].blockers.contains(&"cross_party_linkage".into()));
        assert_eq!(steps[2].status, WalletPlanStepStatus::ReviewRequired);
    }
}
