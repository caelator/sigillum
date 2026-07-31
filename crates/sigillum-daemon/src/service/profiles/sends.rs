//! Profile-backed stealth send request resolution.

use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendErc20WithProfileRequest,
    EthStealthSendResponse, EthStealthSendTransferRequest, EthStealthSendWithProfileRequest,
    EvmFeeEstimateRequest, EvmProviderRef,
};

use super::fees::static_profile_fees;
use crate::service::{ServiceError, ServiceResult, SigillumService};

impl SigillumService {
    pub(crate) async fn eth_stealth_send_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendWithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let operation_guard = self.acquire_session_operation(&session_context).await?;
        self.eth_stealth_send_with_profile_under_operation_guard(token, body, &operation_guard)
            .await
    }

    pub(in crate::service) async fn eth_stealth_send_with_profile_under_operation_guard(
        &self,
        token: &str,
        body: EthStealthSendWithProfileRequest,
        operation_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> ServiceResult<EthStealthSendResponse> {
        let session_token = self.require_session(Some(token))?;
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

        self.eth_stealth_send_transfer_under_operation_guard(
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
            operation_guard,
        )
        .await
    }

    pub(crate) async fn eth_stealth_send_erc20_with_profile(
        &self,
        token: Option<&str>,
        body: EthStealthSendErc20WithProfileRequest,
    ) -> ServiceResult<EthStealthSendResponse> {
        let token = self.require_session(token)?;
        let session_context = self.capture_session_operation_context(Some(token))?;
        let operation_guard = self.acquire_session_operation(&session_context).await?;
        self.eth_stealth_send_erc20_with_profile_under_operation_guard(
            token,
            body,
            &operation_guard,
        )
        .await
    }

    pub(in crate::service) async fn eth_stealth_send_erc20_with_profile_under_operation_guard(
        &self,
        token: &str,
        body: EthStealthSendErc20WithProfileRequest,
        operation_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> ServiceResult<EthStealthSendResponse> {
        let session_token = self.require_session(Some(token))?;
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

        self.eth_stealth_send_erc20_transfer_under_operation_guard(
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
            operation_guard,
        )
        .await
    }
}
