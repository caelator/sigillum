//! Route handlers for transaction queue management and processing.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueProcessRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn list_jobs(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_queue_jobs(bearer_token(&headers).as_deref()))
}

pub(crate) async fn enqueue_eth_stealth_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<QueueEthStealthTransferRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_transfer(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn enqueue_eth_stealth_erc20_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<QueueEthStealthErc20TransferRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_erc20_transfer(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn enqueue_eth_stealth_native_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<QueueEthStealthNativeSweepRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_native_sweep(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn enqueue_eth_stealth_erc20_sweep(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<QueueEthStealthErc20SweepRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_erc20_sweep(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn process_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<QueueProcessRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .process_queue(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
