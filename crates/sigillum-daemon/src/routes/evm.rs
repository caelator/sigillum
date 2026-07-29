//! Route handlers for EVM RPC operations and Ethereum stealth send transactions.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendTransferRequest, EvmFeeEstimateRequest,
    EvmRpcBalanceRequest, EvmRpcBroadcastRequest, EvmRpcErc20BalanceRequest, EvmRpcNonceRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn evm_nonce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmRpcNonceRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .evm_nonce(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn evm_balance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmRpcBalanceRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .evm_balance(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn evm_erc20_balance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmRpcErc20BalanceRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .evm_erc20_balance(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn evm_broadcast(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmRpcBroadcastRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .evm_broadcast(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn evm_estimate_fees(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EvmFeeEstimateRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .evm_estimate_fees(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_send_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSendTransferRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .eth_stealth_send_transfer(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn eth_stealth_send_erc20_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSendErc20TransferRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .eth_stealth_send_erc20_transfer(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
