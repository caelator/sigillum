//! Route handlers for API key and secret management.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{
    KeyOnlyRequest, KeyValueRequest, SecretResolveBatchRequest, SecretsPushRequest,
};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_api_keys(bearer_token(&headers).as_deref()))
}

pub(crate) async fn get_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyOnlyRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.get_api_key(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn set_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyValueRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .set_api_key(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyOnlyRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_api_key(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn list_secrets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_secrets(bearer_token(&headers).as_deref()))
}

pub(crate) async fn get_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyOnlyRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.get_secret(bearer_token(&headers).as_deref(), body))
}

pub(crate) async fn set_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyValueRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .set_secret(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn delete_secret(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<KeyOnlyRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .delete_secret(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn secrets_push(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SecretsPushRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .push_secret(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn resolve_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SecretResolveBatchRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.resolve_secret_batch(bearer_token(&headers).as_deref(), body))
}
