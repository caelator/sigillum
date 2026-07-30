//! Route handlers for the diagnostics endpoints (`/api/diagnostics`,
//! `/api/selfcheck/run`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::SelfCheckRunRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn diagnostics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.diagnostics(bearer_token(&headers).as_deref()))
}

pub(crate) async fn selfcheck_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<SelfCheckRunRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .run_self_check(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
