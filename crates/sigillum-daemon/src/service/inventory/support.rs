use std::collections::BTreeMap;

use sigillum_api::{
    EthStealthDeposit, EvmProviderProfile, NftMetadataCacheEntry, RiskCatalogEntry,
    WalletAddressActivityState, WalletAssetHolding, WalletAssetKind, WalletDiscoveryJob,
    WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::service::helpers::random_id;
use crate::service::{ServiceError, ServiceResult};

use super::{
    DEFAULT_GAP_LIMIT, DEFAULT_MAX_INDEX, DISCOVERY_SOURCE_LOCAL_RPC, DiscoveryWallet,
    MAX_GAP_LIMIT, MAX_SCAN_INDEX, WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH,
    WALLET_FAMILY_ETH_XPUB,
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

#[derive(Clone, Debug, Default)]
pub(super) struct ClaimRecordMetadata {
    pub(super) adapter: Option<String>,
    pub(super) index_hex: Option<String>,
    pub(super) proof: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NftSpamAssessment {
    pub(super) label: String,
    pub(super) reasons: Vec<String>,
}

struct HoldingRecordParts<'a> {
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    token_id_hex: Option<String>,
    counterparty_address: Option<String>,
    protocol_address: Option<String>,
    claim: ClaimRecordMetadata,
    amount_hex: &'a str,
    source: &'a str,
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

pub(super) fn record_inventory_observation(
    job: &mut WalletDiscoveryJob,
    inventory: &mut crate::inventory::WalletInventoryState,
    observation: InventoryAddressObservation,
    detected_holdings: &mut Vec<WalletAssetHolding>,
    scanned_addresses: &mut Vec<WalletInventoryAddress>,
) {
    job.addresses_scanned += 1;
    if observation.address.activity_state != WalletAddressActivityState::Empty {
        job.active_addresses += 1;
    }
    for holding in &observation.holdings {
        if quantity_hex_is_nonzero(&holding.amount_hex) {
            job.holdings_detected += 1;
        }
    }

    upsert_address(&mut inventory.addresses, observation.address.clone());
    for mut holding in observation.holdings {
        let spam_assessment = conservative_nft_spam_label(
            &holding,
            &inventory.addresses,
            &inventory.holdings,
            &inventory.risk_catalog,
        );
        if let Some(assessment) = spam_assessment {
            holding.spam_label = Some(assessment.label.clone());
            upsert_nft_metadata_cache(
                &mut inventory.nft_metadata_cache,
                &holding,
                &assessment.label,
                &assessment.reasons,
            );
        }
        if quantity_hex_is_nonzero(&holding.amount_hex) {
            upsert_holding(&mut inventory.holdings, holding.clone());
            detected_holdings.push(holding);
        } else {
            remove_holding(&mut inventory.holdings, &holding);
        }
    }
    scanned_addresses.push(observation.address);
}

pub(super) fn conservative_nft_spam_label(
    holding: &WalletAssetHolding,
    addresses: &[WalletInventoryAddress],
    holdings: &[WalletAssetHolding],
    risk_catalog: &[RiskCatalogEntry],
) -> Option<NftSpamAssessment> {
    if !matches!(
        holding.asset_kind,
        WalletAssetKind::Erc721 | WalletAssetKind::Erc1155 | WalletAssetKind::Nft
    ) {
        return None;
    }
    if !quantity_hex_is_nonzero(&holding.amount_hex) {
        return None;
    }

    if let Some(contract_address) = holding.asset_address.as_deref() {
        if let Some(entry) = risk_catalog
            .iter()
            .find(|entry| entry.address.eq_ignore_ascii_case(contract_address))
        {
            if entry.risk_level.eq_ignore_ascii_case("trusted") {
                return Some(NftSpamAssessment {
                    label: "operator_trusted".into(),
                    reasons: vec![format!("operator_override:trusted:{}", entry.label)],
                });
            }
            if entry.risk_level.eq_ignore_ascii_case("high")
                || entry.risk_level.eq_ignore_ascii_case("critical")
            {
                return Some(NftSpamAssessment {
                    label: "operator_flagged_spam".into(),
                    reasons: vec![format!(
                        "operator_override:{}:{}",
                        entry.risk_level.to_ascii_lowercase(),
                        entry.label
                    )],
                });
            }
        }

        if let Some(metadata_name) = holding.metadata_name.as_deref() {
            let normalized_name = normalized_spam_match_name(metadata_name);
            if !normalized_name.is_empty() {
                if let Some(entry) = risk_catalog.iter().find(|entry| {
                    entry.risk_level.eq_ignore_ascii_case("trusted")
                        && !entry.address.eq_ignore_ascii_case(contract_address)
                        && normalized_spam_match_name(&entry.label) == normalized_name
                }) {
                    return Some(NftSpamAssessment {
                        label: "suspected_lookalike".into(),
                        reasons: vec![format!("name_lookalike_of_trusted:{}", entry.label)],
                    });
                }
            }
        }

        let received_without_outbound_activity = addresses.iter().any(|address| {
            address.wallet_family == holding.wallet_family
                && address.wallet_profile == holding.wallet_profile
                && address.provider_profile == holding.provider_profile
                && address.chain_id == holding.chain_id
                && address.address.eq_ignore_ascii_case(&holding.address)
                && address.transaction_count == 0
        });
        let no_matching_operator_approval = !holdings.iter().any(|existing| {
            existing.asset_kind == WalletAssetKind::Approval
                && existing.chain_id == holding.chain_id
                && existing.address.eq_ignore_ascii_case(&holding.address)
                && existing
                    .asset_address
                    .as_deref()
                    .is_some_and(|address| address.eq_ignore_ascii_case(contract_address))
        });
        if received_without_outbound_activity && no_matching_operator_approval {
            return Some(NftSpamAssessment {
                label: "suspected_airdrop".into(),
                reasons: vec![
                    "received_without_outbound_activity".into(),
                    "no_matching_operator_approval".into(),
                ],
            });
        }
    }

    Some(NftSpamAssessment {
        label: "unverified_nft_metadata".into(),
        reasons: vec!["metadata_not_verified_locally".into()],
    })
}

pub(super) fn upsert_nft_metadata_cache(
    cache: &mut Vec<NftMetadataCacheEntry>,
    holding: &WalletAssetHolding,
    spam_label: &str,
    spam_reasons: &[String],
) {
    let (Some(contract_address), Some(token_id_hex)) = (
        holding.asset_address.as_ref(),
        holding.token_id_hex.as_ref(),
    ) else {
        return;
    };
    let mut next = NftMetadataCacheEntry {
        chain_id: holding.chain_id,
        contract_address: contract_address.clone(),
        token_id_hex: token_id_hex.clone(),
        metadata_uri: holding.metadata_uri.clone(),
        name: holding.metadata_name.clone(),
        spam_label: spam_label.to_string(),
        spam_reasons: spam_reasons.to_vec(),
        fetched_at_unix: None,
        fetched_uri: None,
        content_sha256: None,
        fetch_skipped_reason: None,
        updated_at_unix: holding.last_checked_at_unix,
    };
    if let Some(existing) = cache.iter_mut().find(|existing| {
        existing.chain_id == next.chain_id
            && existing
                .contract_address
                .eq_ignore_ascii_case(&next.contract_address)
            && existing
                .token_id_hex
                .eq_ignore_ascii_case(&next.token_id_hex)
    }) {
        if existing.fetched_at_unix.is_some() {
            if next.metadata_uri.is_none() {
                next.metadata_uri = existing.metadata_uri.clone();
            }
            if next.name.is_none() {
                next.name = existing.name.clone();
            }
            next.fetched_at_unix = existing.fetched_at_unix;
            next.fetched_uri = existing.fetched_uri.clone();
            next.content_sha256 = existing.content_sha256.clone();
        }
        next.fetch_skipped_reason = existing.fetch_skipped_reason.clone();
        *existing = next;
    } else {
        cache.push(next);
    }
}

fn normalized_spam_match_name(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
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
        Some(WALLET_FAMILY_ETH_WATCH) => Ok(Some(WALLET_FAMILY_ETH_WATCH.into())),
        Some(_) => Err(ServiceError::bad_request(
            "wallet_family must be 'eth-seed', 'eth-xpub', or 'eth-watch'",
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
    activity_state: WalletAddressActivityState,
    native_balance_wei_hex: &str,
    transaction_count: u64,
    last_activity_block: Option<u64>,
    classifications: Vec<sigillum_api::WalletAddressClassification>,
) -> WalletInventoryAddress {
    WalletInventoryAddress {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        derivation_pattern: Some(context.wallet.derivation_pattern.clone()),
        account_index: Some(context.wallet.account_index),
        address_index,
        activity_state,
        native_balance_wei_hex: native_balance_wei_hex.to_string(),
        transaction_count,
        last_activity_block,
        classifications,
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        first_seen_at_unix: context.now,
        last_checked_at_unix: context.now,
    }
}

pub(super) fn holding_record(
    context: &InventoryRecordContext<'_>,
    asset_kind: WalletAssetKind,
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
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_with_counterparty(context, asset_kind, asset_address, None, amount_hex, source)
}

pub(super) fn holding_record_with_token_id(
    context: &InventoryRecordContext<'_>,
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    token_id_hex: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        HoldingRecordParts {
            asset_kind,
            asset_address,
            token_id_hex,
            counterparty_address: None,
            protocol_address: None,
            claim: ClaimRecordMetadata::default(),
            amount_hex,
            source,
        },
    )
}

pub(super) fn holding_record_with_counterparty(
    context: &InventoryRecordContext<'_>,
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    counterparty_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        HoldingRecordParts {
            asset_kind,
            asset_address,
            token_id_hex: None,
            counterparty_address,
            protocol_address: None,
            claim: ClaimRecordMetadata::default(),
            amount_hex,
            source,
        },
    )
}

pub(super) fn holding_record_with_protocol_counterparty(
    context: &InventoryRecordContext<'_>,
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    protocol_address: Option<String>,
    counterparty_address: Option<String>,
    amount_hex: &str,
    source: &str,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        HoldingRecordParts {
            asset_kind,
            asset_address,
            token_id_hex: None,
            counterparty_address,
            protocol_address,
            claim: ClaimRecordMetadata::default(),
            amount_hex,
            source,
        },
    )
}

pub(super) fn holding_record_with_claim_metadata(
    context: &InventoryRecordContext<'_>,
    asset_kind: WalletAssetKind,
    asset_address: Option<String>,
    protocol_address: Option<String>,
    amount_hex: &str,
    source: &str,
    claim: ClaimRecordMetadata,
) -> WalletAssetHolding {
    holding_record_full(
        context,
        HoldingRecordParts {
            asset_kind,
            asset_address,
            token_id_hex: None,
            counterparty_address: None,
            protocol_address,
            claim,
            amount_hex,
            source,
        },
    )
}

fn holding_record_full(
    context: &InventoryRecordContext<'_>,
    parts: HoldingRecordParts<'_>,
) -> WalletAssetHolding {
    WalletAssetHolding {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        asset_kind: parts.asset_kind,
        asset_address: parts.asset_address,
        token_id_hex: parts.token_id_hex,
        counterparty_address: parts.counterparty_address,
        protocol_address: parts.protocol_address,
        claim_adapter: parts.claim.adapter,
        claim_index_hex: parts.claim.index_hex,
        claim_proof: parts.claim.proof,
        metadata_uri: None,
        metadata_name: None,
        spam_label: None,
        amount_hex: parts.amount_hex.to_string(),
        source: parts.source.into(),
        status: if quantity_hex_is_nonzero(parts.amount_hex) {
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
        next.last_activity_block = next.last_activity_block.max(existing.last_activity_block);
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
        && left.claim_adapter == right.claim_adapter
        && left.claim_index_hex == right.claim_index_hex
        && left.claim_proof == right.claim_proof
        && approval_source_key_matches(left, right)
        && protocol_address_key_matches(left, right)
}

fn approval_source_key_matches(left: &WalletAssetHolding, right: &WalletAssetHolding) -> bool {
    left.asset_kind != WalletAssetKind::Approval || left.source == right.source
}

fn protocol_address_key_matches(left: &WalletAssetHolding, right: &WalletAssetHolding) -> bool {
    if left.protocol_address == right.protocol_address {
        return true;
    }
    left.source == "permit2-allowance-probe"
        && right.source == "permit2-allowance-probe"
        && (left.protocol_address.is_none() || right.protocol_address.is_none())
}

/// Flags effectively unlimited approvals, including uint160-max Permit2 allowances, not only
/// uint256-max approvals.
pub(super) fn is_very_large_approval(amount_hex: &str) -> bool {
    decode_quantity_hex(amount_hex)
        .map(|bytes| bytes[..16].iter().any(|byte| *byte != 0))
        .unwrap_or(false)
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

pub(super) fn unique_u64s(values: impl Iterator<Item = u64>) -> Vec<u64> {
    let mut out = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

pub(super) fn announcement_activity_blocks(
    deposits: &[EthStealthDeposit],
) -> BTreeMap<(u64, String), u64> {
    let mut blocks: BTreeMap<(u64, String), u64> = BTreeMap::new();
    for deposit in deposits {
        let Some(note) = deposit.note.as_deref() else {
            continue;
        };
        if !note.starts_with("erc5564-announcement") {
            continue;
        }
        let Some(block) = note
            .split("; ")
            .find_map(|part| part.strip_prefix("block="))
            .and_then(parse_hex_u64)
        else {
            continue;
        };
        blocks
            .entry((
                deposit.chain_id,
                deposit.stealth_address.to_ascii_lowercase(),
            ))
            .and_modify(|existing| *existing = (*existing).max(block))
            .or_insert(block);
    }
    blocks
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CONTRACT: &str = "0x1111111111111111111111111111111111111111";
    const TRUSTED: &str = "0xdead000000000000000000000000000000000000";

    #[test]
    fn very_large_approval_threshold_is_inclusive_at_two_to_the_128() {
        let cases = [
            ("2^128 - 1", "0xffffffffffffffffffffffffffffffff", false),
            ("2^128", "0x100000000000000000000000000000000", true),
            (
                "Permit2 maximum allowance (2^160 - 1)",
                "0xffffffffffffffffffffffffffffffffffffffff",
                true,
            ),
            (
                "2^255",
                "0x8000000000000000000000000000000000000000000000000000000000000000",
                true,
            ),
            (
                "2^256 - 1",
                "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                true,
            ),
        ];

        for (label, amount_hex, expected) in cases {
            assert_eq!(
                is_very_large_approval(amount_hex),
                expected,
                "{label} classification"
            );
        }
    }

    #[test]
    fn airdropped_mock_collection_is_flagged_with_both_reasons() {
        let holding = nft_holding(CONTRACT, None);
        let addresses = vec![owner_address(0)];

        let assessment =
            conservative_nft_spam_label(&holding, &addresses, &[], &[]).expect("assessment");

        assert_eq!(assessment.label, "suspected_airdrop");
        assert_eq!(
            assessment.reasons,
            vec![
                "received_without_outbound_activity".to_string(),
                "no_matching_operator_approval".to_string()
            ]
        );
    }

    #[test]
    fn owner_with_transaction_count_is_not_airdrop_flagged() {
        let holding = nft_holding(CONTRACT, None);
        let addresses = vec![owner_address(1)];

        let assessment =
            conservative_nft_spam_label(&holding, &addresses, &[], &[]).expect("assessment");

        assert_eq!(assessment.label, "unverified_nft_metadata");
        assert_eq!(
            assessment.reasons,
            vec!["metadata_not_verified_locally".to_string()]
        );
    }

    #[test]
    fn lookalike_of_trusted_catalog_entry_is_flagged() {
        let holding = nft_holding(CONTRACT, Some("Trusted Collection!"));
        let catalog = vec![risk_entry(TRUSTED, "Trusted Collection", "trusted")];

        let assessment =
            conservative_nft_spam_label(&holding, &[], &[], &catalog).expect("assessment");

        assert_eq!(assessment.label, "suspected_lookalike");
        assert_eq!(
            assessment.reasons,
            vec!["name_lookalike_of_trusted:Trusted Collection".to_string()]
        );
    }

    #[test]
    fn same_address_trusted_entry_is_operator_trusted() {
        let holding = nft_holding(CONTRACT, Some("Trusted Collection"));
        let catalog = vec![risk_entry(CONTRACT, "Trusted Collection", "trusted")];

        let assessment =
            conservative_nft_spam_label(&holding, &[], &[], &catalog).expect("assessment");

        assert_eq!(assessment.label, "operator_trusted");
        assert_eq!(
            assessment.reasons,
            vec!["operator_override:trusted:Trusted Collection".to_string()]
        );
    }

    #[test]
    fn high_catalog_entry_is_operator_flagged_spam() {
        let holding = nft_holding(CONTRACT, None);
        let catalog = vec![risk_entry(CONTRACT, "Known Spam", "high")];

        let assessment =
            conservative_nft_spam_label(&holding, &[], &[], &catalog).expect("assessment");

        assert_eq!(assessment.label, "operator_flagged_spam");
        assert_eq!(
            assessment.reasons,
            vec!["operator_override:high:Known Spam".to_string()]
        );
    }

    fn nft_holding(contract: &str, name: Option<&str>) -> WalletAssetHolding {
        WalletAssetHolding {
            id: "holding-1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "default".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: OWNER.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: WalletAssetKind::Erc721,
            asset_address: Some(contract.into()),
            token_id_hex: Some("0x1".into()),
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            metadata_uri: None,
            metadata_name: name.map(str::to_string),
            spam_label: None,
            amount_hex: "0x1".into(),
            source: "local-rpc".into(),
            status: "detected".into(),
            first_seen_at_unix: 100,
            last_checked_at_unix: 100,
        }
    }

    fn owner_address(transaction_count: u64) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: "address-1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "default".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: OWNER.into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: None,
            account_index: Some(0),
            address_index: 0,
            activity_state: WalletAddressActivityState::Active,
            native_balance_wei_hex: "0x0".into(),
            transaction_count,
            last_activity_block: None,
            classifications: Vec::new(),
            source: "local-rpc".into(),
            first_seen_at_unix: 100,
            last_checked_at_unix: 100,
        }
    }

    fn risk_entry(address: &str, label: &str, risk_level: &str) -> RiskCatalogEntry {
        RiskCatalogEntry {
            address: address.into(),
            label: label.into(),
            risk_level: risk_level.into(),
            source: "operator".into(),
            notes: Vec::new(),
            created_at_unix: 100,
            updated_at_unix: 100,
        }
    }

    fn deposit(stealth_address: &str, note: Option<&str>) -> EthStealthDeposit {
        EthStealthDeposit {
            id: format!("dep_{stealth_address}"),
            status: "pending".into(),
            asset_kind: "native".into(),
            wallet_profile: "wallet-a".into(),
            chain_id: 1,
            chain_id_assumed: false,
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: "wallet-a".into(),
            short_name: "eth".into(),
            stealth_meta_address: "st:eth:example".into(),
            stealth_address: stealth_address.into(),
            ephemeral_public_key_hex: "0x02".into(),
            view_tag_hex: "0xaa".into(),
            announcement: None,
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: None,
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: false,
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            queue_job_id: None,
            queue_job_state: None,
            note: note.map(str::to_string),
            created_at_unix: 1,
            updated_at_unix: 1,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
            counterparty_id: None,
        }
    }

    #[test]
    fn announcement_activity_blocks_parse_notes_and_keep_max() {
        let deposits = vec![
            deposit(
                "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                Some("erc5564-announcement; block=0x10; tx=0xabc"),
            ),
            deposit(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("erc5564-announcement; block=0x2a; tx=0xdef"),
            ),
        ];

        let blocks = announcement_activity_blocks(&deposits);

        assert_eq!(
            blocks.get(&(1, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into())),
            Some(&42)
        );
    }

    #[test]
    fn announcement_activity_blocks_ignore_missing_or_malformed_block() {
        let deposits = vec![
            deposit(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                Some("erc5564-announcement; tx=0xabc"),
            ),
            deposit(
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                Some("erc5564-announcement; block=not-hex"),
            ),
            deposit(
                "0xcccccccccccccccccccccccccccccccccccccccc",
                Some("operator-note; block=0x10"),
            ),
            deposit("0xdddddddddddddddddddddddddddddddddddddddd", None),
        ];

        assert!(announcement_activity_blocks(&deposits).is_empty());
    }
}
