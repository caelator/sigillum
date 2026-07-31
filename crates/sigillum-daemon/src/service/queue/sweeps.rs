//! Stealth deposit sweep execution helpers.

use sigillum_api::{
    EthStealthSendErc20WithProfileRequest, EthStealthSendWithProfileRequest, EvmProviderRef,
    StealthPaymentRef,
};
use sigillum_core::{StealthHashConvention, decode_quantity_hex};

use crate::service::helpers::{
    compare_u256, is_zero_u256, map_wallet_error, multiply_u256_u64, subtract_u256,
};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::QueueExecution;

impl SigillumService {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_eth_stealth_native_sweep(
        &self,
        token: &str,
        wallet_profile: &str,
        stealth_address: &str,
        ephemeral_public_key_hex: &str,
        destination_address: Option<String>,
        min_value_wei_hex: Option<&str>,
        gas_limit_override: Option<u64>,
        view_tag_hex: Option<String>,
        stealth_hash_convention: Option<StealthHashConvention>,
        operation_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
        let destination_address = destination_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "Native sweep requires destination_address or wallet default destination.",
                )
            })?;
        provider
            .max_priority_fee_per_gas_hex
            .as_ref()
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "provider profile is missing max_priority_fee_per_gas_hex",
                )
            })?;
        let max_fee_per_gas_hex = provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?;
        let gas_limit = gas_limit_override
            .or(provider.native_gas_limit)
            .unwrap_or(21_000);
        let balance = self
            .evm_balance(
                Some(token),
                sigillum_api::EvmRpcBalanceRequest {
                    provider: EvmProviderRef {
                        rpc_url: provider.rpc_url.clone(),
                        auth_token_key: provider.auth_token_key.clone(),
                        compartment_id: Some(provider.compartment_id),
                    },
                    address: stealth_address.to_string(),
                    block_tag: Some("latest".into()),
                },
            )
            .await?;
        let balance_raw =
            decode_quantity_hex(&balance.balance_wei_hex).map_err(map_wallet_error)?;
        let gas_cost = multiply_u256_u64(
            &decode_quantity_hex(&max_fee_per_gas_hex).map_err(map_wallet_error)?,
            gas_limit,
        );
        if compare_u256(&balance_raw, &gas_cost).is_le() {
            return Ok(QueueExecution::Blocked(
                "deposit has insufficient native balance after gas".into(),
            ));
        }
        let spendable = subtract_u256(&balance_raw, &gas_cost);
        if let Some(minimum) = min_value_wei_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&spendable, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "deposit balance has not reached the sweep threshold".into(),
                ));
            }
        }
        let sent = self
            .eth_stealth_send_with_profile_under_operation_guard(
                token,
                EthStealthSendWithProfileRequest {
                    wallet_profile: wallet_profile.into(),
                    stealth: StealthPaymentRef {
                        stealth_address: stealth_address.into(),
                        ephemeral_public_key_hex: ephemeral_public_key_hex.into(),
                        view_tag_hex,
                        // Record-stamped convention from the sweep job; `None`
                        // (pre-switch job) makes the send path probe both.
                        stealth_hash_convention,
                    },
                    value_wei_hex: super::super::evm::encode_quantity_u256(&spendable),
                    destination_address: Some(destination_address),
                    nonce: None,
                    gas_limit: Some(gas_limit),
                    estimate_fees: None,
                    broadcast: Some(false),
                },
                operation_guard,
            )
            .await?;
        Ok(QueueExecution::prepared_from_send(sent))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_eth_stealth_erc20_sweep(
        &self,
        token: &str,
        wallet_profile: &str,
        stealth_address: &str,
        ephemeral_public_key_hex: &str,
        token_address: &str,
        recipient_address: Option<String>,
        min_amount_hex: Option<&str>,
        gas_limit_override: Option<u64>,
        view_tag_hex: Option<String>,
        stealth_hash_convention: Option<StealthHashConvention>,
        operation_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_wallet_profile(wallet_profile)?;
        let recipient_address = recipient_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "ERC-20 sweep requires recipient_address or wallet default destination.",
                )
            })?;
        let gas_limit = gas_limit_override
            .or(provider.erc20_gas_limit)
            .unwrap_or(65_000);
        let max_fee = provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?;
        let (native_balance_raw, token_balance_raw) = self
            .evm_native_and_erc20_balance_for_provider(
                provider.compartment_id,
                &provider,
                stealth_address,
                token_address,
                "latest",
            )
            .await?;
        let gas_cost = multiply_u256_u64(
            &decode_quantity_hex(&max_fee).map_err(map_wallet_error)?,
            gas_limit,
        );
        if compare_u256(&native_balance_raw, &gas_cost).is_lt() {
            return Ok(QueueExecution::Blocked(
                "deposit lacks native gas for ERC-20 sweep".into(),
            ));
        }

        let amount_hex = super::super::evm::encode_quantity_u256(&token_balance_raw);
        let amount = decode_quantity_hex(&amount_hex).map_err(map_wallet_error)?;
        if is_zero_u256(&amount) {
            return Ok(QueueExecution::Blocked(
                "deposit has no ERC-20 balance to sweep".into(),
            ));
        }
        if let Some(minimum) = min_amount_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&amount, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "deposit token balance has not reached the sweep threshold".into(),
                ));
            }
        }

        let sent = self
            .eth_stealth_send_erc20_with_profile_under_operation_guard(
                token,
                EthStealthSendErc20WithProfileRequest {
                    wallet_profile: wallet_profile.into(),
                    stealth: StealthPaymentRef {
                        stealth_address: stealth_address.into(),
                        ephemeral_public_key_hex: ephemeral_public_key_hex.into(),
                        view_tag_hex,
                        // Record-stamped convention from the sweep job; `None`
                        // (pre-switch job) makes the send path probe both.
                        stealth_hash_convention,
                    },
                    token_address: token_address.into(),
                    recipient_address,
                    amount_hex,
                    nonce: None,
                    gas_limit: Some(gas_limit),
                    estimate_fees: None,
                    broadcast: Some(false),
                },
                operation_guard,
            )
            .await?;
        Ok(QueueExecution::prepared_from_send(sent))
    }
}
