//! Route handlers for transaction queue management and processing.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    QueueEthStealthErc20SweepRequest, QueueEthStealthErc20TransferRequest,
    QueueEthStealthNativeSweepRequest, QueueEthStealthTransferRequest, QueueProcessRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::list_query::QueueJobsRawQuery;
use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn list_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<QueueJobsRawQuery>,
) -> Response {
    let query = match query.resolve() {
        Ok(query) => query,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.list_queue_jobs(bearer_token(&headers).as_deref(), query))
}

pub(crate) async fn enqueue_eth_stealth_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<QueueEthStealthTransferRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<QueueEthStealthErc20TransferRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<QueueEthStealthNativeSweepRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<QueueEthStealthErc20SweepRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<QueueProcessRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .process_queue(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn pause_execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .set_queue_execution_paused(bearer_token(&headers).as_deref(), true)
            .await,
    )
}

pub(crate) async fn resume_execution(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .set_queue_execution_paused(bearer_token(&headers).as_deref(), false)
            .await,
    )
}
