//! Route handler for the audit log endpoint (`/api/audit`).

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response};

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    limit: Option<usize>,
}

pub(crate) async fn audit_recent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Response {
    let default_limit = state.runtime_policy().audit_default_limit;
    let service = SigillumService::new(state);
    service_response(
        service
            .audit_recent(
                bearer_token(&headers).as_deref(),
                query.limit.unwrap_or(default_limit),
            )
            .await,
    )
}
