//! Route handlers for FIDO2 authentication: setup, registration, unlock, and credential removal.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{
    Fido2RegisterRequest, Fido2RemoveRequest, Fido2SetPinRequest, Fido2SetupRequest,
    Fido2UnlockRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn fido2_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.fido2_status(bearer_token(&headers).as_deref()))
}

pub(crate) async fn fido2_detect(State(state): State<Arc<AppState>>) -> Response {
    let service = SigillumService::new(state);
    service_response(Ok(service.fido2_detect()))
}

pub(crate) async fn fido2_set_pin(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<Fido2SetPinRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.fido2_set_pin(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn fido2_list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let service = SigillumService::new(state);
    service_response(service.fido2_list_keys(bearer_token(&headers).as_deref()))
}

pub(crate) async fn fido2_setup(
    State(state): State<Arc<AppState>>,
    ValidatedJson(body): ValidatedJson<Fido2SetupRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.fido2_setup(body).await)
}

pub(crate) async fn fido2_register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<Fido2RegisterRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .fido2_register(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn fido2_unlock(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<Fido2UnlockRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .fido2_unlock(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn fido2_remove(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<Fido2RemoveRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .fido2_remove(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
