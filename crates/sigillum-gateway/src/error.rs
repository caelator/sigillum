//! Gateway error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Gateway-level errors.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Missing project scope: {0}")]
    MissingScope(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Daemon error: {0}")]
    Daemon(#[from] sigillum_client::ClientError),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            GatewayError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            GatewayError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            GatewayError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".into()),
            GatewayError::MissingScope(scope) => {
                let body = json!({ "error": "missing_scope", "required": scope });
                return (StatusCode::FORBIDDEN, axum::Json(body)).into_response();
            }
            GatewayError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            GatewayError::Daemon(err) => {
                tracing::error!("Daemon communication error: {err}");
                (StatusCode::BAD_GATEWAY, "Daemon unavailable".into())
            }
            GatewayError::Database(err) => {
                tracing::error!("Database error: {err}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".into(),
                )
            }
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
