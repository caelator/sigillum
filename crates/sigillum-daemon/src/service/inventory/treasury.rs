//! Treasury console aggregation over inventory, routing, risk, and planning,
//! plus purpose-labeled receive-address allocation.

use std::collections::{BTreeMap, BTreeSet};

use sigillum_api::{
    Counterparty, CounterpartyCreateRequest, CounterpartyDeleteRequest, CounterpartyListResponse,
    CounterpartyMutationResponse, CounterpartyUpdateRequest, EthStealthDeposit,
    EthStealthDepositRefreshRequest, EvmProviderProfile, ReceivingCoverage, ReceivingItem,
    ReceivingOverviewResponse, ReceivingPartyGroup, ReceivingRefreshResponse, ReceivingTotals,
    TreasuryAllowedDestination, TreasuryAutomationStatus, TreasuryChainSummary,
    TreasuryGroupSummary, TreasuryOverviewResponse, TreasuryPlanSummary, TreasuryPolicy,
    TreasuryPolicyMutationResponse, TreasuryPolicyResponse, TreasuryPolicyUpdateRequest,
    TreasuryReceiveAllocateRequest, TreasuryReceiveAllocation,
    TreasuryReceiveAllocationListResponse, TreasuryReceiveAllocationMutationResponse,
    TreasuryReceiveRotateRequest, TreasuryReceiveSummary, TreasuryRiskSummary,
    TreasuryRoutingStatus, WalletAddressClassification, WalletAssetHolding, WalletAssetKind,
    WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::deposits::DepositState;
use crate::inventory::WalletInventoryState;
use crate::service::evm::normalize_address;
use crate::service::helpers::{
    compare_u256, map_xpub_error, now_unix, random_id, session_fingerprint_hex,
};
use crate::service::transaction_policy::{
    TransactionPolicyCheck, TransactionPolicyKind, transaction_policy_actions,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::risk::derive_inventory_risk_findings;
use super::support::{
    load_inventory_state, quantity_hex_is_nonzero, save_inventory_state, select_providers,
    trimmed_optional, trimmed_required, upsert_address,
};
use super::wallet_selection::{
    SeedDerivationPattern, derive_discovery_wallet_address, select_discovery_wallets,
};
use super::{
    DISCOVERY_SOURCE_LOCAL_RPC, WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH,
    WALLET_FAMILY_ETH_XPUB,
};

const DEFAULT_NATIVE_SYMBOL: &str = "ETH";
const RECEIVE_STATUS_ACTIVE: &str = "active";
const RECEIVE_STATUS_RETIRED: &str = "retired";
const RECEIVING_LINKAGE_WARNING: &str = "Sweeping here would link this payer with another party. Set a distinct per-party sweep destination.";
const DEFAULT_HOT_FLOOR_WEI_HEX: &str = "0xde0b6b3a7640000";
const DEFAULT_HOT_TARGET_WEI_HEX: &str = "0xde0b6b3a7640000";
/// Absurdly large but bounded: receive indices beyond this point indicate a
/// runaway caller, not a treasury that genuinely needs a million addresses.
const MAX_RECEIVE_INDEX: u32 = 1_000_000;

/// Saturating big-endian addition of two 256-bit quantities.
pub(in crate::service) fn add_u256(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for index in (0..32).rev() {
        let sum = left[index] as u16 + right[index] as u16 + carry;
        out[index] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    if carry > 0 {
        // Saturate rather than wrap: an overflowing treasury total is already
        // far outside plausible balances, and wrapping would understate value.
        return [0xff; 32];
    }
    out
}

pub(super) fn encode_quantity_hex(value: &[u8; 32]) -> String {
    let first_nonzero = value.iter().position(|byte| *byte != 0);
    match first_nonzero {
        None => "0x0".to_string(),
        Some(start) => {
            let mut encoded = String::with_capacity(2 + (32 - start) * 2);
            encoded.push_str("0x");
            let mut rendered = false;
            for byte in &value[start..] {
                if rendered {
                    encoded.push_str(&format!("{byte:02x}"));
                } else {
                    encoded.push_str(&format!("{byte:x}"));
                    rendered = true;
                }
            }
            encoded
        }
    }
}

fn decoded_balance(hex: &str) -> [u8; 32] {
    decode_quantity_hex(hex).unwrap_or([0u8; 32])
}

fn balance_is_nonzero(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn usize_to_u32_count(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
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

fn build_receiving_overview(
    state: &WalletInventoryState,
    deposits: &DepositState,
    now: u64,
) -> ReceivingOverviewResponse {
    let mut balances_by_address: BTreeMap<String, [u8; 32]> = BTreeMap::new();
    for address in &state.addresses {
        let balance = decode_quantity_hex(&address.native_balance_wei_hex).unwrap_or([0u8; 32]);
        let total = balances_by_address
            .entry(address.address.to_ascii_lowercase())
            .or_insert([0u8; 32]);
        let current = *total;
        *total = add_u256(&current, &balance);
    }

    let known_party_ids: BTreeSet<String> =
        state.parties.iter().map(|party| party.id.clone()).collect();
    let mut items_by_party_id: BTreeMap<String, Vec<ReceivingItem>> = BTreeMap::new();
    let mut unassigned_items: Vec<ReceivingItem> = Vec::new();
    let mut hd_count = 0u32;

    for allocation in state
        .receive_allocations
        .iter()
        .filter(|allocation| allocation.status == RECEIVE_STATUS_ACTIVE)
    {
        hd_count += 1;
        let item = hd_receiving_item(allocation, &balances_by_address);
        let resolved_counterparty_id = item
            .counterparty_id
            .as_ref()
            .filter(|counterparty_id| known_party_ids.contains(*counterparty_id))
            .cloned();
        if let Some(counterparty_id) = resolved_counterparty_id {
            items_by_party_id
                .entry(counterparty_id)
                .or_default()
                .push(item);
        } else {
            unassigned_items.push(item);
        }
    }

    let stealth_linkage_warnings =
        receiving_linkage_warning_deposit_ids(&deposits.eth_stealth, &state.parties);
    let mut stealth_count = 0u32;
    for deposit in &deposits.eth_stealth {
        stealth_count += 1;
        let mut item = stealth_receiving_item(deposit);
        if stealth_linkage_warnings.contains(&deposit.id) {
            item.linkage_warning = Some(RECEIVING_LINKAGE_WARNING.into());
        }
        let resolved_counterparty_id = item
            .counterparty_id
            .as_ref()
            .filter(|counterparty_id| known_party_ids.contains(*counterparty_id))
            .cloned();
        if let Some(counterparty_id) = resolved_counterparty_id {
            items_by_party_id
                .entry(counterparty_id)
                .or_default()
                .push(item);
        } else {
            unassigned_items.push(item);
        }
    }

    let mut groups = Vec::new();
    for party in &state.parties {
        if let Some(items) = items_by_party_id.remove(&party.id) {
            groups.push(receiving_party_group(Some(party.clone()), items));
        }
    }
    if !unassigned_items.is_empty() {
        groups.push(receiving_party_group(None, unassigned_items));
    }

    let mut item_count = 0u32;
    let mut addresses_with_known_balance = 0u32;
    let mut native_total = [0u8; 32];
    for group in &groups {
        item_count += group.item_count;
        addresses_with_known_balance +=
            group.items.iter().filter(|item| item.balance_known).count() as u32;
        native_total = add_u256(&native_total, &decoded_balance(&group.native_total_wei_hex));
    }

    ReceivingOverviewResponse {
        generated_at_unix: now,
        include_retired: false,
        groups,
        totals: ReceivingTotals {
            item_count,
            hd_count,
            stealth_count,
            native_total_wei_hex: encode_quantity_hex(&native_total),
        },
        coverage: ReceivingCoverage {
            addresses_total: item_count,
            addresses_with_known_balance,
            note: "Balances are from the last saved scan. Use Refresh balances to query your provider now.".into(),
        },
    }
}

fn receiving_linkage_warning_deposit_ids(
    deposits: &[EthStealthDeposit],
    parties: &[Counterparty],
) -> BTreeSet<String> {
    let mut bucket_by_deposit_id: BTreeMap<String, String> = BTreeMap::new();
    let mut identities_by_bucket: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for deposit in deposits {
        let Some(bucket) = stealth_dashboard_destination_bucket(deposit, parties) else {
            continue;
        };
        identities_by_bucket
            .entry(bucket.clone())
            .or_default()
            .insert(stealth_dashboard_identity_key(deposit));
        bucket_by_deposit_id.insert(deposit.id.clone(), bucket);
    }

    bucket_by_deposit_id
        .into_iter()
        .filter_map(|(deposit_id, bucket)| {
            identities_by_bucket
                .get(&bucket)
                .is_some_and(|identities| identities.len() > 1)
                .then_some(deposit_id)
        })
        .collect()
}

fn stealth_dashboard_destination_bucket(
    deposit: &EthStealthDeposit,
    parties: &[Counterparty],
) -> Option<String> {
    if let Some(destination) = trimmed_str(deposit.sweep_destination_address.as_deref()) {
        return Some(format!(
            "destination:{}",
            normalize_stealth_dashboard_linkage_key(destination)
        ));
    }

    if let Some(counterparty_id) = trimmed_str(deposit.counterparty_id.as_deref()) {
        return parties
            .iter()
            .find(|party| party.id.as_str() == counterparty_id)
            .and_then(|party| trimmed_str(party.sweep_destination_address.as_deref()))
            .map(|destination| {
                format!(
                    "destination:{}",
                    normalize_stealth_dashboard_linkage_key(destination)
                )
            });
    }

    trimmed_str(Some(&deposit.wallet_profile)).map(|profile| format!("wallet_profile:{profile}"))
}

fn stealth_dashboard_identity_key(deposit: &EthStealthDeposit) -> String {
    trimmed_str(deposit.counterparty_id.as_deref())
        .map(|id| format!("counterparty:{id}"))
        .unwrap_or_else(|| {
            format!(
                "unattributed:{}",
                normalize_stealth_dashboard_linkage_key(&deposit.stealth_address)
            )
        })
}

fn normalize_stealth_dashboard_linkage_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn trimmed_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn hd_receiving_item(
    allocation: &TreasuryReceiveAllocation,
    balances_by_address: &BTreeMap<String, [u8; 32]>,
) -> ReceivingItem {
    let balance = balances_by_address.get(&allocation.address.to_ascii_lowercase());
    let (balance_known, balance_native_wei_hex) = match balance {
        Some(balance) => (true, Some(encode_quantity_hex(balance))),
        None => (false, None),
    };

    ReceivingItem {
        source_type: "hd".into(),
        address: allocation.address.clone(),
        chain_id: allocation.chain_id,
        chain_id_assumed: allocation.chain_id_assumed,
        derivation_path: Some(allocation.derivation_path.clone()),
        purpose: Some(allocation.purpose.clone()),
        label: allocation.label.clone(),
        counterparty_id: allocation.counterparty_id.clone(),
        linkage_warning: None,
        balance_native_wei_hex,
        balance_known,
        status: allocation.status.clone(),
        created_at_unix: allocation.created_at_unix,
    }
}

fn stealth_receiving_item(deposit: &EthStealthDeposit) -> ReceivingItem {
    ReceivingItem {
        source_type: "stealth".into(),
        address: deposit.stealth_address.clone(),
        chain_id: deposit.chain_id,
        chain_id_assumed: deposit.chain_id_assumed,
        derivation_path: None,
        purpose: None,
        label: deposit.note.clone(),
        counterparty_id: deposit.counterparty_id.clone(),
        linkage_warning: None,
        balance_native_wei_hex: Some(
            deposit
                .observed_native_balance_wei_hex
                .clone()
                .unwrap_or_else(|| "0x0".to_string()),
        ),
        balance_known: true,
        status: deposit.status.clone(),
        created_at_unix: deposit.created_at_unix,
    }
}

fn receiving_party_group(
    counterparty: Option<Counterparty>,
    items: Vec<ReceivingItem>,
) -> ReceivingPartyGroup {
    let native_total = receiving_items_native_total(&items);
    ReceivingPartyGroup {
        counterparty,
        item_count: items.len() as u32,
        native_total_wei_hex: encode_quantity_hex(&native_total),
        items,
    }
}

fn receiving_items_native_total(items: &[ReceivingItem]) -> [u8; 32] {
    let mut total = [0u8; 32];
    for item in items {
        if item.balance_known {
            if let Some(balance) = item.balance_native_wei_hex.as_deref() {
                total = add_u256(&total, &decoded_balance(balance));
            }
        }
    }
    total
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

    pub(crate) fn receiving_overview(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<sigillum_api::ReceivingOverviewResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;

        Ok(build_receiving_overview(&state, &deposits, now_unix()))
    }

    pub(crate) async fn refresh_receiving_balances(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<ReceivingRefreshResponse> {
        let token =
            self.require_scope(token, crate::service::capability_scopes::DEPOSITS_REFRESH)?;
        let _guard = self.state.operation_guard().await;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let providers = if registry.evm_providers.is_empty() {
            Vec::new()
        } else {
            select_providers(&registry.evm_providers, None)?
        };
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let mut seen_addresses = BTreeSet::new();
        let mut active_allocations = Vec::new();
        for allocation in inventory
            .receive_allocations
            .iter()
            .filter(|allocation| allocation.status == RECEIVE_STATUS_ACTIVE)
        {
            let normalized_address = normalize_address(&allocation.address)?;
            if seen_addresses.insert(normalized_address.clone()) {
                let mut allocation = allocation.clone();
                allocation.address = normalized_address;
                active_allocations.push(allocation);
            }
        }

        let addresses_requested = usize_to_u32_count(active_allocations.len());
        let cap = self.state.runtime_policy().receiving_refresh_address_cap;
        let addresses_skipped = active_allocations.len().saturating_sub(cap);
        active_allocations.truncate(cap);

        let mut errors = Vec::new();
        let mut provider_error_count = 0usize;
        let mut refreshed_addresses = BTreeSet::new();
        if !providers.is_empty() {
            let limit = self
                .state
                .runtime_policy()
                .provider_balance_observation_concurrency
                .max(1);
            let mut work_items: Vec<(TreasuryReceiveAllocation, EvmProviderProfile)> = Vec::new();
            for allocation in &active_allocations {
                for provider in &providers {
                    work_items.push((allocation.clone(), provider.clone()));
                }
            }

            for chunk in work_items.chunks(limit) {
                for (allocation, provider) in chunk {
                    match self
                        .evm_native_balance_for_provider(
                            provider.compartment_id,
                            provider,
                            &allocation.address,
                            "latest",
                        )
                        .await
                    {
                        Ok(native_balance_wei_hex) => {
                            let now = now_unix();
                            let activity_state = if quantity_hex_is_nonzero(&native_balance_wei_hex)
                            {
                                sigillum_api::WalletAddressActivityState::Funded
                            } else {
                                sigillum_api::WalletAddressActivityState::Empty
                            };
                            upsert_address(
                                &mut inventory.addresses,
                                WalletInventoryAddress {
                                    id: random_id(),
                                    wallet_family: allocation.wallet_family.clone(),
                                    wallet_profile: allocation.wallet_profile.clone(),
                                    provider_profile: provider.name.clone(),
                                    chain_id: provider.chain_id,
                                    address: allocation.address.clone(),
                                    derivation_path: allocation.derivation_path.clone(),
                                    derivation_pattern: None,
                                    account_index: None,
                                    address_index: allocation.address_index,
                                    activity_state,
                                    native_balance_wei_hex,
                                    transaction_count: 0,
                                    last_activity_block: None,
                                    classifications: Vec::new(),
                                    source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
                                    first_seen_at_unix: now,
                                    last_checked_at_unix: now,
                                },
                            );
                            refreshed_addresses.insert(allocation.address.to_ascii_lowercase());
                        }
                        Err(error) => {
                            provider_error_count += 1;
                            errors.push(format!(
                                "provider={} chain={} address={}: {}",
                                provider.name, provider.chain_id, allocation.address, error
                            ));
                        }
                    }
                }
            }
        }
        save_inventory_state(&self.state.base_dir, &inventory)?;

        let stealth_result: ServiceResult<()> = async {
            let mut deposits =
                crate::deposits::load_deposits(&self.state.base_dir).map_err(|error| {
                    ServiceError::internal(format!("Failed to load deposits: {error}"))
                })?;
            let mut queue =
                crate::queue_store::load_queue(&self.state.base_dir).map_err(|error| {
                    ServiceError::internal(format!("Failed to load queue: {error}"))
                })?;
            self.refresh_eth_stealth_deposits_state(
                token,
                &mut deposits,
                &mut queue,
                EthStealthDepositRefreshRequest {
                    id: None,
                    limit: None,
                    auto_enqueue: Some(false),
                },
            )
            .await?;
            crate::queue_store::save_queue(&self.state.base_dir, &queue).map_err(|error| {
                ServiceError::internal(format!("Failed to save queue: {error}"))
            })?;
            crate::deposits::save_deposits(&self.state.base_dir, &deposits).map_err(|error| {
                ServiceError::internal(format!("Failed to save deposits: {error}"))
            })?;
            Ok(())
        }
        .await;
        let stealth_refreshed = match stealth_result {
            Ok(()) => true,
            Err(error) => {
                errors.push(format!("stealth refresh failed: {error}"));
                false
            }
        };

        let addresses_refreshed = usize_to_u32_count(refreshed_addresses.len());
        let addresses_skipped = usize_to_u32_count(addresses_skipped);
        let provider_status =
            if providers.is_empty() || (provider_error_count > 0 && addresses_refreshed == 0) {
                "no_provider"
            } else if provider_error_count > 0 {
                "partial"
            } else {
                "ok"
            }
            .to_string();

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::ReceivingRefreshBalances {
                addresses_requested,
                addresses_refreshed,
                addresses_skipped,
                stealth_refreshed,
            },
        )?;

        Ok(ReceivingRefreshResponse {
            generated_at_unix: now_unix(),
            addresses_requested,
            addresses_refreshed,
            addresses_skipped,
            stealth_refreshed,
            provider_status,
            errors,
        })
    }

    pub(crate) fn get_treasury_policy(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryPolicyResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(TreasuryPolicyResponse {
            policy: state.treasury_policy,
        })
    }

    pub(crate) async fn update_treasury_policy(
        &self,
        token: Option<&str>,
        body: TreasuryPolicyUpdateRequest,
    ) -> ServiceResult<TreasuryPolicyMutationResponse> {
        let token = self.require_session(token)?;
        if body.simulation_freshness_secs == Some(0) {
            return Err(ServiceError::bad_request(
                "simulation_freshness_secs must be greater than 0",
            ));
        }
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let previous_policy = state.treasury_policy.clone();
        let now = now_unix();

        let mut allowed_destinations: Vec<TreasuryAllowedDestination> = Vec::new();
        for destination in body.allowed_destinations {
            let address = normalize_address(&destination.address)?;
            // Dedupe case-insensitively and keep the first label, so repeated
            // operator input cannot silently relabel an approved destination.
            if allowed_destinations
                .iter()
                .any(|existing| existing.address.eq_ignore_ascii_case(&address))
            {
                continue;
            }
            allowed_destinations.push(TreasuryAllowedDestination {
                address,
                label: destination.label.and_then(trimmed_optional),
            });
        }

        let hot_floor_wei_hex = validated_required_quantity_hex(
            "hot_floor_wei_hex",
            body.hot_floor_wei_hex,
            DEFAULT_HOT_FLOOR_WEI_HEX,
        )?;
        let hot_target_wei_hex = validated_required_quantity_hex(
            "hot_target_wei_hex",
            body.hot_target_wei_hex,
            DEFAULT_HOT_TARGET_WEI_HEX,
        )?;
        let hot_floor = decode_quantity_hex(&hot_floor_wei_hex).map_err(|_| {
            ServiceError::bad_request("hot_floor_wei_hex must be a hex uint256 quantity")
        })?;
        let hot_target = decode_quantity_hex(&hot_target_wei_hex).map_err(|_| {
            ServiceError::bad_request("hot_target_wei_hex must be a hex uint256 quantity")
        })?;
        if compare_u256(&hot_floor, &hot_target).is_gt() {
            return Err(ServiceError::bad_request(
                "hot_floor_wei_hex must be less than or equal to hot_target_wei_hex",
            ));
        }
        let hot_overflow_wei_hex =
            validated_cap_hex("hot_overflow_wei_hex", body.hot_overflow_wei_hex)?;
        if let Some(hot_overflow_wei_hex) = hot_overflow_wei_hex.as_ref() {
            let hot_overflow = decode_quantity_hex(hot_overflow_wei_hex).map_err(|_| {
                ServiceError::bad_request("hot_overflow_wei_hex must be a hex uint256 quantity")
            })?;
            if compare_u256(&hot_target, &hot_overflow).is_gt() {
                return Err(ServiceError::bad_request(
                    "hot_target_wei_hex must be less than or equal to hot_overflow_wei_hex",
                ));
            }
        }
        let previous_execution_paused = previous_policy
            .as_ref()
            .map(|policy| policy.execution_paused)
            .unwrap_or(false);
        // Policy edits that omit the kill switch must not silently resume execution.
        let execution_paused = body.execution_paused.unwrap_or(previous_execution_paused);

        let policy = TreasuryPolicy {
            enabled: body.enabled,
            allowed_destinations,
            max_step_native_wei_hex: validated_cap_hex(
                "max_step_native_wei_hex",
                body.max_step_native_wei_hex,
            )?,
            max_plan_native_wei_hex: validated_cap_hex(
                "max_plan_native_wei_hex",
                body.max_plan_native_wei_hex,
            )?,
            require_simulation: body.require_simulation.unwrap_or(true),
            allow_raw_digest_signing: body.allow_raw_digest_signing.unwrap_or(false),
            block_cross_party_linkage: body.block_cross_party_linkage.unwrap_or(false),
            allow_claim_execution: body.allow_claim_execution.unwrap_or(false),
            allow_gas_topups: body.allow_gas_topups.unwrap_or(false),
            max_gas_topup_wei_hex: validated_cap_hex(
                "max_gas_topup_wei_hex",
                body.max_gas_topup_wei_hex,
            )?,
            allow_plan_execution: body.allow_plan_execution.unwrap_or(false),
            allow_sweep_execution: body.allow_sweep_execution.unwrap_or(false),
            allow_revoke_execution: body.allow_revoke_execution.unwrap_or(false),
            allow_exit_execution: body.allow_exit_execution.unwrap_or(false),
            execution_paused,
            max_fee_per_gas_cap_hex: validated_cap_hex(
                "max_fee_per_gas_cap_hex",
                body.max_fee_per_gas_cap_hex,
            )?,
            simulation_freshness_secs: body.simulation_freshness_secs.unwrap_or(900),
            hot_floor_wei_hex,
            hot_target_wei_hex,
            hot_overflow_wei_hex,
            allow_treasury_automation: body.allow_treasury_automation.unwrap_or(false),
            created_at_unix: previous_policy
                .as_ref()
                .map(|existing| existing.created_at_unix)
                .unwrap_or(now),
            updated_at_unix: now,
        };
        state.treasury_policy = Some(policy.clone());
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPolicyUpdate {
                enabled: policy.enabled,
                destinations: policy.allowed_destinations.len(),
            },
        )?;
        let fingerprint_hex = session_fingerprint_hex(token);
        for (gate, old_value, new_value) in [
            (
                "allow_plan_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_plan_execution)
                    .unwrap_or(false),
                policy.allow_plan_execution,
            ),
            (
                "allow_sweep_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_sweep_execution)
                    .unwrap_or(false),
                policy.allow_sweep_execution,
            ),
            (
                "allow_revoke_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_revoke_execution)
                    .unwrap_or(false),
                policy.allow_revoke_execution,
            ),
            (
                "allow_exit_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_exit_execution)
                    .unwrap_or(false),
                policy.allow_exit_execution,
            ),
            (
                "allow_claim_execution",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_claim_execution)
                    .unwrap_or(false),
                policy.allow_claim_execution,
            ),
            (
                "allow_gas_topups",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_gas_topups)
                    .unwrap_or(false),
                policy.allow_gas_topups,
            ),
            (
                "allow_treasury_automation",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.allow_treasury_automation)
                    .unwrap_or(false),
                policy.allow_treasury_automation,
            ),
            (
                "execution_paused",
                previous_policy
                    .as_ref()
                    .map(|policy| policy.execution_paused)
                    .unwrap_or(false),
                policy.execution_paused,
            ),
        ] {
            if old_value != new_value {
                self.record_audit(
                    self.state.active_compartment_id_for(token),
                    AuditEventSpec::TreasuryExecutionGateUpdate {
                        gate: gate.into(),
                        old_value,
                        new_value,
                        session_fingerprint_hex: fingerprint_hex.clone(),
                    },
                )?;
            }
        }

        Ok(TreasuryPolicyMutationResponse {
            status: "updated".into(),
            policy,
        })
    }

    /// All receive allocations, active and retired, in allocation order.
    pub(crate) fn list_treasury_receive_allocations(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<TreasuryReceiveAllocationListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(TreasuryReceiveAllocationListResponse {
            allocations: state.receive_allocations,
        })
    }

    pub(crate) fn list_parties(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<CounterpartyListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(CounterpartyListResponse {
            parties: state.parties,
        })
    }

    pub(crate) async fn create_party(
        &self,
        token: Option<&str>,
        body: CounterpartyCreateRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let name = trimmed_required("name", &body.name)?;
        let note = body.note.and_then(trimmed_optional);
        let sweep_destination_address = body
            .sweep_destination_address
            .and_then(trimmed_optional)
            .map(|address| normalize_address(&address))
            .transpose()?;

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let party = Counterparty {
            id: random_id(),
            name,
            note,
            sweep_destination_address,
            created_at_unix: now_unix(),
        };
        state.parties.push(party.clone());
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPartyCreate {
                name: party.name.clone(),
            },
        )?;

        Ok(CounterpartyMutationResponse {
            status: "created".into(),
            party: Some(party),
        })
    }

    pub(crate) async fn update_party(
        &self,
        token: Option<&str>,
        body: CounterpartyUpdateRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let _ = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let id = body.id.trim().to_string();
        let name = trimmed_required("name", &body.name)?;
        let note = body.note.and_then(trimmed_optional);
        // Omitted keeps the stored destination; an explicit blank clears it.
        let sweep_destination_address = body
            .sweep_destination_address
            .map(|value| {
                let value = value.trim();
                if value.is_empty() {
                    Ok(None)
                } else {
                    normalize_address(value).map(Some)
                }
            })
            .transpose()?;

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(party) = state
            .parties
            .iter_mut()
            .find(|party| party.id.as_str() == id.as_str())
        else {
            return Err(ServiceError::not_found("Counterparty not found."));
        };
        party.name = name;
        party.note = note;
        if let Some(sweep_destination_address) = sweep_destination_address {
            party.sweep_destination_address = sweep_destination_address;
        }
        let updated = party.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        Ok(CounterpartyMutationResponse {
            status: "updated".into(),
            party: Some(updated),
        })
    }

    pub(crate) async fn delete_party(
        &self,
        token: Option<&str>,
        body: CounterpartyDeleteRequest,
    ) -> ServiceResult<CounterpartyMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let id = body.id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(position) = state
            .parties
            .iter()
            .position(|party| party.id.as_str() == id.as_str())
        else {
            return Err(ServiceError::not_found("Counterparty not found."));
        };
        let name = state.parties[position].name.clone();
        for allocation in &mut state.receive_allocations {
            if allocation.counterparty_id.as_deref() == Some(id.as_str()) {
                allocation.counterparty_id = None;
            }
        }
        state.parties.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryPartyDelete { name },
        )?;

        Ok(CounterpartyMutationResponse {
            status: "deleted".into(),
            party: None,
        })
    }

    /// Allocate a fresh purpose-labeled receive address for a wallet profile.
    ///
    /// Privacy: derivation is pure local xpub math — no provider or network
    /// I/O — and every allocation takes the next unused receive index, so a
    /// counterparty only ever sees an address that has never been handed out
    /// for another purpose. Fresh-per-purpose addresses keep unrelated
    /// payments unlinkable on-chain.
    pub(crate) async fn allocate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveAllocateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let wallet_profile = trimmed_required("wallet_profile", &body.wallet_profile)?;
        let purpose = trimmed_required("purpose", &body.purpose)?;
        let label = body.label.and_then(trimmed_optional);

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let counterparty_id = body.counterparty_id.and_then(trimmed_optional);
        let counterparty_name = if let Some(id) = counterparty_id.as_deref() {
            let Some(party) = state.parties.iter().find(|party| party.id.as_str() == id) else {
                return Err(ServiceError::not_found("Counterparty not found."));
            };
            Some(party.name.clone())
        } else {
            None
        };
        let allocation = self.issue_receive_allocation(
            &mut state,
            &wallet_profile,
            purpose,
            label,
            counterparty_id,
        )?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveAllocate {
                wallet_profile: allocation.wallet_profile.clone(),
                purpose: allocation.purpose.clone(),
            },
        )?;
        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "allocated".into(),
            allocation,
        })
    }

    /// Retire an active allocation and issue the next index for the same
    /// wallet profile, carrying over its purpose and label.
    ///
    /// Both mutations land in a single state save: either the old allocation
    /// is retired AND its replacement exists, or neither change persists.
    /// Like allocation, the replacement address is derived locally from the
    /// profile xpub with no network I/O.
    pub(crate) async fn rotate_treasury_receive_address(
        &self,
        token: Option<&str>,
        body: TreasuryReceiveRotateRequest,
    ) -> ServiceResult<TreasuryReceiveAllocationMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let allocation_id = body.allocation_id.trim().to_string();

        let mut state = load_inventory_state(&self.state.base_dir)?;
        let Some(existing) = state
            .receive_allocations
            .iter_mut()
            .find(|allocation| allocation.id == allocation_id)
        else {
            return Err(ServiceError::not_found("Receive allocation not found."));
        };
        if existing.status != RECEIVE_STATUS_ACTIVE {
            return Err(ServiceError::bad_request(
                "Receive allocation is not active.",
            ));
        }
        existing.status = RECEIVE_STATUS_RETIRED.into();
        existing.retired_at_unix = Some(now_unix());
        let wallet_profile = existing.wallet_profile.clone();
        let purpose = existing.purpose.clone();
        let label = existing.label.clone();
        let counterparty_id = existing.counterparty_id.clone();
        let counterparty_name = counterparty_id.as_deref().and_then(|id| {
            state
                .parties
                .iter()
                .find(|party| party.id.as_str() == id)
                .map(|party| party.name.clone())
        });

        let allocation = self.issue_receive_allocation(
            &mut state,
            &wallet_profile,
            purpose,
            label,
            counterparty_id,
        )?;
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::TreasuryReceiveRotate { id: allocation_id },
        )?;
        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        Ok(TreasuryReceiveAllocationMutationResponse {
            status: "rotated".into(),
            allocation,
        })
    }

    /// Derive the next receive allocation for `wallet_profile` and append it
    /// to `state` (callers persist the state afterwards).
    ///
    /// Profile resolution prefers seed profiles over xpub profiles with the
    /// same name, matching discovery-wallet selection. The next index is one
    /// past the highest index either previously allocated or already observed
    /// in scanned inventory, so fresh allocations never collide with
    /// addresses the treasury has used before.
    fn issue_receive_allocation(
        &self,
        state: &mut WalletInventoryState,
        wallet_profile: &str,
        purpose: String,
        label: Option<String>,
        counterparty_id: Option<String>,
    ) -> ServiceResult<TreasuryReceiveAllocation> {
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let wallets = select_discovery_wallets(
            self,
            &registry.eth_seed_wallets,
            &registry.eth_xpub_wallets,
            None,
            Some(wallet_profile),
            SeedDerivationPattern::Project,
            1,
        )?;
        let Some(wallet) = wallets.into_iter().next() else {
            return Err(ServiceError::not_found("Wallet profile not found."));
        };
        let chain_id = provider_chain_id_for_discovery_wallet(&registry, &wallet)?;

        let next_index = next_receive_index(
            &state.receive_allocations,
            &state.addresses,
            &wallet.family,
            &wallet.profile,
        );
        if next_index > MAX_RECEIVE_INDEX {
            return Err(ServiceError::bad_request("Receive index space exhausted."));
        }

        let derived =
            derive_discovery_wallet_address(&wallet, next_index).map_err(map_xpub_error)?;
        let allocation = TreasuryReceiveAllocation {
            id: random_id(),
            wallet_family: wallet.family.clone(),
            wallet_profile: wallet.profile.clone(),
            chain_id,
            chain_id_assumed: false,
            address: derived.address,
            derivation_path: format!("{}/{}", wallet.receive_path, next_index),
            address_index: next_index,
            purpose,
            label,
            status: RECEIVE_STATUS_ACTIVE.into(),
            created_at_unix: now_unix(),
            retired_at_unix: None,
            counterparty_id,
        };
        state.receive_allocations.push(allocation.clone());
        Ok(allocation)
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

fn provider_chain_id_for_discovery_wallet(
    registry: &crate::profiles::ProfileRegistry,
    wallet: &super::wallet_selection::DiscoveryWallet,
) -> ServiceResult<u64> {
    let provider_profile = match wallet.family.as_str() {
        WALLET_FAMILY_ETH_SEED => registry
            .eth_seed_wallets
            .iter()
            .find(|profile| profile.name == wallet.profile)
            .map(|profile| profile.provider_profile.as_str()),
        WALLET_FAMILY_ETH_XPUB => registry
            .eth_xpub_wallets
            .iter()
            .find(|profile| profile.name == wallet.profile)
            .map(|profile| profile.provider_profile.as_str()),
        _ => None,
    }
    .ok_or_else(|| ServiceError::not_found("Wallet profile not found."))?;

    registry
        .evm_providers
        .iter()
        .find(|provider| provider.name == provider_profile)
        .map(|provider| provider.chain_id)
        .ok_or_else(|| ServiceError::not_found("Provider profile not found."))
}

/// Console rollup of allocation counts and distinct active purposes.
fn receive_summary(allocations: &[TreasuryReceiveAllocation]) -> TreasuryReceiveSummary {
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

/// Next unused receive index for one wallet profile.
///
/// Considers both prior allocations (including retired ones, which must never
/// be re-issued) and inventory rows discovered by scans, so the allocator
/// stays ahead of any index the wallet has already exposed on-chain. Returns
/// 0 only when neither source knows the profile.
fn next_receive_index(
    allocations: &[TreasuryReceiveAllocation],
    addresses: &[WalletInventoryAddress],
    wallet_family: &str,
    wallet_profile: &str,
) -> u32 {
    let allocated_max = allocations
        .iter()
        .filter(|allocation| allocation.wallet_profile == wallet_profile)
        .map(|allocation| allocation.address_index)
        .max();
    let observed_max = addresses
        .iter()
        .filter(|address| {
            address.wallet_family == wallet_family && address.wallet_profile == wallet_profile
        })
        .map(|address| address.address_index)
        .max();
    match allocated_max.into_iter().chain(observed_max).max() {
        Some(max_index) => max_index.saturating_add(1),
        None => 0,
    }
}

/// Reject caps that would not decode during enforcement: a cap that silently
/// fails to parse later would be a guardrail that never fires.
fn validated_cap_hex(field: &str, value: Option<String>) -> ServiceResult<Option<String>> {
    let Some(value) = value.and_then(trimmed_optional) else {
        return Ok(None);
    };
    if !has_hex_quantity_prefix(&value) {
        return Err(ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        )));
    }
    decode_quantity_hex(&value).map_err(|_| {
        ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        ))
    })?;
    Ok(Some(value))
}

fn validated_required_quantity_hex(
    field: &str,
    value: Option<String>,
    default_value: &str,
) -> ServiceResult<String> {
    let value = value
        .and_then(trimmed_optional)
        .unwrap_or_else(|| default_value.to_string());
    if !has_hex_quantity_prefix(&value) {
        return Err(ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        )));
    }
    decode_quantity_hex(&value).map_err(|_| {
        ServiceError::bad_request(format!(
            "{field} must be a 0x-prefixed hex uint256 quantity"
        ))
    })?;
    Ok(value)
}

fn has_hex_quantity_prefix(value: &str) -> bool {
    value.starts_with("0x") || value.starts_with("0X")
}

/// Treasury policy blockers for a single consolidation plan step.
///
/// Returned markers extend the step's planner blockers; policy violations
/// block a step rather than rewriting it, so the operator always sees which
/// guardrail fired. Only sweep actions are destination-routed, and only
/// native amounts are comparable in wei, so other actions pass untouched.
/// A sweep step with no destination is already a planner blocker
/// (`missing_destination`) and is not duplicated here.
pub(super) fn policy_blockers_for_step(
    policy: &TreasuryPolicy,
    action: &str,
    destination_address: Option<&str>,
    asset_kind: &str,
    amount_hex: &str,
) -> Vec<String> {
    if !action.starts_with("sweep") && action != "raw_digest" {
        return Vec::new();
    }
    let kind = if action == "raw_digest" {
        TransactionPolicyKind::RawDigest
    } else {
        TransactionPolicyKind::RoutedTransfer
    };
    transaction_policy_actions(
        policy,
        TransactionPolicyCheck {
            kind,
            destination_address,
            asset_kind,
            amount_hex,
        },
    )
    .into_iter()
    .map(|action| action.as_str().to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_u256_carries_and_saturates() {
        let one = decode_quantity_hex("0x1").unwrap();
        let max_byte = decode_quantity_hex("0xff").unwrap();
        let sum = add_u256(&one, &max_byte);
        assert_eq!(encode_quantity_hex(&sum), "0x100");

        let max = [0xffu8; 32];
        let saturated = add_u256(&max, &one);
        assert_eq!(saturated, [0xffu8; 32]);
    }

    #[test]
    fn encode_quantity_hex_trims_leading_zeroes() {
        assert_eq!(encode_quantity_hex(&[0u8; 32]), "0x0");
        let value = decode_quantity_hex("0xde0b6b3a7640000").unwrap();
        assert_eq!(encode_quantity_hex(&value), "0xde0b6b3a7640000");
    }

    #[test]
    fn build_receiving_overview_groups_active_hd_and_stealth_deposits() {
        let party = Counterparty {
            id: "party_1".into(),
            name: "Acme Labs".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 1,
        };
        let named_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let unresolved_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let untagged_stealth_address = "0xcccccccccccccccccccccccccccccccccccccccc";
        let retired_address = "0xdddddddddddddddddddddddddddddddddddddddd";
        let tagged_stealth_address = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let state = WalletInventoryState {
            parties: vec![party],
            addresses: vec![
                receiving_inventory_address("named_a", named_address, "0x1"),
                receiving_inventory_address("named_b", &named_address.to_ascii_uppercase(), "0x2"),
            ],
            receive_allocations: vec![
                receiving_allocation(
                    "alloc_named",
                    named_address,
                    RECEIVE_STATUS_ACTIVE,
                    Some("party_1"),
                    10,
                ),
                receiving_allocation(
                    "alloc_unresolved",
                    unresolved_address,
                    RECEIVE_STATUS_ACTIVE,
                    Some("missing_party"),
                    11,
                ),
                receiving_allocation(
                    "alloc_retired",
                    retired_address,
                    RECEIVE_STATUS_RETIRED,
                    Some("party_1"),
                    12,
                ),
            ],
            ..WalletInventoryState::default()
        };
        let deposits = DepositState {
            eth_stealth: vec![
                receiving_stealth_deposit("dep_1", untagged_stealth_address, Some("0x4"), None),
                receiving_stealth_deposit(
                    "dep_2",
                    tagged_stealth_address,
                    Some("0x5"),
                    Some("party_1"),
                ),
            ],
            announcement_scan_cursors: Vec::new(),
        };

        let overview = build_receiving_overview(&state, &deposits, 123);

        assert_eq!(overview.generated_at_unix, 123);
        assert!(!overview.include_retired);
        assert_eq!(overview.groups.len(), 2);

        let named = &overview.groups[0];
        assert_eq!(named.counterparty.as_ref().unwrap().id, "party_1");
        assert_eq!(named.item_count, 2);
        assert_eq!(named.native_total_wei_hex, "0x8");
        assert_eq!(named.items[0].source_type, "hd");
        assert_eq!(named.items[0].counterparty_id.as_deref(), Some("party_1"));
        assert!(named.items[0].balance_known);
        assert_eq!(
            named.items[0].balance_native_wei_hex.as_deref(),
            Some("0x3")
        );
        let tagged_stealth = named
            .items
            .iter()
            .find(|item| item.address == tagged_stealth_address)
            .expect("tagged stealth item");
        assert_eq!(tagged_stealth.source_type, "stealth");
        assert_eq!(tagged_stealth.counterparty_id.as_deref(), Some("party_1"));
        assert!(tagged_stealth.balance_known);
        assert_eq!(
            tagged_stealth.balance_native_wei_hex.as_deref(),
            Some("0x5")
        );

        let unassigned = &overview.groups[1];
        assert!(unassigned.counterparty.is_none());
        assert_eq!(unassigned.item_count, 2);
        assert_eq!(unassigned.native_total_wei_hex, "0x4");

        let unresolved = unassigned
            .items
            .iter()
            .find(|item| item.address == unresolved_address)
            .expect("unresolved HD item");
        assert_eq!(unresolved.source_type, "hd");
        assert_eq!(unresolved.counterparty_id.as_deref(), Some("missing_party"));
        assert!(!unresolved.balance_known);
        assert!(unresolved.balance_native_wei_hex.is_none());

        let stealth = unassigned
            .items
            .iter()
            .find(|item| item.address == untagged_stealth_address)
            .expect("untagged stealth item");
        assert_eq!(stealth.source_type, "stealth");
        assert!(stealth.balance_known);
        assert_eq!(stealth.balance_native_wei_hex.as_deref(), Some("0x4"));
        assert_eq!(stealth.label.as_deref(), Some("stealth note"));
        assert!(stealth.counterparty_id.is_none());

        assert!(
            !overview
                .groups
                .iter()
                .flat_map(|group| group.items.iter())
                .any(|item| item.address == retired_address)
        );
        assert_eq!(overview.totals.item_count, 4);
        assert_eq!(overview.totals.hd_count, 2);
        assert_eq!(overview.totals.stealth_count, 2);
        assert_eq!(overview.totals.native_total_wei_hex, "0xc");
        assert_eq!(overview.coverage.addresses_total, 4);
        assert_eq!(overview.coverage.addresses_with_known_balance, 3);
    }

    #[test]
    fn build_receiving_overview_warns_for_cross_party_stealth_sweep_destinations() {
        let destination = "0x9999999999999999999999999999999999999999";
        let parties = vec![
            Counterparty {
                id: "party_1".into(),
                name: "Acme Labs".into(),
                note: None,
                sweep_destination_address: None,
                created_at_unix: 1,
            },
            Counterparty {
                id: "party_2".into(),
                name: "Beta Labs".into(),
                note: None,
                sweep_destination_address: None,
                created_at_unix: 2,
            },
        ];
        let mut party_one_deposit = receiving_stealth_deposit(
            "dep_1",
            "0x1111111111111111111111111111111111111111",
            Some("0x1"),
            Some("party_1"),
        );
        party_one_deposit.sweep_destination_address = Some(destination.into());
        let mut party_two_deposit = receiving_stealth_deposit(
            "dep_2",
            "0x2222222222222222222222222222222222222222",
            Some("0x2"),
            Some("party_2"),
        );
        party_two_deposit.sweep_destination_address = Some(destination.into());
        let state = WalletInventoryState {
            parties,
            ..WalletInventoryState::default()
        };
        let deposits = DepositState {
            eth_stealth: vec![party_one_deposit, party_two_deposit],
            announcement_scan_cursors: Vec::new(),
        };

        let overview = build_receiving_overview(&state, &deposits, 123);
        let warnings: Vec<_> = overview
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .map(|item| item.linkage_warning.as_deref())
            .collect();
        assert_eq!(
            warnings,
            vec![
                Some(RECEIVING_LINKAGE_WARNING),
                Some(RECEIVING_LINKAGE_WARNING)
            ]
        );

        let same_party = Counterparty {
            id: "party_same".into(),
            name: "Same Party".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 1,
        };
        let mut first_same_party_deposit = receiving_stealth_deposit(
            "dep_3",
            "0x3333333333333333333333333333333333333333",
            Some("0x3"),
            Some("party_same"),
        );
        first_same_party_deposit.sweep_destination_address = Some(destination.into());
        let mut second_same_party_deposit = receiving_stealth_deposit(
            "dep_4",
            "0x4444444444444444444444444444444444444444",
            Some("0x4"),
            Some("party_same"),
        );
        second_same_party_deposit.sweep_destination_address = Some(destination.into());
        let same_party_overview = build_receiving_overview(
            &WalletInventoryState {
                parties: vec![same_party],
                ..WalletInventoryState::default()
            },
            &DepositState {
                eth_stealth: vec![first_same_party_deposit, second_same_party_deposit],
                announcement_scan_cursors: Vec::new(),
            },
            123,
        );

        assert!(same_party_overview.groups.iter().all(|group| {
            group
                .items
                .iter()
                .all(|item| item.linkage_warning.is_none())
        }));
    }

    fn sample_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".into(),
                label: Some("cold-treasury".into()),
            }],
            max_step_native_wei_hex: Some("0xde0b6b3a7640000".into()),
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
            hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
            hot_target_wei_hex: "0xde0b6b3a7640000".into(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    #[test]
    fn policy_allows_allowlisted_sweep_within_cap() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0xde0b6b3a7640000",
        );
        assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
    }

    #[test]
    fn policy_allowlist_match_is_case_insensitive() {
        let destination = "0x9999999999999999999999999999999999999999".to_ascii_uppercase();
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some(destination.as_str()),
            "native",
            "0x1",
        );
        assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
    }

    #[test]
    fn policy_blocks_non_allowlisted_sweep_destination() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_erc20",
            Some("0x8888888888888888888888888888888888888888"),
            "erc20",
            "0x1",
        );
        assert_eq!(blockers, vec!["block_destination".to_string()]);
    }

    #[test]
    fn policy_blocks_native_amount_above_step_cap() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0xde0b6b3a7640001",
        );
        assert_eq!(blockers, vec!["block_step_cap".to_string()]);
    }

    #[test]
    fn policy_reports_destination_and_cap_violations_together() {
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "sweep_native",
            Some("0x8888888888888888888888888888888888888888"),
            "native",
            "0x1bc16d674ec80000",
        );
        assert_eq!(
            blockers,
            vec![
                "block_destination".to_string(),
                "block_step_cap".to_string(),
            ]
        );
    }

    #[test]
    fn disabled_policy_blocks_nothing() {
        let mut policy = sample_policy();
        policy.enabled = false;
        let blockers = policy_blockers_for_step(
            &policy,
            "sweep_native",
            Some("0x8888888888888888888888888888888888888888"),
            "native",
            "0xffffffffffffffffffffffff",
        );
        assert!(blockers.is_empty());
    }

    #[test]
    fn policy_ignores_non_sweep_actions_and_missing_destinations() {
        // Revokes and claims are not destination-routed value moves.
        let blockers = policy_blockers_for_step(
            &sample_policy(),
            "revoke_erc20_approval",
            Some("0x8888888888888888888888888888888888888888"),
            "approval",
            "0x1",
        );
        assert!(blockers.is_empty());

        // A sweep with no destination is already blocked by the planner.
        let blockers =
            policy_blockers_for_step(&sample_policy(), "sweep_erc20", None, "erc20", "0x1");
        assert!(blockers.is_empty());
    }

    #[test]
    fn empty_allowlist_blocks_routed_sweeps_when_enabled() {
        let mut policy = sample_policy();
        policy.allowed_destinations.clear();
        let blockers = policy_blockers_for_step(
            &policy,
            "sweep_native",
            Some("0x9999999999999999999999999999999999999999"),
            "native",
            "0x1",
        );
        assert_eq!(blockers, vec!["block_destination".to_string()]);
    }

    fn sample_allocation(
        wallet_profile: &str,
        address_index: u32,
        status: &str,
        purpose: &str,
    ) -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: format!("alloc_{wallet_profile}_{address_index}"),
            wallet_family: "eth-seed".into(),
            wallet_profile: wallet_profile.into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
            address_index,
            purpose: purpose.into(),
            label: None,
            status: status.into(),
            created_at_unix: 1,
            retired_at_unix: None,
            counterparty_id: None,
        }
    }

    fn sample_inventory_address(
        wallet_family: &str,
        wallet_profile: &str,
        address_index: u32,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: format!("addr_{wallet_profile}_{address_index}"),
            wallet_family: wallet_family.into(),
            wallet_profile: wallet_profile.into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x2222222222222222222222222222222222222222".into(),
            derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index,
            activity_state: "funded".into(),
            native_balance_wei_hex: "0x1".into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn receiving_allocation(
        id: &str,
        address: &str,
        status: &str,
        counterparty_id: Option<&str>,
        created_at_unix: u64,
    ) -> TreasuryReceiveAllocation {
        TreasuryReceiveAllocation {
            id: id.into(),
            wallet_family: "eth-xpub".into(),
            wallet_profile: "mainnet-xpub".into(),
            chain_id: 1,
            chain_id_assumed: false,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            address_index: 0,
            purpose: "invoice".into(),
            label: Some(format!("label-{id}")),
            status: status.into(),
            created_at_unix,
            retired_at_unix: (status == RECEIVE_STATUS_RETIRED).then_some(created_at_unix + 1),
            counterparty_id: counterparty_id.map(str::to_string),
        }
    }

    fn receiving_inventory_address(
        id: &str,
        address: &str,
        balance: &str,
    ) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: id.into(),
            wallet_family: "eth-xpub".into(),
            wallet_profile: "mainnet-xpub".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: address.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: balance.into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "persisted-test".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn receiving_stealth_deposit(
        id: &str,
        address: &str,
        balance: Option<&str>,
        counterparty_id: Option<&str>,
    ) -> EthStealthDeposit {
        EthStealthDeposit {
            id: id.into(),
            status: "detected".into(),
            asset_kind: "native".into(),
            wallet_profile: "mainnet-xpub".into(),
            chain_id: 1,
            chain_id_assumed: false,
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: "mainnet-xpub".into(),
            short_name: "eth".into(),
            stealth_meta_address: "st:eth:example".into(),
            stealth_address: address.into(),
            ephemeral_public_key_hex: "0x02".into(),
            view_tag_hex: "0xaa".into(),
            stealth_hash_convention: sigillum_core::StealthHashConvention::STANDARD,
            announcement: None,
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: None,
            observed_native_balance_wei_hex: balance.map(str::to_string),
            auto_queue_sweep: false,
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            queue_job_id: None,
            queue_job_state: None,
            note: Some("stealth note".into()),
            created_at_unix: 20,
            updated_at_unix: 21,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
            counterparty_id: counterparty_id.map(str::to_string),
            requested_gas_wei_hex: None,
            gas_topup_job_id: None,
            gas_topup_job_state: None,
        }
    }

    #[test]
    fn next_receive_index_starts_at_zero_when_nothing_is_known() {
        assert_eq!(next_receive_index(&[], &[], "eth-seed", "seed-main"), 0);
    }

    #[test]
    fn next_receive_index_advances_past_allocations_only() {
        let allocations = vec![
            sample_allocation("seed-main", 0, "retired", "acme"),
            sample_allocation("seed-main", 4, "active", "beta"),
        ];
        assert_eq!(
            next_receive_index(&allocations, &[], "eth-seed", "seed-main"),
            5
        );
    }

    #[test]
    fn next_receive_index_advances_past_inventory_only() {
        let addresses = vec![
            sample_inventory_address("eth-seed", "seed-main", 2),
            sample_inventory_address("eth-seed", "seed-main", 7),
        ];
        assert_eq!(
            next_receive_index(&[], &addresses, "eth-seed", "seed-main"),
            8
        );
    }

    #[test]
    fn next_receive_index_takes_the_max_of_both_sources() {
        let allocations = vec![sample_allocation("seed-main", 9, "active", "acme")];
        let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 3)];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            10
        );

        let allocations = vec![sample_allocation("seed-main", 1, "active", "acme")];
        let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 6)];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            7
        );
    }

    #[test]
    fn next_receive_index_ignores_other_profiles_and_families() {
        let allocations = vec![sample_allocation("seed-other", 40, "active", "acme")];
        let addresses = vec![
            sample_inventory_address("eth-seed", "seed-other", 50),
            // Same profile name under a different family does not count for
            // the inventory source.
            sample_inventory_address("eth-xpub", "seed-main", 60),
        ];
        assert_eq!(
            next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
            0
        );
    }

    #[test]
    fn receive_summary_counts_active_retired_and_distinct_purposes() {
        let allocations = vec![
            sample_allocation("seed-main", 0, "retired", "acme"),
            sample_allocation("seed-main", 1, "active", "acme"),
            sample_allocation("seed-main", 2, "active", "acme"),
            sample_allocation("seed-main", 3, "active", "beta"),
        ];
        let summary = receive_summary(&allocations);
        assert_eq!(summary.active_allocations, 3);
        assert_eq!(summary.retired_allocations, 1);
        // Retired purposes do not count; duplicates collapse.
        assert_eq!(summary.purposes, 2);

        assert_eq!(receive_summary(&[]), TreasuryReceiveSummary::default());
    }

    #[test]
    fn hd_receiving_item_uses_allocation_chain_id_and_assumption_marker() {
        let mut allocation = receiving_allocation(
            "alloc-base",
            "0x1111111111111111111111111111111111111111",
            RECEIVE_STATUS_ACTIVE,
            None,
            1,
        );
        allocation.chain_id = 8453;
        allocation.chain_id_assumed = true;

        let item = hd_receiving_item(&allocation, &BTreeMap::new());

        assert_eq!(item.chain_id, 8453);
        assert!(item.chain_id_assumed);
    }

    #[test]
    fn stealth_receiving_item_uses_deposit_chain_id_and_assumption_marker() {
        let mut deposit = receiving_stealth_deposit(
            "dep-base",
            "0x2222222222222222222222222222222222222222",
            Some("0x1"),
            None,
        );
        deposit.chain_id = 8453;
        deposit.chain_id_assumed = true;

        let item = stealth_receiving_item(&deposit);

        assert_eq!(item.chain_id, 8453);
        assert!(item.chain_id_assumed);
    }
}
