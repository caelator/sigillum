//! Profile-backed stealth send request resolution.

use sigillum_api::{
    Eip1559Fees, EthStealthSendErc20TransferRequest, EthStealthSendErc20WithProfileRequest,
    EthStealthSendResponse, EthStealthSendTransferRequest, EthStealthSendWithProfileRequest,
    EvmFeeEstimateRequest, EvmProviderRef,
};

use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) async fn eth_stealth_send_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendWithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let session_token = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        let chain_id = wallet.chain_id.unwrap_or(provider.chain_id);
        let gas_limit = body.gas_limit.or(provider.native_gas_limit);
        let fees = if body.estimate_fees.unwrap_or(false) {
            self.evm_estimate_fees(
                Some(session_token),
                EvmFeeEstimateRequest {
                    provider: EvmProviderRef {
                        rpc_url: provider.rpc_url.clone(),
                        auth_token_key: provider.auth_token_key.clone(),
                        compartment_id: Some(provider.compartment_id),
                    },
                    chain_id,
                    gas_limit: Some(gas_limit.unwrap_or(21_000)),
                },
            )
            .await?
            .fees
        } else {
            static_profile_fees(&provider, chain_id)?
        };
        let destination_address = body
            .destination_address
            .or(wallet.default_destination_address.clone())
            .ok_or_else(|| {
                ServiceError::bad_request(
                    "destination_address is required when the wallet profile has no default.",
                )
            })?;

        self.eth_stealth_send_transfer(
            token,
            EthStealthSendTransferRequest {
                rpc_url: provider.rpc_url,
                wallet: wallet.wallet,
                stealth: body.stealth.clone(),
                fees,
                destination_address,
                value_wei_hex: body.value_wei_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit,
                broadcast: body.broadcast,
            },
        )
        .await
    }

    pub(crate) async fn eth_stealth_send_erc20_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendErc20WithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let session_token = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        let chain_id = wallet.chain_id.unwrap_or(provider.chain_id);
        let gas_limit = body.gas_limit.or(provider.erc20_gas_limit);
        let fees = if body.estimate_fees.unwrap_or(false) {
            self.evm_estimate_fees(
                Some(session_token),
                EvmFeeEstimateRequest {
                    provider: EvmProviderRef {
                        rpc_url: provider.rpc_url.clone(),
                        auth_token_key: provider.auth_token_key.clone(),
                        compartment_id: Some(provider.compartment_id),
                    },
                    chain_id,
                    gas_limit: Some(gas_limit.unwrap_or(65_000)),
                },
            )
            .await?
            .fees
        } else {
            static_profile_fees(&provider, chain_id)?
        };

        self.eth_stealth_send_erc20_transfer(
            token,
            EthStealthSendErc20TransferRequest {
                rpc_url: provider.rpc_url,
                wallet: wallet.wallet,
                stealth: body.stealth.clone(),
                fees,
                token_address: body.token_address,
                recipient_address: body.recipient_address,
                amount_hex: body.amount_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit,
                broadcast: body.broadcast,
            },
        )
        .await
    }
}

fn static_profile_fees(
    provider: &sigillum_api::EvmProviderProfile,
    chain_id: u64,
) -> ServiceResult<Eip1559Fees> {
    Ok(Eip1559Fees {
        chain_id,
        max_priority_fee_per_gas_hex: provider.max_priority_fee_per_gas_hex.clone().ok_or_else(
            || {
                ServiceError::bad_request(
                    "provider profile is missing max_priority_fee_per_gas_hex",
                )
            },
        )?,
        max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
            ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
        })?,
    })
}
