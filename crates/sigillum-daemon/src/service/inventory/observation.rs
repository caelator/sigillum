use sigillum_api::EvmProviderProfile;

use crate::service::evm::normalize_address;
use crate::service::{ServiceResult, SigillumService};

use super::allowance_discovery::{
    DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE, Erc20AllowanceDiscoveryConfig,
};
use super::claim_discovery::{ClaimCandidateDiscoveryConfig, claim_candidate_source};
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
            "funded"
        } else if transaction_count > 0 {
            "active"
        } else {
            "empty"
        };

        let record_context = InventoryRecordContext {
            wallet,
            provider,
            address: &address,
            derivation_path,
            now,
        };
        let mut holdings = vec![holding_record(
            &record_context,
            "native",
            None,
            &native_balance_wei_hex,
        )];

        let mut observed_token_addresses = token_addresses.to_vec();
        let mut transfer_log_tokens = Vec::new();
        if let Some(config) = token_discovery {
            transfer_log_tokens = self
                .discover_erc20_transfer_tokens_for_address(provider, &address, config)
                .await?;
            if !transfer_log_tokens.is_empty() && activity_state == "empty" {
                activity_state = "active";
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
                activity_state = "funded";
            }
            holdings.push(holding_record_with_source(
                &record_context,
                "erc20",
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
                    "approval",
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
                    "approval",
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
            let nfts = self
                .discover_erc721_transfer_holdings_for_address(provider, &address, config)
                .await?;
            if !nfts.is_empty() {
                activity_state = "funded";
            }
            for nft in nfts {
                let contract_address = nft.contract_address;
                push_unique_contract(
                    &mut observed_nft_contract_addresses,
                    contract_address.clone(),
                );
                holdings.push(holding_record_with_token_id(
                    &record_context,
                    "erc721",
                    Some(contract_address),
                    Some(nft.token_id_hex),
                    "0x1",
                    DISCOVERY_SOURCE_ERC721_TRANSFER_LOG,
                ));
            }
        }

        if let Some(config) = erc1155_discovery {
            let tokens = self
                .discover_erc1155_transfer_holdings_for_address(provider, &address, config)
                .await?;
            if !tokens.is_empty() {
                activity_state = "funded";
            }
            for token in tokens {
                let contract_address = token.contract_address;
                push_unique_contract(
                    &mut observed_nft_contract_addresses,
                    contract_address.clone(),
                );
                holdings.push(holding_record_with_token_id(
                    &record_context,
                    "erc1155",
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
                    "approval",
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
                    activity_state = "funded";
                }
                holdings.push(holding_record_with_protocol_counterparty(
                    &record_context,
                    "defi",
                    Some(position.token_address),
                    position.protocol_address,
                    None,
                    &position.amount_hex,
                    &defi_token_probe_source(&position.protocol),
                ));
            }
        }

        if let Some(config) = claim_candidate_discovery {
            for candidate in config.candidates_for_address(&address) {
                activity_state = "funded";
                holdings.push(holding_record_with_claim_metadata(
                    &record_context,
                    &candidate.kind,
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

        Ok(InventoryAddressObservation {
            address: address_record(
                &record_context,
                address_index,
                activity_state,
                &native_balance_wei_hex,
                transaction_count,
                address_classifications(
                    wallet,
                    &native_balance_wei_hex,
                    transaction_count,
                    &holdings,
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
) -> Vec<String> {
    let mut classifications = Vec::new();
    match wallet.family.as_str() {
        WALLET_FAMILY_ETH_SEED => push_classification(&mut classifications, "signer_available"),
        WALLET_FAMILY_ETH_XPUB | WALLET_FAMILY_ETH_WATCH => {
            push_classification(&mut classifications, "watch_only");
        }
        _ => push_classification(&mut classifications, "signer_unknown"),
    }

    let has_native_gas = quantity_hex_is_nonzero(native_balance_wei_hex);
    if has_native_gas {
        push_classification(&mut classifications, "gas_available");
    }
    if transaction_count > 0 {
        push_classification(&mut classifications, "transaction_history");
    }

    let mut has_value = has_native_gas;
    let mut has_non_native_value = false;
    let mut has_approval_exposure = false;
    for holding in holdings {
        if !quantity_hex_is_nonzero(&holding.amount_hex) {
            continue;
        }
        match holding.asset_kind.as_str() {
            "native" => {
                has_value = true;
            }
            "erc20" => {
                has_value = true;
                has_non_native_value = true;
                push_classification(&mut classifications, "token_holding");
            }
            "erc721" | "erc1155" | "nft" => {
                has_value = true;
                has_non_native_value = true;
                push_classification(&mut classifications, "nft_holding");
            }
            "defi" | "airdrop" | "reward" => {
                has_value = true;
                has_non_native_value = true;
                push_classification(&mut classifications, "protocol_holding");
            }
            "approval" => {
                has_approval_exposure = true;
            }
            _ => {}
        }
    }
    if has_value {
        push_classification(&mut classifications, "value_detected");
    }
    if has_non_native_value {
        push_classification(&mut classifications, "asset_value_detected");
        if !has_native_gas {
            push_classification(&mut classifications, "stranded_value");
        }
    }
    if has_approval_exposure {
        push_classification(&mut classifications, "approval_exposure");
    }
    if has_value && transaction_count == 0 {
        push_classification(&mut classifications, "dormant_candidate");
    }
    if !has_value && !has_approval_exposure && transaction_count == 0 {
        push_classification(&mut classifications, "empty_candidate");
    }
    classifications
}

fn push_classification(classifications: &mut Vec<String>, value: &str) {
    if !classifications.iter().any(|existing| existing == value) {
        classifications.push(value.into());
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
            asset_kind: asset_kind.into(),
            asset_address: Some("0x2222222222222222222222222222222222222222".into()),
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: None,
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
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
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == "signer_available")
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == "stranded_value")
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == "dormant_candidate")
        );
        assert!(classifications.iter().any(|value| value == "token_holding"));
    }

    #[test]
    fn classifies_watch_only_nft_with_gas() {
        let classifications = address_classifications(
            &wallet(WALLET_FAMILY_ETH_XPUB),
            "0x1",
            2,
            &[holding("erc721", "0x1"), holding("approval", "0x1")],
        );
        assert!(classifications.iter().any(|value| value == "watch_only"));
        assert!(classifications.iter().any(|value| value == "gas_available"));
        assert!(classifications.iter().any(|value| value == "nft_holding"));
        assert!(
            classifications
                .iter()
                .any(|value| value == "approval_exposure")
        );
        assert!(
            !classifications
                .iter()
                .any(|value| value == "stranded_value")
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
            "funded",
            "0x1",
            0,
            vec!["signer_available".into(), "value_detected".into()],
        );
        assert_eq!(
            record.classifications,
            vec!["signer_available".to_string(), "value_detected".to_string()]
        );
    }
}
