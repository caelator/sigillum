//! Route handlers for Ethereum stealth wallet operations: generate, check, sign, and export.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::{
    EthStealthCheckRequest, EthStealthExportRequest, EthStealthGenerateRequest,
    EthStealthSignErc20TransferRequest, EthStealthSignRequest, EthStealthSignTransferRequest,
    EthXpubDeriveRequest, EthXpubExportRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn eth_xpub_export(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthXpubExportRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_xpub_export(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn eth_xpub_derive(
    State(state): State<Arc<AppState>>,
    ValidatedJson(body): ValidatedJson<EthXpubDeriveRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_xpub_derive(body))
}

pub(crate) async fn eth_stealth_export(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthExportRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_stealth_export(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn eth_stealth_generate(
    State(state): State<Arc<AppState>>,
    ValidatedJson(body): ValidatedJson<EthStealthGenerateRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_stealth_generate(body))
}

pub(crate) async fn eth_stealth_check(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthCheckRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_stealth_check(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn eth_stealth_sign(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSignRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_stealth_sign(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn eth_stealth_sign_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSignTransferRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.eth_stealth_sign_transfer(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn eth_stealth_sign_erc20_transfer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<EthStealthSignErc20TransferRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service.eth_stealth_sign_erc20_transfer(bearer_token(&headers).as_deref(), body),
    )
}
