//! Stealth-address native and ERC-20 transfer signing and broadcast.

use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendResponse, EthStealthSendTransferRequest,
};
use sigillum_core::{
    EthereumEip1559Erc20Transfer, EthereumEip1559Transfer, VaultLifecycle, decode_quantity_hex,
    derive_sigillum_ethereum_stealth_wallet, sign_ethereum_stealth_erc20_transfer,
    sign_ethereum_stealth_native_transfer,
};

use crate::audit_log::AuditEventSpec;
use crate::service::helpers::{decode_optional_view_tag, map_wallet_error};
use crate::service::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) async fn eth_stealth_send_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSendTransferRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(&body.destination_address),
            asset_kind: "native",
            amount_hex: &body.value_wei_hex,
        })?;
        let active_compartment_id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::vault_locked("No active compartment."))?;
        let provider_compartment_id = body
            .provider_compartment_id
            .unwrap_or(active_compartment_id);
        let wallet_compartment_id = body.wallet_compartment_id.unwrap_or(active_compartment_id);
        let rpc = self.resolve_provider_rpc_client_for_compartment(
            provider_compartment_id,
            &body.rpc_url,
            body.auth_token_key.as_deref(),
        )?;
        let nonce = match body.nonce {
            Some(nonce) => nonce,
            None => {
                rpc.get_transaction_count(&body.stealth.stealth_address, "pending")
                    .await?
            }
        };
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let value = decode_quantity_hex(&body.value_wei_hex).map_err(map_wallet_error)?;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let gas_limit = body.gas_limit.unwrap_or(21_000);
        let broadcast = body.broadcast.unwrap_or(false);

        let signed = self.with_vault(wallet_compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::vault_locked("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &body.wallet, "eth")
                    .map_err(map_wallet_error)?;
            sign_ethereum_stealth_native_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Transfer {
                    chain_id: body.fees.chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    destination_address: body.destination_address.clone(),
                    value,
                },
            )
            .map_err(map_wallet_error)
        })?;

        let broadcast_transaction_hash_hex = if broadcast {
            Some(
                rpc.send_raw_transaction(&signed.raw_transaction_hex)
                    .await?,
            )
        } else {
            None
        };

        self.record_audit(
            Some(wallet_compartment_id),
            AuditEventSpec::WalletEthStealthSendTransfer {
                wallet: body.wallet.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
                broadcast,
                transaction_hash_hex: signed.transaction_hash_hex.clone(),
                broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.clone(),
            },
        )?;

        Ok(EthStealthSendResponse {
            wallet: body.wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
            broadcast,
            broadcast_transaction_hash_hex,
        })
    }

    pub(crate) async fn eth_stealth_send_erc20_transfer(
        &self,
        token: Option<&str>,
        body: EthStealthSendErc20TransferRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        self.authorize_transaction_policy(TransactionPolicyCheck {
            kind: TransactionPolicyKind::RoutedTransfer,
            destination_address: Some(&body.recipient_address),
            asset_kind: "erc20",
            amount_hex: &body.amount_hex,
        })?;
        let active_compartment_id = self
            .state
            .active_compartment_id_for(token)
            .ok_or_else(|| ServiceError::vault_locked("No active compartment."))?;
        let provider_compartment_id = body
            .provider_compartment_id
            .unwrap_or(active_compartment_id);
        let wallet_compartment_id = body.wallet_compartment_id.unwrap_or(active_compartment_id);
        let rpc = self.resolve_provider_rpc_client_for_compartment(
            provider_compartment_id,
            &body.rpc_url,
            body.auth_token_key.as_deref(),
        )?;
        let nonce = match body.nonce {
            Some(nonce) => nonce,
            None => {
                rpc.get_transaction_count(&body.stealth.stealth_address, "pending")
                    .await?
            }
        };
        let max_priority_fee_per_gas = decode_quantity_hex(&body.fees.max_priority_fee_per_gas_hex)
            .map_err(map_wallet_error)?;
        let max_fee_per_gas =
            decode_quantity_hex(&body.fees.max_fee_per_gas_hex).map_err(map_wallet_error)?;
        let amount = decode_quantity_hex(&body.amount_hex).map_err(map_wallet_error)?;
        let view_tag = decode_optional_view_tag(body.stealth.view_tag_hex.as_deref())?;
        let gas_limit = body.gas_limit.unwrap_or(65_000);
        let broadcast = body.broadcast.unwrap_or(false);

        let signed = self.with_vault(wallet_compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::vault_locked("Vault is locked."))?;
            let derived =
                derive_sigillum_ethereum_stealth_wallet(master_key.as_ref(), &body.wallet, "eth")
                    .map_err(map_wallet_error)?;
            sign_ethereum_stealth_erc20_transfer(
                &derived,
                &body.stealth.stealth_address,
                &body.stealth.ephemeral_public_key_hex,
                view_tag,
                &EthereumEip1559Erc20Transfer {
                    chain_id: body.fees.chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    token_address: body.token_address.clone(),
                    recipient_address: body.recipient_address.clone(),
                    amount,
                },
            )
            .map_err(map_wallet_error)
        })?;

        let broadcast_transaction_hash_hex = if broadcast {
            Some(
                rpc.send_raw_transaction(&signed.raw_transaction_hex)
                    .await?,
            )
        } else {
            None
        };

        self.record_audit(
            Some(wallet_compartment_id),
            AuditEventSpec::WalletEthStealthSendErc20Transfer {
                wallet: body.wallet.clone(),
                to: signed.to_address.clone(),
                nonce: signed.nonce,
                broadcast,
                transaction_hash_hex: signed.transaction_hash_hex.clone(),
                broadcast_transaction_hash_hex: broadcast_transaction_hash_hex.clone(),
            },
        )?;

        Ok(EthStealthSendResponse {
            wallet: body.wallet,
            kind: signed.kind,
            chain_id: signed.chain_id,
            nonce: signed.nonce,
            from_address: signed.from_address,
            to_address: signed.to_address,
            value_hex: signed.value_hex,
            data_hex: signed.data_hex,
            raw_transaction_hex: signed.raw_transaction_hex,
            transaction_hash_hex: signed.transaction_hash_hex,
            broadcast,
            broadcast_transaction_hash_hex,
        })
    }
}
