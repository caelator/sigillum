//! Profile-backed stealth send request resolution.

use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendErc20WithProfileRequest,
    EthStealthSendResponse, EthStealthSendTransferRequest, EthStealthSendWithProfileRequest,
};

use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) async fn eth_stealth_send_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendWithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let _ = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
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
                fees: sigillum_api::Eip1559Fees {
                    chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                    max_priority_fee_per_gas_hex: provider
                        .max_priority_fee_per_gas_hex
                        .clone()
                        .ok_or_else(|| {
                            ServiceError::bad_request(
                                "provider profile is missing max_priority_fee_per_gas_hex",
                            )
                        })?,
                    max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
                        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
                    })?,
                },
                destination_address,
                value_wei_hex: body.value_wei_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit: body.gas_limit.or(provider.native_gas_limit),
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
        let _ = self.require_session(token)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;

        self.eth_stealth_send_erc20_transfer(
            token,
            EthStealthSendErc20TransferRequest {
                rpc_url: provider.rpc_url,
                wallet: wallet.wallet,
                stealth: body.stealth.clone(),
                fees: sigillum_api::Eip1559Fees {
                    chain_id: wallet.chain_id.unwrap_or(provider.chain_id),
                    max_priority_fee_per_gas_hex: provider
                        .max_priority_fee_per_gas_hex
                        .clone()
                        .ok_or_else(|| {
                            ServiceError::bad_request(
                                "provider profile is missing max_priority_fee_per_gas_hex",
                            )
                        })?,
                    max_fee_per_gas_hex: provider.max_fee_per_gas_hex.clone().ok_or_else(|| {
                        ServiceError::bad_request("provider profile is missing max_fee_per_gas_hex")
                    })?,
                },
                token_address: body.token_address,
                recipient_address: body.recipient_address,
                amount_hex: body.amount_hex,
                auth_token_key: provider.auth_token_key,
                provider_compartment_id: Some(provider.compartment_id),
                wallet_compartment_id: Some(wallet.compartment_id),
                nonce: body.nonce,
                gas_limit: body.gas_limit.or(provider.erc20_gas_limit),
                broadcast: body.broadcast,
            },
        )
        .await
    }
}
