use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceResult, SigillumService};

use super::support::{
    InventoryAddressObservation, InventoryRecordContext, address_record, holding_record,
    holding_record_with_source, quantity_hex_is_nonzero,
};
use super::token_discovery::{
    DISCOVERY_SOURCE_ERC20_TRANSFER_LOG, Erc20TransferDiscoveryConfig, push_unique_token,
};
use super::{DISCOVERY_SOURCE_LOCAL_RPC, DiscoveryWallet};

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
        now: u64,
    ) -> ServiceResult<InventoryAddressObservation> {
        let address = super::normalize_address(address)?;
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

        for token_address in observed_token_addresses {
            let amount_hex = self
                .evm_erc20_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    &token_address,
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
                source_for_token(&transfer_log_tokens, &token_address),
            ));
        }

        Ok(InventoryAddressObservation {
            address: address_record(
                &record_context,
                address_index,
                activity_state,
                &native_balance_wei_hex,
                transaction_count,
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
