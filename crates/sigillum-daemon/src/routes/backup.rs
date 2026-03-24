//! Route handlers for vault backup export and snapshot restore operations.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{PassphraseRequest, SetupResetRequest, SnapshotRestoreRequest};

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn backup_export(
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
            .backup_export(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn backup_restore(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SnapshotRestoreRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .backup_restore(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn setup_reset(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetupResetRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.setup_reset(body).await)
}
