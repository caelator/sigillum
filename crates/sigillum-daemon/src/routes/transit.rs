//! Route handlers for transit encryption: encrypt, decrypt, and HMAC operations.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{TransitDecryptRequest, TransitEncryptRequest, TransitHmacRequest};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn transit_encrypt(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TransitEncryptRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.transit_encrypt(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn transit_decrypt(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TransitDecryptRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.transit_decrypt(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn transit_hmac(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<TransitHmacRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.transit_hmac(bearer_token(&headers).as_deref(), body))
}
