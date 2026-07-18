//! `EthSeed*` legacy queue job execution (W7.3).
//!
//! Mirrors `sweeps.rs`'s stealth-sweep pattern (balance check, then sign +
//! broadcast) but signs with a seed-wallet key derived on demand from the
//! profile's vault-stored mnemonic, instead of a stealth-derived key. Gated
//! the same way as `PlanStepExecution` (`ExecutionFamily::Sweep`, see
//! `gates.rs`): with gates off the drain loop never reaches this module.

use sigillum_api::{EthSeedWalletProfile, EvmProviderProfile, error_codes};
use sigillum_core::{
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, decode_quantity_hex,
    ethereum_address_from_signing_key, sign_ethereum_erc20_transfer, sign_ethereum_native_transfer,
};

use crate::service::helpers::{
    compare_u256, is_zero_u256, map_wallet_error, multiply_u256_u64, subtract_u256,
};
use crate::service::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::QueueExecution;

impl SigillumService {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_eth_seed_transfer(
        &self,
        wallet_profile: &str,
        address: &str,
        derivation_path: &str,
        value_wei_hex: &str,
        destination_address: &str,
        nonce_override: Option<u64>,
        gas_limit_override: Option<u64>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_eth_seed_wallet_profile(wallet_profile)?;
        let value = decode_quantity_hex(value_wei_hex).map_err(map_wallet_error)?;
        let gas_limit = gas_limit_override
            .or(provider.native_gas_limit)
            .unwrap_or(21_000);
        let (signing_key, nonce) = self
            .prepare_eth_seed_signer(&provider, &wallet, address, derivation_path, nonce_override)
            .await?;
        let signed = sign_ethereum_native_transfer(
            &signing_key,
            &EthereumEip1559Transfer {
                chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                nonce,
                max_priority_fee_per_gas: static_max_priority_fee(&provider)?,
                max_fee_per_gas: static_max_fee(&provider)?,
                gas_limit,
                destination_address: destination_address.into(),
                value,
            },
        )
        .map_err(map_wallet_error)?;
        drop(signing_key);
        Ok(QueueExecution::prepared_from_signed(signed))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_eth_seed_native_sweep(
        &self,
        wallet_profile: &str,
        address: &str,
        derivation_path: &str,
        destination_address: Option<String>,
        min_value_wei_hex: Option<&str>,
        gas_limit_override: Option<u64>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_eth_seed_wallet_profile(wallet_profile)?;
        let destination_address = destination_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "Native sweep requires destination_address or wallet default destination.",
                )
            })?;
        let gas_limit = gas_limit_override
            .or(provider.native_gas_limit)
            .unwrap_or(21_000);
        let max_fee_per_gas = static_max_fee(&provider)?;
        let balance_hex = self
            .evm_native_balance_for_provider(provider.compartment_id, &provider, address, "latest")
            .await?;
        let balance_raw = decode_quantity_hex(&balance_hex).map_err(map_wallet_error)?;
        let gas_cost = multiply_u256_u64(&max_fee_per_gas, gas_limit);
        if compare_u256(&balance_raw, &gas_cost).is_le() {
            return Ok(QueueExecution::Blocked(
                "seed wallet has insufficient native balance after gas".into(),
            ));
        }
        let spendable = subtract_u256(&balance_raw, &gas_cost);
        if let Some(minimum) = min_value_wei_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&spendable, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "seed wallet balance has not reached the sweep threshold".into(),
                ));
            }
        }
        // The queued sweep threshold is only a lower bound. Re-authorize the
        // freshly observed, gas-adjusted amount before deriving the signer or
        // fetching its nonce so a balance increase cannot bypass the per-step
        // treasury cap between enqueue and execution.
        let spendable_hex = super::super::evm::encode_quantity_u256(&spendable);
        if let Err(error) = self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(&destination_address),
            asset_kind: "native",
            amount_hex: &spendable_hex,
        }) {
            if error.code() == error_codes::POLICY_VIOLATION {
                let Some(action) = error.action() else {
                    return Err(error);
                };
                return Ok(QueueExecution::Blocked(format!(
                    "policy_violation: {action}"
                )));
            }
            return Err(error);
        }
        let (signing_key, nonce) = self
            .prepare_eth_seed_signer(&provider, &wallet, address, derivation_path, None)
            .await?;
        let signed = sign_ethereum_native_transfer(
            &signing_key,
            &EthereumEip1559Transfer {
                chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                nonce,
                max_priority_fee_per_gas: static_max_priority_fee(&provider)?,
                max_fee_per_gas,
                gas_limit,
                destination_address,
                value: spendable,
            },
        )
        .map_err(map_wallet_error)?;
        drop(signing_key);
        Ok(QueueExecution::prepared_from_signed(signed))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::service::queue) async fn process_eth_seed_erc20_sweep(
        &self,
        wallet_profile: &str,
        address: &str,
        derivation_path: &str,
        token_address: &str,
        recipient_address: Option<String>,
        min_amount_hex: Option<&str>,
        gas_limit_override: Option<u64>,
    ) -> ServiceResult<QueueExecution> {
        let (provider, wallet) = self.resolve_eth_seed_wallet_profile(wallet_profile)?;
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
        let max_fee = static_max_fee(&provider)?;
        let (native_balance_raw, token_balance_raw) = self
            .evm_native_and_erc20_balance_for_provider(
                provider.compartment_id,
                &provider,
                address,
                token_address,
                "latest",
            )
            .await?;
        let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
        if compare_u256(&native_balance_raw, &gas_cost).is_lt() {
            return Ok(QueueExecution::Blocked(
                "seed wallet lacks native gas for ERC-20 sweep".into(),
            ));
        }
        if is_zero_u256(&token_balance_raw) {
            return Ok(QueueExecution::Blocked(
                "seed wallet has no ERC-20 balance to sweep".into(),
            ));
        }
        if let Some(minimum) = min_amount_hex {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            if compare_u256(&token_balance_raw, &minimum).is_lt() {
                return Ok(QueueExecution::Blocked(
                    "seed wallet token balance has not reached the sweep threshold".into(),
                ));
            }
        }
        let (signing_key, nonce) = self
            .prepare_eth_seed_signer(&provider, &wallet, address, derivation_path, None)
            .await?;
        let signed = sign_ethereum_erc20_transfer(
            &signing_key,
            &EthereumEip1559Erc20Transfer {
                chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                nonce,
                max_priority_fee_per_gas: static_max_priority_fee(&provider)?,
                max_fee_per_gas: max_fee,
                gas_limit,
                token_address: token_address.into(),
                recipient_address,
                amount: token_balance_raw,
            },
        )
        .map_err(map_wallet_error)?;
        drop(signing_key);
        Ok(QueueExecution::prepared_from_signed(signed))
    }

    /// Derive the signing key, verify it matches `address` (defense in
    /// depth against a corrupted/mismatched derivation path), and fetch the
    /// pending nonce. Locked compartment / missing profile fail closed via
    /// the caller's normal error classification (never panics).
    async fn prepare_eth_seed_signer(
        &self,
        provider: &EvmProviderProfile,
        wallet: &EthSeedWalletProfile,
        address: &str,
        derivation_path: &str,
        nonce_override: Option<u64>,
    ) -> ServiceResult<(k256::ecdsa::SigningKey, u64)> {
        let signing_key = self.derive_eth_seed_signing_key(wallet, derivation_path)?;
        if !ethereum_address_from_signing_key(&signing_key).eq_ignore_ascii_case(address) {
            return Err(ServiceError::forbidden(
                "Derived signing key does not match the job's source address.",
            ));
        }
        let nonce = match nonce_override {
            Some(nonce) => nonce,
            None => {
                self.evm_transaction_count_for_provider(
                    provider.compartment_id,
                    provider,
                    address,
                    "pending",
                )
                .await?
            }
        };
        Ok((signing_key, nonce))
    }
}

fn static_max_priority_fee(provider: &EvmProviderProfile) -> ServiceResult<[u8; 32]> {
    let hex = provider
        .max_priority_fee_per_gas_hex
        .as_deref()
        .ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_priority_fee_per_gas_hex")
        })?;
    decode_quantity_hex(hex).map_err(map_wallet_error)
}

fn static_max_fee(provider: &EvmProviderProfile) -> ServiceResult<[u8; 32]> {
    let hex = provider.max_fee_per_gas_hex.as_deref().ok_or_else(|| {
        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
    })?;
    decode_quantity_hex(hex).map_err(map_wallet_error)
}
