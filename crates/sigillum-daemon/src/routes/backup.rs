//! Route handlers for vault backup export and snapshot restore operations.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::{PassphraseRequest, SetupResetRequest, SnapshotRestoreRequest};

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn backup_export(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<PassphraseRequest>,
) -> Response {
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
    ValidatedJson(body): ValidatedJson<SnapshotRestoreRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .backup_restore(bearer_token(&headers).as_deref(), body)
            .await,
    )
}

pub(crate) async fn setup_reset(
    State(state): State<Arc<AppState>>,
    ValidatedJson(body): ValidatedJson<SetupResetRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.setup_reset(body).await)
}
