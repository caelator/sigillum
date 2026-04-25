//! Route handler for the audit log endpoint (`/api/audit`).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use sigillum_api::request::RunAuditRequest;

use crate::AppState;
use crate::service::SigillumService;

use super::{bearer_token, service_response, validated};

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    tail: Option<usize>,
    limit: Option<usize>,
    kind: Option<String>,
    since: Option<u64>,
    key: Option<String>,
}

pub(crate) async fn audit_recent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> Response {
    let default_limit = state.runtime_policy().audit_default_limit;
    let service = SigillumService::new(state);
    let tail = query.tail.or(query.limit).unwrap_or(default_limit);
    service_response(
        service
            .audit_recent(
                bearer_token(&headers).as_deref(),
                crate::audit_db::AuditQuery {
                    tail,
                    kind: query.kind,
                    since: query.since,
                    key: query.key,
                },
            )
            .await,
    )
}

pub(crate) async fn audit_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RunAuditRequest>,
) -> Response {
    let body = match validated(Json(body)) {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    let service = SigillumService::new(state);
    service_response(service.record_run_audit(bearer_token(&headers).as_deref(), body))
}
