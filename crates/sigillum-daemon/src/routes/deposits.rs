//! Route handlers for Ethereum stealth deposits: creation, deletion, refresh, and sweep operations.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthStealthDepositCreateErc20Request, EthStealthDepositCreateNativeRequest,
    EthStealthDepositDeleteRequest, EthStealthDepositEnqueueSweepRequest,
    EthStealthDepositRefreshRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn list_eth_stealth_deposits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_eth_stealth_deposits(bearer_token(&headers).as_deref()))
}

pub(crate) async fn create_eth_stealth_native_deposit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EthStealthDepositCreateNativeRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .create_eth_stealth_native_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn create_eth_stealth_erc20_deposit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EthStealthDepositCreateErc20Request>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .create_eth_stealth_erc20_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_eth_stealth_deposit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EthStealthDepositDeleteRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_eth_stealth_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn refresh_eth_stealth_deposits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EthStealthDepositRefreshRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .refresh_eth_stealth_deposits(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn enqueue_eth_stealth_deposit_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<EthStealthDepositEnqueueSweepRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_deposit_sweep(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
