//! Route handler for vault maintenance operations.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::MaintenanceRunRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn run_maintenance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<MaintenanceRunRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .run_maintenance(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
