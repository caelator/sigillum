//! Wire-level tests for the structured error envelope (`code` + `fields`).
//!
//! The daemon maps every `ServiceError` to an HTTP status plus a stable
//! machine-readable code (see `sigillum_api::error_codes`). These tests pin
//! the envelope shape for representative statuses at the HTTP boundary.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

use sigillum_daemon::{AppState, build_router};

fn test_app_with_state() -> (axum::Router, Arc<AppState>, TempDir) {
    let dir = TempDir::new().unwrap();
    let (app, state) = build_router(dir.path().to_path_buf(), 0).expect("router should initialize");
    (app, state, dir)
}

async fn post_request(
    app: &axum::Router,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(Body::from(body_bytes)).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn get_request(app: &axum::Router, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("GET")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(Body::empty()).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn wrong_passphrase_unlock_maps_to_unauthorized_code() {
    let (app, _state, _dir) = test_app_with_state();

    let (status, body) = post_request(
        &app,
        "/api/unlock",
        json!({ "passphrase": "definitely-wrong" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], json!("unauthorized"));
    assert!(body["error"].as_str().unwrap().contains("passphrase"));
    assert!(body.get("fields").is_none());
}

#[tokio::test]
async fn dto_validation_failure_maps_to_validation_failed_code() {
    let (app, _state, _dir) = test_app_with_state();

    // PassphraseRequest caps at 1024 bytes; the DTO has no per-field
    // breakdown, so `fields` must be absent.
    let (status, body) = post_request(
        &app,
        "/api/unlock",
        json!({ "passphrase": "x".repeat(2048) }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], json!("validation_failed"));
    assert!(body["error"].as_str().unwrap().contains("passphrase"));
    assert!(body.get("fields").is_none());
}

#[tokio::test]
async fn startup_gate_maps_to_unavailable_code() {
    let (app, state, _dir) = test_app_with_state();
    state.mark_startup_failed("boom");

    let (status, body) = get_request(&app, "/api/status", None).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["code"], json!("unavailable"));
    assert_eq!(body["error"], json!("Startup recovery is not ready."));
}

#[tokio::test]
async fn upgraded_dto_reports_field_level_validation_errors() {
    let (app, _state, _dir) = test_app_with_state();

    let (init_status, init_body) = post_request(
        &app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "test-passphrase-123"
        }),
        None,
    )
    .await;
    assert_eq!(init_status, StatusCode::OK, "init: {init_body:?}");
    let token = init_body["session_token"].as_str().unwrap();

    let (status, body) = post_request(
        &app,
        "/api/profiles/evm/upsert",
        json!({
            "name": "n".repeat(300),
            "rpc_url": "u".repeat(2100),
            "chain_id": 1,
            "max_fee_per_gas_hex": "f".repeat(4100)
        }),
        Some(token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], json!("validation_failed"));
    // Top-level message stays the first field failure (legacy contract).
    assert!(body["error"].as_str().unwrap().contains("name"));
    let fields = body["fields"].as_array().expect("fields array present");
    let paths: Vec<&str> = fields
        .iter()
        .map(|field| field["field"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["name", "rpc_url", "max_fee_per_gas_hex"]);
    assert!(
        fields[0]["message"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum length")
    );
}

#[tokio::test]
async fn gas_topup_opt_in_without_cap_reports_the_cap_field() {
    let (app, _state, _dir) = test_app_with_state();

    let (init_status, init_body) = post_request(
        &app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "test-passphrase-123"
        }),
        None,
    )
    .await;
    assert_eq!(init_status, StatusCode::OK, "init: {init_body:?}");
    let token = init_body["session_token"].as_str().unwrap();

    let (status, body) = post_request(
        &app,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_gas_topups": true
        }),
        Some(token),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], json!("validation_failed"));
    assert_eq!(
        body["error"],
        json!("max_gas_topup_wei_hex is required when allow_gas_topups is true")
    );
    assert_eq!(
        body["fields"],
        json!([{
            "field": "max_gas_topup_wei_hex",
            "message": "max_gas_topup_wei_hex is required when allow_gas_topups is true"
        }])
    );
}
