//! Route handlers for Ethereum stealth deposits: creation, deletion, refresh, and sweep operations.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthStealthAnnouncementScanRequest, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositRefreshRequest,
    ReceivingDepositTagRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

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
    ValidatedJson(body): ValidatedJson<EthStealthDepositCreateNativeRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<EthStealthDepositCreateErc20Request>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .create_eth_stealth_erc20_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn scan_eth_stealth_announcements(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthAnnouncementScanRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .scan_eth_stealth_announcements(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_eth_stealth_deposit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthDepositDeleteRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_eth_stealth_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn tag_eth_stealth_deposit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<ReceivingDepositTagRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .tag_eth_stealth_deposit(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn refresh_eth_stealth_deposits(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthDepositRefreshRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<EthStealthDepositEnqueueSweepRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .enqueue_eth_stealth_deposit_sweep(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
