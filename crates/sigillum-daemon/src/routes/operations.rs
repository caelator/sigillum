//! Route handlers for the background operations resource.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response};

/// Bound on operation-id path parameters, mirroring the DTO id length limit
/// enforced for body-carried ids.
const MAX_OPERATION_ID_LEN: usize = 256;

#[allow(clippy::result_large_err)]
fn validated_operation_id(id: String) -> Result<String, Response> {
    let id = id.trim().to_string();
    if id.is_empty() || id.len() > MAX_OPERATION_ID_LEN {
        return Err(super::err(
            axum::http::StatusCode::BAD_REQUEST,
            sigillum_api::error_codes::BAD_REQUEST,
            "Invalid operation id.",
        ));
    }
    Ok(id)
}

pub(crate) async fn list_operations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let service = SigillumService::new(state);
    service_response(service.list_operations(bearer_token(&headers).as_deref()))
}

pub(crate) async fn get_operation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let id = match validated_operation_id(id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.get_operation(bearer_token(&headers).as_deref(), &id))
}

pub(crate) async fn cancel_operation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let id = match validated_operation_id(id) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.cancel_operation(bearer_token(&headers).as_deref(), &id))
}
