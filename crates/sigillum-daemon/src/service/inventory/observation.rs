use sigillum_api::{
    EvmProviderProfile, WalletAddressActivityState, WalletAddressClassification, WalletAssetKind,
    WalletDiscoveryBlockCursor,
};

use crate::service::evm::normalize_address;
use crate::service::{ServiceResult, SigillumService};

use super::allowance_discovery::{
    DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE, Erc20AllowanceDiscoveryConfig,
};
use super::checkpoints::{
    BlockCursorProgress, TOPIC_FAMILY_ERC20_TRANSFER, TOPIC_FAMILY_ERC721_TRANSFER,
    TOPIC_FAMILY_ERC1155_TRANSFER, effective_from_block, latest_cursor_block, update_block_cursor,
};
use super::claim_discovery::{ClaimCandidateDiscoveryConfig, claim_candidate_source};
use super::defi_adapters::adapter_for_protocol;
use super::defi_discovery::{DefiTokenPositionDiscoveryConfig, defi_token_probe_source};
use super::nft_approval_discovery::{
    DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE, NftOperatorApprovalDiscoveryConfig,
};
use super::nft_discovery::{
    DISCOVERY_SOURCE_ERC721_TRANSFER_LOG, DISCOVERY_SOURCE_ERC1155_TRANSFER_LOG,
    Erc721TransferDiscoveryConfig, Erc1155TransferDiscoveryConfig,
};
use super::permit2_discovery::{
    DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE, Permit2AllowanceDiscoveryConfig,
};
use super::support::{
    ClaimRecordMetadata, InventoryAddressObservation, InventoryRecordContext, address_record,
    holding_record, holding_record_with_claim_metadata, holding_record_with_counterparty,
    holding_record_with_protocol_counterparty, holding_record_with_source,
    holding_record_with_token_id, quantity_hex_is_nonzero,
};
use super::token_discovery::{
    DISCOVERY_SOURCE_ERC20_TRANSFER_LOG, Erc20TransferDiscoveryConfig, push_unique_token,
};
use super::{
    DISCOVERY_SOURCE_LOCAL_RPC, DiscoveryWallet, WALLET_FAMILY_ETH_SEED, WALLET_FAMILY_ETH_WATCH,
    WALLET_FAMILY_ETH_XPUB,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct AddressActivityContext {
    pub(super) prior_last_activity_block: Option<u64>,
    pub(super) announcement_activity_block: Option<u64>,
    pub(super) chain_tip_block: Option<u64>,
    pub(super) dormancy_block_window: u64,
}

impl SigillumService {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn observe_inventory_address(
        &self,
        wallet: &DiscoveryWallet,
        provider: &EvmProviderProfile,
        address: &str,
        derivation_path: &str,
        address_index: u32,
        block_tag: &str,
        token_addresses: &[String],
        token_discovery: Option<&Erc20TransferDiscoveryConfig>,
        allowance_discovery: Option<&Erc20AllowanceDiscoveryConfig>,
        permit2_allowance_discovery: Option<&Permit2AllowanceDiscoveryConfig>,
        nft_discovery: Option<&Erc721TransferDiscoveryConfig>,
        erc1155_discovery: Option<&Erc1155TransferDiscoveryConfig>,
        nft_operator_approval_discovery: Option<&NftOperatorApprovalDiscoveryConfig>,
        defi_position_discovery: Option<&DefiTokenPositionDiscoveryConfig>,
        claim_candidate_discovery: Option<&ClaimCandidateDiscoveryConfig>,
        activity: AddressActivityContext,
        block_cursors: &mut Vec<WalletDiscoveryBlockCursor>,
        now: u64,
    ) -> ServiceResult<InventoryAddressObservation> {
        let address = normalize_address(address)?;
        let native_balance_wei_hex = self
            .evm_native_balance_for_provider(provider.compartment_id, provider, &address, block_tag)
            .await?;
        let transaction_count = self
            .evm_transaction_count_for_provider(
                provider.compartment_id,
                provider,
                &address,
                block_tag,
            )
            .await?;
        let mut activity_state = if quantity_hex_is_nonzero(&native_balance_wei_hex) {
            WalletAddressActivityState::Funded
        } else if transaction_count > 0 {
            WalletAddressActivityState::Active
        } else {
            WalletAddressActivityState::Empty
        };

        let record_context = InventoryRecordContext {
            wallet,
            provider,
            address: &address,
            derivation_path,
            now,
        };
        let mut observed_activity_block = None;
        let mut holdings = vec![holding_record(
            &record_context,
            WalletAssetKind::Native,
            None,
            &native_balance_wei_hex,
        )];

        let mut observed_token_addresses = token_addresses.to_vec();
        let mut transfer_log_tokens = Vec::new();
        if let Some(config) = token_discovery {
            let from_block = effective_from_block(
                &config.from_block,
                latest_cursor_block(
                    block_cursors,
                    &address,
                    provider.chain_id,
                    TOPIC_FAMILY_ERC20_TRANSFER,
                ),
            );
            let (tokens, scanned_to_block) = self
                .discover_erc20_transfer_tokens_for_address(provider, &address, config, &from_block)
                .await?;
            transfer_log_tokens = tokens;
            if !transfer_log_tokens.is_empty() {
                observed_activity_block =
                    max_optional_block(observed_activity_block, scanned_to_block);
            }
            if let Some(scanned_to_block) = scanned_to_block {
                update_block_cursor(
                    block_cursors,
                    BlockCursorProgress {
                        address: &address,
                        chain_id: provider.chain_id,
                        topic_family: TOPIC_FAMILY_ERC20_TRANSFER,
                        last_scanned_block: scanned_to_block,
                        updated_at_unix: now,
                    },
                );
            }
            if !transfer_log_tokens.is_empty()
                && activity_state == WalletAddressActivityState::Empty
            {
                activity_state = WalletAddressActivityState::Active;
            }
            for token_address in transfer_log_tokens.iter().cloned() {
                push_unique_token(&mut observed_token_addresses, token_address);
            }
        }

        for token_address in &observed_token_addresses {
            let amount_hex = self
                .evm_erc20_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    token_address,
                    &address,
                    block_tag,
                )
                .await?;
            if quantity_hex_is_nonzero(&amount_hex) {
                activity_state = WalletAddressActivityState::Funded;
            }
            holdings.push(holding_record_with_source(
                &record_context,
                WalletAssetKind::Erc20,
                Some(token_address.clone()),
                &amount_hex,
                source_for_token(&transfer_log_tokens, token_address),
            ));
        }

        if let Some(config) = allowance_discovery {
            let allowances = self
                .discover_erc20_allowances_for_address(
                    provider,
                    &address,
                    &observed_token_addresses,
                    block_tag,
                    config,
                )
                .await?;
            for allowance in allowances {
                holdings.push(holding_record_with_counterparty(
                    &record_context,
                    WalletAssetKind::Approval,
                    Some(allowance.token_address),
                    Some(allowance.spender_address),
                    &allowance.amount_hex,
                    DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE,
                ));
            }
        }

        if let Some(config) = permit2_allowance_discovery {
            let allowances = self
                .discover_permit2_allowances_for_address(
                    provider,
                    &address,
                    &observed_token_addresses,
                    block_tag,
                    config,
                )
                .await?;
            for allowance in allowances {
                holdings.push(holding_record_with_protocol_counterparty(
                    &record_context,
                    WalletAssetKind::Approval,
                    Some(allowance.token_address),
                    Some(allowance.permit2_address),
                    Some(allowance.spender_address),
                    &allowance.amount_hex,
                    DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE,
                ));
            }
        }

        let mut observed_nft_contract_addresses = Vec::new();
        if let Some(config) = nft_discovery {
            let from_block = effective_from_block(
                &config.from_block,
                latest_cursor_block(
                    block_cursors,
                    &address,
                    provider.chain_id,
                    TOPIC_FAMILY_ERC721_TRANSFER,
                ),
            );
            let (nfts, scanned_to_block) = self
                .discover_erc721_transfer_holdings_for_address(
                    provider,
                    &address,
                    config,
                    &from_block,
                )
                .await?;
            let has_observed_nfts = !nfts.is_empty();
            if has_observed_nfts {
                observed_activity_block =
                    max_optional_block(observed_activity_block, scanned_to_block);
            }
            if let Some(scanned_to_block) = scanned_to_block {
                update_block_cursor(
                    block_cursors,
                    BlockCursorProgress {
                        address: &address,
                        chain_id: provider.chain_id,
                        topic_family: TOPIC_FAMILY_ERC721_TRANSFER,
                        last_scanned_block: scanned_to_block,
                        updated_at_unix: now,
                    },
                );
            }
            if has_observed_nfts {
                activity_state = WalletAddressActivityState::Funded;
            }
            for nft in nfts {
                let contract_address = nft.contract_address;
                push_unique_contract(
                    &mut observed_nft_contract_addresses,
                    contract_address.clone(),
                );
                holdings.push(holding_record_with_token_id(
                    &record_context,
                    WalletAssetKind::Erc721,
                    Some(contract_address),
                    Some(nft.token_id_hex),
                    "0x1",
                    DISCOVERY_SOURCE_ERC721_TRANSFER_LOG,
                ));
            }
        }

        if let Some(config) = erc1155_discovery {
            let from_block = effective_from_block(
                &config.from_block,
                latest_cursor_block(
                    block_cursors,
                    &address,
                    provider.chain_id,
                    TOPIC_FAMILY_ERC1155_TRANSFER,
                ),
            );
            let (tokens, scanned_to_block) = self
                .discover_erc1155_transfer_holdings_for_address(
                    provider,
                    &address,
                    config,
                    &from_block,
                )
                .await?;
            let has_observed_tokens = !tokens.is_empty();
            if has_observed_tokens {
                observed_activity_block =
                    max_optional_block(observed_activity_block, scanned_to_block);
            }
            if let Some(scanned_to_block) = scanned_to_block {
                update_block_cursor(
                    block_cursors,
                    BlockCursorProgress {
                        address: &address,
                        chain_id: provider.chain_id,
                        topic_family: TOPIC_FAMILY_ERC1155_TRANSFER,
                        last_scanned_block: scanned_to_block,
                        updated_at_unix: now,
                    },
                );
            }
            if has_observed_tokens {
                activity_state = WalletAddressActivityState::Funded;
            }
            for token in tokens {
                let contract_address = token.contract_address;
                push_unique_contract(
                    &mut observed_nft_contract_addresses,
                    contract_address.clone(),
                );
                holdings.push(holding_record_with_token_id(
                    &record_context,
                    WalletAssetKind::Erc1155,
                    Some(contract_address),
                    Some(token.token_id_hex),
                    &token.amount_hex,
                    DISCOVERY_SOURCE_ERC1155_TRANSFER_LOG,
                ));
            }
        }

        if let Some(config) = nft_operator_approval_discovery {
            let approvals = self
                .discover_nft_operator_approvals_for_address(
                    provider,
                    &address,
                    &observed_nft_contract_addresses,
                    block_tag,
                    config,
                )
                .await?;
            for approval in approvals {
                holdings.push(holding_record_with_counterparty(
                    &record_context,
                    WalletAssetKind::Approval,
                    Some(approval.contract_address),
                    Some(approval.operator_address),
                    &approval.amount_hex,
                    DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE,
                ));
            }
        }

        if let Some(config) = defi_position_discovery {
            let positions = self
                .discover_defi_token_positions_for_address(provider, &address, block_tag, config)
                .await?;
            for position in positions {
                if quantity_hex_is_nonzero(&position.amount_hex) {
                    activity_state = WalletAddressActivityState::Funded;
                }
                let mut holding = holding_record_with_protocol_counterparty(
                    &record_context,
                    WalletAssetKind::Defi,
                    Some(position.token_address),
                    position.protocol_address,
                    None,
                    &position.amount_hex,
                    &defi_token_probe_source(&position.protocol),
                );
                holding.claim_adapter =
                    adapter_for_protocol(&position.protocol).map(str::to_string);
                holdings.push(holding);
            }
        }

        if let Some(config) = claim_candidate_discovery {
            for candidate in config.candidates_for_address(&address) {
                activity_state = WalletAddressActivityState::Funded;
                holdings.push(holding_record_with_claim_metadata(
                    &record_context,
                    WalletAssetKind::from(candidate.kind.as_str()),
                    Some(candidate.asset_address),
                    Some(candidate.claim_contract_address),
                    &candidate.amount_hex,
                    &claim_candidate_source(
                        &candidate.kind,
                        &candidate.protocol,
                        &candidate.source_label,
                    ),
                    ClaimRecordMetadata {
                        adapter: candidate.claim_adapter,
                        index_hex: candidate.claim_index_hex,
                        proof: candidate.claim_proof,
                    },
                ));
            }
        }

        let last_activity_block = max_optional_block(
            max_optional_block(observed_activity_block, activity.prior_last_activity_block),
            activity.announcement_activity_block,
        );
        Ok(InventoryAddressObservation {
            address: address_record(
                &record_context,
                address_index,
                activity_state,
                &native_balance_wei_hex,
                transaction_count,
                last_activity_block,
                address_classifications(
                    wallet,
                    &native_balance_wei_hex,
                    transaction_count,
                    &holdings,
                    last_activity_block,
                    activity.chain_tip_block,
                    activity.dormancy_block_window,
                ),
            ),
            holdings,
        })
    }
}

fn source_for_token(discovered_tokens: &[String], token_address: &str) -> &'static str {
    if discovered_tokens
        .iter()
        .any(|discovered| discovered.eq_ignore_ascii_case(token_address))
    {
        DISCOVERY_SOURCE_ERC20_TRANSFER_LOG
    } else {
        DISCOVERY_SOURCE_LOCAL_RPC
    }
}

fn push_unique_contract(contracts: &mut Vec<String>, contract_address: String) {
    if !contracts
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&contract_address))
    {
        contracts.push(contract_address);
    }
}

fn address_classifications(
    wallet: &DiscoveryWallet,
    native_balance_wei_hex: &str,
    transaction_count: u64,
    holdings: &[sigillum_api::WalletAssetHolding],
    last_activity_block: Option<u64>,
    chain_tip_block: Option<u64>,
    dormancy_block_window: u64,
) -> Vec<WalletAddressClassification> {
    let mut classifications = Vec::new();
    match wallet.family.as_str() {
        WALLET_FAMILY_ETH_SEED => push_classification(
            &mut classifications,
            WalletAddressClassification::SignerAvailable,
        ),
        WALLET_FAMILY_ETH_XPUB | WALLET_FAMILY_ETH_WATCH => {
            push_classification(&mut classifications, WalletAddressClassification::WatchOnly);
        }
        _ => push_classification(
            &mut classifications,
            WalletAddressClassification::SignerUnknown,
        ),
    }

    let has_native_gas = quantity_hex_is_nonzero(native_balance_wei_hex);
    if has_native_gas {
        push_classification(
            &mut classifications,
            WalletAddressClassification::GasAvailable,
        );
    }
    if transaction_count > 0 {
        push_classification(
            &mut classifications,
            WalletAddressClassification::TransactionHistory,
        );
    }

    let mut has_value = has_native_gas;
    let mut has_non_native_value = false;
    let mut has_approval_exposure = false;
    for holding in holdings {
        if !quantity_hex_is_nonzero(&holding.amount_hex) {
            continue;
        }
        match &holding.asset_kind {
            WalletAssetKind::Native => {
                has_value = true;
            }
            WalletAssetKind::Erc20 => {
                has_value = true;
                has_non_native_value = true;
                push_classification(
                    &mut classifications,
                    WalletAddressClassification::TokenHolding,
                );
            }
            WalletAssetKind::Erc721 | WalletAssetKind::Erc1155 | WalletAssetKind::Nft => {
                has_value = true;
                has_non_native_value = true;
                push_classification(
                    &mut classifications,
                    WalletAddressClassification::NftHolding,
                );
            }
            WalletAssetKind::Defi | WalletAssetKind::Airdrop | WalletAssetKind::Reward => {
                has_value = true;
                has_non_native_value = true;
                push_classification(
                    &mut classifications,
                    WalletAddressClassification::ProtocolHolding,
                );
            }
            WalletAssetKind::Approval => {
                has_approval_exposure = true;
            }
            WalletAssetKind::Other(_) => {}
        }
    }
    if has_value {
        push_classification(
            &mut classifications,
            WalletAddressClassification::ValueDetected,
        );
    }
    if has_non_native_value {
        push_classification(
            &mut classifications,
            WalletAddressClassification::AssetValueDetected,
        );
        if !has_native_gas {
            push_classification(
                &mut classifications,
                WalletAddressClassification::StrandedValue,
            );
        }
    }
    if has_approval_exposure {
        push_classification(
            &mut classifications,
            WalletAddressClassification::ApprovalExposure,
        );
    }
    let dormant = has_value
        && match (last_activity_block, chain_tip_block) {
            (Some(activity), Some(tip)) => activity.saturating_add(dormancy_block_window) < tip,
            _ => transaction_count == 0,
        };
    if dormant {
        push_classification(
            &mut classifications,
            WalletAddressClassification::DormantCandidate,
        );
    }
    if !has_value && !has_approval_exposure && transaction_count == 0 {
        push_classification(
            &mut classifications,
            WalletAddressClassification::EmptyCandidate,
        );
    }
    classifications
}

fn max_optional_block(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    left.max(right)
}

fn push_classification(
    classifications: &mut Vec<WalletAddressClassification>,
    value: WalletAddressClassification,
) {
    if !classifications.iter().any(|existing| existing == &value) {
        classifications.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sigillum_api::{EvmProviderProfile, WalletAssetHolding};

    fn wallet(family: &str) -> DiscoveryWallet {
        DiscoveryWallet {
            family: family.into(),
            profile: "archive".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "xpub661MyMwAqRbcFexample".into(),
            derivation_pattern: "project".into(),
            account_index: 0,
        }
    }

    fn provider() -> EvmProviderProfile {
        EvmProviderProfile {
            name: "mainnet".into(),
            compartment_id: 0,
            chain_id: 1,
            rpc_url: "http://localhost:8545".into(),
            auth_token_key: None,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
        }
    }

    fn holding(asset_kind: &str, amount_hex: &str) -> WalletAssetHolding {
        WalletAssetHolding {
            id: "holding_1".into(),
            wallet_family: WALLET_FAMILY_ETH_SEED.into(),
            wallet_profile: "archive".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: WalletAssetKind::from(asset_kind),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            metadata_uri: None,
            metadata_name: None,
            spam_label: None,
            amount_hex: amount_hex.into(),
            source: "test".into(),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 1,
        }
    }

    #[test]
    fn classifies_stranded_dormant_token_value() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_SEED),
            "0x0",
            0,
            &[holding("erc20", "0x1")],
            None,
            None,
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::SignerAvailable)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::StrandedValue)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::DormantCandidate)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::TokenHolding)
        );
    }

    #[test]
    fn classifies_watch_only_nft_with_gas() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_XPUB),
            "0x1",
            2,
            &[holding("erc721", "0x1"), holding("approval", "0x1")],
            None,
            None,
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::WatchOnly)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::GasAvailable)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::NftHolding)
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::ApprovalExposure)
        );
        assert!(
            !classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::StrandedValue)
        );
    }

    #[test]
    fn dormancy_window_marks_old_activity_dormant_despite_transaction_count() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_SEED),
            "0x0",
            5,
            &[holding("erc20", "0x1")],
            Some(100),
            Some(2_000_000),
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
        );

        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::DormantCandidate)
        );
    }

    #[test]
    fn dormancy_window_keeps_recent_activity_active_despite_zero_transaction_count() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_SEED),
            "0x0",
            0,
            &[holding("erc20", "0x1")],
            Some(1_950_000),
            Some(2_000_000),
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
        );

        assert!(
            !classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::DormantCandidate)
        );
    }

    #[test]
    fn dormancy_falls_back_to_transaction_count_without_block_evidence() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_SEED),
            "0x0",
            0,
            &[holding("erc20", "0x1")],
            None,
            None,
            sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
        );

        assert!(
            classifications
                .iter()
                .any(|value| value == &WalletAddressClassification::DormantCandidate)
        );
    }

    #[test]
    fn address_record_persists_classifications() {
        let wallet = wallet(WALLET_FAMILY_ETH_SEED);
        let provider = provider();
        let context = InventoryRecordContext {
            wallet: &wallet,
            provider: &provider,
            address: "0x1111111111111111111111111111111111111111",
            derivation_path: "m/44'/60'/0'/0/0",
            now: 1,
        };
        let record = address_record(
            &context,
            0,
            WalletAddressActivityState::Funded,
            "0x1",
            0,
            Some(123),
            vec![
                WalletAddressClassification::SignerAvailable,
                WalletAddressClassification::ValueDetected,
            ],
        );
        assert_eq!(
            record.classifications,
            vec![
                WalletAddressClassification::SignerAvailable,
                WalletAddressClassification::ValueDetected,
            ]
        );
    }
}
