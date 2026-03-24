//! Route handler for the diagnostics endpoint (`/api/diagnostics`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response};

pub(crate) async fn diagnostics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.diagnostics(bearer_token(&headers).as_deref()))
}
