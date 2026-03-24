//! Route handler for vault maintenance operations.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::MaintenanceRunRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

pub(crate) async fn run_maintenance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MaintenanceRunRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(b) => b,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(
        service
            .run_maintenance(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
