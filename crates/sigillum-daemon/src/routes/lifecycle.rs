//! Route handlers for vault lifecycle: status, unlock, lock, and session revocation.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::PassphraseRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn get_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let service = SigillumService::new(state);
    service_response(service.status(bearer_token(&headers).as_deref()))
}

pub(crate) async fn post_unlock(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PassphraseRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .unlock_with_passphrase(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn post_lock(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let service = SigillumService::new(state);
    service_response(service.lock_all(bearer_token(&headers).as_deref()).await)
}

pub(crate) async fn post_revoke_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .revoke_session(bearer_token(&headers).as_deref())
            .await,
    )
}
