//! Health check endpoint.

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use crate::state::AppState;

/// `GET /api/v1/health` — gateway + daemon health.
pub async fn health_check(State(state): State<AppState>) -> Json<Value> {
    let daemon_ok = state
        .daemon_healthy
        .load(std::sync::atomic::Ordering::Relaxed);

    // Also do a live check
    let live = state.daemon.status().await.is_ok();

    Json(json!({
        "gateway": "ok",
        "daemon": if live { "ok" } else { "unreachable" },
        "daemon_healthy": daemon_ok,
        "daemon_url": state.config.daemon_url,
    }))
}
