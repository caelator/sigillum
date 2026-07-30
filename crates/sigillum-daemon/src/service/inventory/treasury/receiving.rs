use std::collections::{BTreeMap, BTreeSet};

use sigillum_api::{
    Counterparty, EthStealthDeposit, EthStealthDepositRefreshRequest, EvmProviderProfile,
    ReceivingCoverage, ReceivingItem, ReceivingOverviewResponse, ReceivingPartyGroup,
    ReceivingRefreshResponse, ReceivingTotals, TreasuryReceiveAllocation, WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::audit_log::AuditEventSpec;
use crate::deposits::DepositState;
use crate::inventory::WalletInventoryState;
use crate::service::evm::normalize_address;
use crate::service::helpers::{add_u256, encode_quantity_hex, now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::DISCOVERY_SOURCE_LOCAL_RPC;
use super::super::support::{
    load_inventory_state, quantity_hex_is_nonzero, save_inventory_state, select_providers,
    upsert_address,
};
use super::allocations::RECEIVE_STATUS_ACTIVE;
use super::overview::decoded_balance;

pub(super) const RECEIVING_LINKAGE_WARNING: &str = "Sweeping here would link this payer with another party. Set a distinct per-party sweep destination.";

type ReceivingAllocationIdentity = (String, String, u64, String);

fn usize_to_u32_count(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

pub(super) fn receiving_allocation_identity(
    allocation: &TreasuryReceiveAllocation,
) -> ServiceResult<ReceivingAllocationIdentity> {
    Ok((
        allocation.wallet_family.clone(),
        allocation.wallet_profile.clone(),
        allocation.chain_id,
        normalize_address(&allocation.address)?,
    ))
}

pub(super) fn active_receiving_allocations(
    allocations: &[TreasuryReceiveAllocation],
) -> ServiceResult<Vec<TreasuryReceiveAllocation>> {
    let mut seen = BTreeSet::new();
    let mut active = Vec::new();
    for allocation in allocations
        .iter()
        .filter(|allocation| allocation.status == RECEIVE_STATUS_ACTIVE)
    {
        let identity = receiving_allocation_identity(allocation)?;
        if seen.insert(identity.clone()) {
            let mut allocation = allocation.clone();
            allocation.address = identity.3;
            active.push(allocation);
        }
    }
    Ok(active)
}

pub(super) fn receiving_refresh_work_items(
    allocations: &[TreasuryReceiveAllocation],
    providers: &[EvmProviderProfile],
) -> Vec<(TreasuryReceiveAllocation, EvmProviderProfile)> {
    let mut work_items = Vec::new();
    for allocation in allocations {
        for provider in providers
            .iter()
            .filter(|provider| provider.chain_id == allocation.chain_id)
        {
            work_items.push((allocation.clone(), provider.clone()));
        }
    }
    work_items
}

pub(super) fn build_receiving_overview(
    state: &WalletInventoryState,
    deposits: &DepositState,
    now: u64,
) -> ReceivingOverviewResponse {
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
        let item = hd_receiving_item(allocation, &state.addresses);
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

pub(super) fn hd_receiving_item(
    allocation: &TreasuryReceiveAllocation,
    addresses: &[WalletInventoryAddress],
) -> ReceivingItem {
    let selected_address = addresses
        .iter()
        .filter(|address| {
            address.wallet_family == allocation.wallet_family
                && address.wallet_profile == allocation.wallet_profile
                && address.chain_id == allocation.chain_id
                && address.address.eq_ignore_ascii_case(&allocation.address)
        })
        .max_by(|left, right| {
            left.last_checked_at_unix
                .cmp(&right.last_checked_at_unix)
                .then_with(|| left.provider_profile.cmp(&right.provider_profile))
                .then_with(|| left.id.cmp(&right.id))
        });
    let balance = selected_address
        .and_then(|address| decode_quantity_hex(&address.native_balance_wei_hex).ok());
    let (balance_known, balance_native_wei_hex) = match balance {
        Some(balance) => (true, Some(encode_quantity_hex(&balance))),
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
        balance_last_checked_at_unix: selected_address.map(|address| address.last_checked_at_unix),
        status: allocation.status.clone(),
        created_at_unix: allocation.created_at_unix,
    }
}

pub(super) fn stealth_receiving_item(deposit: &EthStealthDeposit) -> ReceivingItem {
    let balance_native_wei_hex = deposit.observed_native_balance_wei_hex.clone();
    let balance_known = balance_native_wei_hex.is_some();
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
        balance_native_wei_hex,
        balance_known,
        balance_last_checked_at_unix: deposit.last_checked_at_unix,
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
        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let providers = if registry.evm_providers.is_empty() {
            Vec::new()
        } else {
            select_providers(&registry.evm_providers, None)?
        };
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let mut active_allocations = active_receiving_allocations(&inventory.receive_allocations)?;

        let addresses_requested = usize_to_u32_count(active_allocations.len());
        let cap = self.state.runtime_policy().receiving_refresh_address_cap;
        let addresses_skipped = active_allocations.len().saturating_sub(cap);
        active_allocations.truncate(cap);

        let mut errors = Vec::new();
        let mut provider_error_count = 0usize;
        let mut refreshed_allocations = BTreeSet::new();
        if !providers.is_empty() {
            let limit = self
                .state
                .runtime_policy()
                .provider_balance_observation_concurrency
                .max(1);
            for allocation in &active_allocations {
                if !providers
                    .iter()
                    .any(|provider| provider.chain_id == allocation.chain_id)
                {
                    provider_error_count += 1;
                    errors.push(format!(
                        "no provider configured for chain={} wallet_family={} wallet_profile={} address={}",
                        allocation.chain_id,
                        allocation.wallet_family,
                        allocation.wallet_profile,
                        allocation.address
                    ));
                }
            }
            let work_items = receiving_refresh_work_items(&active_allocations, &providers);

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
                            refreshed_allocations
                                .insert(receiving_allocation_identity(allocation)?);
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

        let addresses_refreshed = usize_to_u32_count(refreshed_allocations.len());
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
}
