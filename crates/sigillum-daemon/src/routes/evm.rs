//! Route handlers for EVM RPC operations and Ethereum stealth send transactions.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthStealthSendErc20TransferRequest, EthStealthSendTransferRequest, EvmFeeEstimateRequest,
    EvmRpcBalanceRequest, EvmRpcBroadcastRequest, EvmRpcErc20BalanceRequest, EvmRpcNonceRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn evm_nonce(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EvmRpcNonceRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EvmRpcBalanceRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EvmRpcErc20BalanceRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EvmRpcBroadcastRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EvmFeeEstimateRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EthStealthSendTransferRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
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
    Json(body): Json<EthStealthSendErc20TransferRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .eth_stealth_send_erc20_transfer(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
