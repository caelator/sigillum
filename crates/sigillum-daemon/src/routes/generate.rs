//! Route handlers for secret generation.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use sigillum_api::request::GenerateStoreRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{ValidatedJson, bearer_token, service_response};

pub(crate) async fn generate_store(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ValidatedJson(body): ValidatedJson<GenerateStoreRequest>,
) -> Response {
    let service = SigillumService::new(state);
    service_response(
        service
            .generate_and_store(bearer_token(&headers).as_deref(), body)
            .await,
    )
}
