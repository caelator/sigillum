use sigillum_api::{EvmProviderProfile, WalletAssetHolding, WalletInventoryAddress};

use crate::service::helpers::random_id;
use crate::service::{ServiceError, ServiceResult};

use super::{
    DEFAULT_GAP_LIMIT, DEFAULT_MAX_INDEX, DISCOVERY_SOURCE_LOCAL_RPC, DiscoveryWallet,
    MAX_GAP_LIMIT, MAX_SCAN_INDEX, WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_XPUB,
};

#[derive(Clone, Debug)]
pub(super) struct InventoryAddressObservation {
    pub(super) address: WalletInventoryAddress,
    pub(super) holdings: Vec<WalletAssetHolding>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct InventoryRecordContext<'a> {
    pub(super) wallet: &'a DiscoveryWallet,
    pub(super) provider: &'a EvmProviderProfile,
    pub(super) address: &'a str,
    pub(super) derivation_path: &'a str,
    pub(super) now: u64,
}

pub(super) fn load_inventory_state(
    base_dir: &std::path::Path,
) -> ServiceResult<crate::inventory::WalletInventoryState> {
    crate::inventory::load_wallet_inventory(base_dir).map_err(|error| {
        ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
    })
}

pub(super) fn save_inventory_state(
    base_dir: &std::path::Path,
    state: &crate::inventory::WalletInventoryState,
) -> ServiceResult<()> {
    crate::inventory::save_wallet_inventory(base_dir, state).map_err(|error| {
        ServiceError::internal(format!("Failed to save wallet inventory: {error}"))
    })
}

pub(super) fn trimmed_required(field: &str, value: &str) -> ServiceResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ServiceError::bad_request(format!("{field} is required")));
    }
    Ok(value.to_string())
}

pub(super) fn trimmed_optional(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

pub(super) fn default_native_symbol(chain_family: &str) -> &'static str {
    match chain_family.trim() {
        "bitcoin" | "utxo" => "BTC",
        "solana" => "SOL",
        "tron" => "TRX",
        "cosmos" => "ATOM",
        _ => "ETH",
    }
}

pub(super) fn validated_gap_limit(value: Option<u32>) -> ServiceResult<u32> {
    let value = value.unwrap_or(DEFAULT_GAP_LIMIT);
    if value == 0 || value > MAX_GAP_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "gap_limit must be between 1 and {MAX_GAP_LIMIT}"
        )));
    }
    Ok(value)
}

pub(super) fn validated_max_index(value: Option<u32>) -> ServiceResult<u32> {
    let value = value.unwrap_or(DEFAULT_MAX_INDEX);
    if value > MAX_SCAN_INDEX {
        return Err(ServiceError::bad_request(format!(
            "max_index must be <= {MAX_SCAN_INDEX}"
        )));
    }
    Ok(value)
}

pub(super) fn normalized_wallet_family(value: Option<&str>) -> ServiceResult<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(WALLET_FAMILY_ETH_SEED) => Ok(Some(WALLET_FAMILY_ETH_SEED.into())),
        Some(WALLET_FAMILY_ETH_XPUB) => Ok(Some(WALLET_FAMILY_ETH_XPUB.into())),
        Some(_) => Err(ServiceError::bad_request(
            "wallet_family must be 'eth-seed' or 'eth-xpub'",
        )),
    }
}

pub(super) fn select_providers(
    providers: &[EvmProviderProfile],
    requested_profile: Option<&str>,
) -> ServiceResult<Vec<EvmProviderProfile>> {
    let selected = providers
        .iter()
        .filter(|provider| requested_profile.is_none_or(|name| name == provider.name))
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(ServiceError::not_found(
            "No matching EVM provider profiles found.",
        ));
    }
    Ok(selected)
}

pub(super) fn address_record(
    context: &InventoryRecordContext<'_>,
    address_index: u32,
    activity_state: &str,
    native_balance_wei_hex: &str,
    transaction_count: u64,
) -> WalletInventoryAddress {
    WalletInventoryAddress {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        address_index,
        activity_state: activity_state.to_string(),
        native_balance_wei_hex: native_balance_wei_hex.to_string(),
        transaction_count,
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        first_seen_at_unix: context.now,
        last_checked_at_unix: context.now,
    }
}

pub(super) fn holding_record(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    amount_hex: &str,
) -> WalletAssetHolding {
    holding_record_with_source(
        context,
        asset_kind,
        asset_address,
        amount_hex,
        DISCOVERY_SOURCE_LOCAL_RPC,
    )
}

pub(super) fn holding_record_with_source(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_with_counterparty(context, asset_kind, asset_address, None, amount_hex, source)
}

pub(super) fn holding_record_with_token_id(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    token_id_hex: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        asset_kind,
        asset_address,
        token_id_hex,
        None,
        amount_hex,
        source,
    )
}

pub(super) fn holding_record_with_counterparty(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    counterparty_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        asset_kind,
        asset_address,
        None,
        counterparty_address,
        amount_hex,
        source,
    )
}

fn holding_record_full(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    token_id_hex: Option<String>,
    counterparty_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    WalletAssetHolding {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        asset_kind: asset_kind.to_string(),
        asset_address,
        token_id_hex,
        counterparty_address,
        amount_hex: amount_hex.to_string(),
        source: source.into(),
        status: if quantity_hex_is_nonzero(amount_hex) {
            "detected".into()
        } else {
            "not_detected".into()
        },
        first_seen_at_unix: context.now,
        last_checked_at_unix: context.now,
    }
}

pub(super) fn upsert_address(
    addresses: &mut Vec<WalletInventoryAddress>,
    mut next: WalletInventoryAddress,
) {
    if let Some(existing) = addresses.iter_mut().find(|existing| {
        existing.wallet_family == next.wallet_family
            && existing.wallet_profile == next.wallet_profile
            && existing.provider_profile == next.provider_profile
            && existing.chain_id == next.chain_id
            && existing.address == next.address
    }) {
        next.id = existing.id.clone();
        next.first_seen_at_unix = existing.first_seen_at_unix;
        *existing = next;
    } else {
        addresses.push(next);
    }
}

pub(super) fn upsert_holding(holdings: &mut Vec<WalletAssetHolding>, mut next: WalletAssetHolding) {
    if let Some(existing) = holdings
        .iter_mut()
        .find(|existing| holding_key_matches(existing, &next))
    {
        next.id = existing.id.clone();
        next.first_seen_at_unix = existing.first_seen_at_unix;
        *existing = next;
    } else {
        holdings.push(next);
    }
}

pub(super) fn remove_holding(holdings: &mut Vec<WalletAssetHolding>, target: &WalletAssetHolding) {
    holdings.retain(|existing| !holding_key_matches(existing, target));
}

fn holding_key_matches(left: &WalletAssetHolding, right: &WalletAssetHolding) -> bool {
    left.wallet_family == right.wallet_family
        && left.wallet_profile == right.wallet_profile
        && left.provider_profile == right.provider_profile
        && left.chain_id == right.chain_id
        && left.address == right.address
        && left.asset_kind == right.asset_kind
        && left.asset_address == right.asset_address
        && left.token_id_hex == right.token_id_hex
        && left.counterparty_address == right.counterparty_address
}

pub(super) fn quantity_hex_is_nonzero(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
        .bytes()
        .any(|byte| byte != b'0')
}

pub(super) fn unique_strings(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}
