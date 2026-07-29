use std::collections::{BTreeMap, BTreeSet};

use sigillum_api::{
    TreasuryAutomationStatus, TreasuryChainSummary, TreasuryGroupSummary, TreasuryOverviewResponse,
    TreasuryPlanSummary, TreasuryReceiveAllocation, TreasuryReceiveSummary, TreasuryRiskSummary,
    TreasuryRoutingStatus, WalletAddressClassification, WalletAssetHolding, WalletAssetKind,
    WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::inventory::WalletInventoryState;
use crate::service::helpers::{add_u256, encode_quantity_hex, now_unix};
use crate::service::{ServiceResult, SigillumService};

use super::super::risk::derive_inventory_risk_findings;
use super::super::support::load_inventory_state;
use super::super::{WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH, WALLET_FAMILY_ETH_XPUB};
use super::allocations::RECEIVE_STATUS_ACTIVE;

const DEFAULT_NATIVE_SYMBOL: &str = "ETH";

pub(super) fn decoded_balance(hex: &str) -> [u8; 32] {
    decode_quantity_hex(hex).unwrap_or([0u8; 32])
}

fn balance_is_nonzero(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn has_classification(
    address: &WalletInventoryAddress,
    classification: &WalletAddressClassification,
) -> bool {
    address
        .classifications
        .iter()
        .any(|entry| entry == classification)
}

#[derive(Default)]
struct ChainAccumulator {
    address_count: usize,
    funded_address_count: usize,
    native_total: [u8; 32],
}

#[derive(Default)]
struct GroupAccumulator {
    address_count: usize,
    funded_address_count: usize,
    native_total: [u8; 32],
    signer_address_count: usize,
    watch_only_address_count: usize,
    erc20_holding_count: usize,
    nft_holding_count: usize,
    defi_holding_count: usize,
    claimable_holding_count: usize,
    approval_exposure_count: usize,
    dormant_candidate_count: usize,
}

fn classify_holding(group: &mut GroupAccumulator, holding: &WalletAssetHolding) {
    match &holding.asset_kind {
        WalletAssetKind::Erc20 => group.erc20_holding_count += 1,
        WalletAssetKind::Erc721 | WalletAssetKind::Erc1155 | WalletAssetKind::Nft => {
            group.nft_holding_count += 1;
        }
        WalletAssetKind::Defi => group.defi_holding_count += 1,
        WalletAssetKind::Airdrop | WalletAssetKind::Reward => group.claimable_holding_count += 1,
        WalletAssetKind::Approval => group.approval_exposure_count += 1,
        _ => {}
    }
}

impl SigillumService {
    pub(crate) fn treasury_overview(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryOverviewResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            crate::service::ServiceError::internal(format!("Failed to load profiles: {error}"))
        })?;

        let mut chains: BTreeMap<u64, ChainAccumulator> = BTreeMap::new();
        let mut groups: BTreeMap<(String, String, u64), GroupAccumulator> = BTreeMap::new();
        let mut tracked_address_count = 0usize;
        let mut funded_address_count = 0usize;
        let mut watch_only_address_count = 0usize;
        let mut signer_address_count = 0usize;

        for address in &state.addresses {
            let balance = decoded_balance(&address.native_balance_wei_hex);
            let funded = balance_is_nonzero(&balance);
            tracked_address_count += 1;
            if funded {
                funded_address_count += 1;
            }
            let watch_only =
                matches!(
                    address.wallet_family.as_str(),
                    WALLET_FAMILY_ETH_XPUB | WALLET_FAMILY_ETH_WATCH
                ) || has_classification(address, &WalletAddressClassification::WatchOnly);
            if watch_only {
                watch_only_address_count += 1;
            }
            if address.wallet_family == WALLET_FAMILY_ETH_SEED
                || has_classification(address, &WalletAddressClassification::SignerAvailable)
            {
                signer_address_count += 1;
            }

            let chain = chains.entry(address.chain_id).or_default();
            chain.address_count += 1;
            if funded {
                chain.funded_address_count += 1;
            }
            chain.native_total = add_u256(&chain.native_total, &balance);

            let group = groups
                .entry((
                    address.wallet_family.clone(),
                    address.wallet_profile.clone(),
                    address.chain_id,
                ))
                .or_default();
            group.address_count += 1;
            if funded {
                group.funded_address_count += 1;
            }
            group.native_total = add_u256(&group.native_total, &balance);
            if watch_only {
                group.watch_only_address_count += 1;
            } else if address.wallet_family == WALLET_FAMILY_ETH_SEED
                || has_classification(address, &WalletAddressClassification::SignerAvailable)
            {
                group.signer_address_count += 1;
            }
            if has_classification(address, &WalletAddressClassification::DormantCandidate) {
                group.dormant_candidate_count += 1;
            }
        }

        for holding in &state.holdings {
            let group = groups
                .entry((
                    holding.wallet_family.clone(),
                    holding.wallet_profile.clone(),
                    holding.chain_id,
                ))
                .or_default();
            classify_holding(group, holding);
        }

        let native_symbol_for_chain = |chain_id: u64| -> String {
            state
                .chain_profiles
                .iter()
                .find(|profile| profile.chain_id == Some(chain_id))
                .map(|profile| profile.native_symbol.clone())
                .unwrap_or_else(|| DEFAULT_NATIVE_SYMBOL.to_string())
        };

        let chains = chains
            .into_iter()
            .map(|(chain_id, accumulator)| TreasuryChainSummary {
                chain_id,
                native_symbol: native_symbol_for_chain(chain_id),
                address_count: accumulator.address_count,
                funded_address_count: accumulator.funded_address_count,
                native_total_wei_hex: encode_quantity_hex(&accumulator.native_total),
            })
            .collect();

        let groups = groups
            .into_iter()
            .map(
                |((wallet_family, wallet_profile, chain_id), accumulator)| TreasuryGroupSummary {
                    wallet_family,
                    wallet_profile,
                    chain_id,
                    address_count: accumulator.address_count,
                    funded_address_count: accumulator.funded_address_count,
                    native_total_wei_hex: encode_quantity_hex(&accumulator.native_total),
                    signer_address_count: accumulator.signer_address_count,
                    watch_only_address_count: accumulator.watch_only_address_count,
                    erc20_holding_count: accumulator.erc20_holding_count,
                    nft_holding_count: accumulator.nft_holding_count,
                    defi_holding_count: accumulator.defi_holding_count,
                    claimable_holding_count: accumulator.claimable_holding_count,
                    approval_exposure_count: accumulator.approval_exposure_count,
                    dormant_candidate_count: accumulator.dormant_candidate_count,
                },
            )
            .collect();

        let balance_for_address = |target: &str| -> Option<String> {
            let target = target.to_ascii_lowercase();
            let mut total = [0u8; 32];
            let mut seen = false;
            for address in &state.addresses {
                if address.address.to_ascii_lowercase() == target {
                    total = add_u256(&total, &decoded_balance(&address.native_balance_wei_hex));
                    seen = true;
                }
            }
            seen.then(|| encode_quantity_hex(&total))
        };

        let routing = registry
            .eth_seed_wallets
            .iter()
            .map(|profile| TreasuryRoutingStatus {
                wallet_profile: profile.name.clone(),
                hot_address: profile.hot_address.clone(),
                treasury_address: profile.treasury_address.clone(),
                default_destination_address: profile.default_destination_address.clone(),
                hot_native_balance_wei_hex: profile
                    .hot_address
                    .as_deref()
                    .and_then(balance_for_address),
                treasury_native_balance_wei_hex: profile
                    .treasury_address
                    .as_deref()
                    .and_then(balance_for_address),
                routing_ready: profile.treasury_address.is_some()
                    || profile.default_destination_address.is_some(),
            })
            .collect();

        let mut risk = TreasuryRiskSummary::default();
        let mut findings = state.risk_findings.clone();
        findings.extend(derive_inventory_risk_findings(
            &state.addresses,
            &state.holdings,
            &state.risk_catalog,
            &state.chain_profiles,
        ));
        for finding in &findings {
            risk.total_findings += 1;
            match finding.risk_level.as_str() {
                "critical" => risk.critical_findings += 1,
                "high" => risk.high_findings += 1,
                "medium" => risk.medium_findings += 1,
                _ => risk.low_findings += 1,
            }
        }

        let latest_plan = state
            .consolidation_plans
            .iter()
            .max_by_key(|plan| plan.created_at_unix);
        let plans = TreasuryPlanSummary {
            total_plans: state.consolidation_plans.len(),
            latest_plan_id: latest_plan.map(|plan| plan.id.clone()),
            latest_plan_status: latest_plan.map(|plan| plan.status.to_string()),
            latest_review_required_steps: latest_plan
                .map(|plan| plan.summary.review_required_steps)
                .unwrap_or(0),
            latest_approved_steps: latest_plan
                .map(|plan| plan.summary.approved_steps)
                .unwrap_or(0),
            latest_executable_steps: latest_plan
                .map(|plan| plan.summary.executable_steps)
                .unwrap_or(0),
            latest_blocked_steps: latest_plan
                .map(|plan| plan.summary.blocked_steps)
                .unwrap_or(0),
            latest_policy_violations: latest_plan
                .map(|plan| plan.policy_violations.clone())
                .unwrap_or_default(),
        };

        let receive = receive_summary(&state.receive_allocations);
        let automation = treasury_automation_status(&state);

        Ok(TreasuryOverviewResponse {
            generated_at_unix: now_unix(),
            tracked_address_count,
            funded_address_count,
            watch_only_address_count,
            signer_address_count,
            chains,
            groups,
            routing,
            risk,
            plans,
            receive,
            automation,
        })
    }
}

fn treasury_automation_status(state: &WalletInventoryState) -> TreasuryAutomationStatus {
    let mut generated_steps = 0usize;
    let mut enqueued_steps = 0usize;
    for plan in state
        .consolidation_plans
        .iter()
        .filter(|plan| plan.origin.as_deref() == Some("treasury_automation"))
    {
        generated_steps += plan.steps.len();
        enqueued_steps += plan
            .steps
            .iter()
            .filter(|step| step.queued_job_id.is_some())
            .count();
    }
    TreasuryAutomationStatus {
        enabled: state
            .treasury_policy
            .as_ref()
            .map(|policy| policy.enabled && policy.allow_treasury_automation)
            .unwrap_or(false),
        hot_overflow_wei_hex: state
            .treasury_policy
            .as_ref()
            .and_then(|policy| policy.hot_overflow_wei_hex.clone()),
        generated_steps,
        enqueued_steps,
    }
}

/// Console rollup of allocation counts and distinct active purposes.
pub(super) fn receive_summary(allocations: &[TreasuryReceiveAllocation]) -> TreasuryReceiveSummary {
    let mut summary = TreasuryReceiveSummary::default();
    let mut purposes = BTreeSet::new();
    for allocation in allocations {
        if allocation.status == RECEIVE_STATUS_ACTIVE {
            summary.active_allocations += 1;
            purposes.insert(allocation.purpose.as_str());
        } else {
            summary.retired_allocations += 1;
        }
    }
    summary.purposes = purposes.len();
    summary
}
