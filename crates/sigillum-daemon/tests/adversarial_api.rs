//! Adversarial daemon API boundary checks.
//!
//! These tests exercise HTTP extraction, authentication, and validation edges
//! that library-level serde or service tests do not cover on their own.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::util::ServiceExt;

fn test_app() -> (axum::Router, TempDir) {
    let dir = TempDir::new().unwrap();
    let (app, _state) = sigillum_daemon::build_router(dir.path().to_path_buf(), 0);
    (app, dir)
}

async fn raw_request(
    app: &axum::Router,
    method: Method,
    path: &str,
    body: impl Into<Body>,
    content_type: Option<&str>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = app
        .clone()
        .oneshot(builder.body(body.into()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn post_json(
    app: &axum::Router,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    raw_request(
        app,
        Method::POST,
        path,
        serde_json::to_vec(&body).unwrap(),
        Some("application/json"),
        token,
    )
    .await
}

async fn init_session(app: &axum::Router) -> String {
    let (status, body) = post_json(
        app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "adversarial",
            "threshold": 1,
            "passphrase": "adversarial-passphrase-123"
        }),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "init should succeed: {body:?}");
    body["session_token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn malformed_json_bodies_are_rejected_without_internal_errors() {
    let (app, _dir) = test_app();

    for path in [
        "/api/compartment/init",
        "/api/unlock",
        "/api/api-keys/set",
        "/api/profiles/evm/upsert",
        "/api/wallets/eth-stealth/generate",
    ] {
        let (status, body) = raw_request(
            &app,
            Method::POST,
            path,
            r#"{"unterminated": "#,
            Some("application/json"),
            Some("not-a-session"),
        )
        .await;
        assert!(
            status.is_client_error(),
            "{path} should reject malformed JSON with a client error, got {status}: {body:?}"
        );
        assert_ne!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{path} must not surface malformed JSON as 500"
        );
    }
}

#[tokio::test]
async fn json_routes_reject_unexpected_content_types_and_empty_bodies() {
    let (app, _dir) = test_app();

    let (plain_status, plain_body) = raw_request(
        &app,
        Method::POST,
        "/api/compartment/init",
        r#"{"id":0,"label":"bad","threshold":1,"passphrase":"adversarial-passphrase-123"}"#,
        Some("text/plain"),
        None,
    )
    .await;
    assert!(
        plain_status.is_client_error(),
        "unexpected content type should be rejected, got {plain_status}: {plain_body:?}"
    );

    let (empty_status, empty_body) = raw_request(
        &app,
        Method::POST,
        "/api/compartment/init",
        Body::empty(),
        Some("application/json"),
        None,
    )
    .await;
    assert!(
        empty_status.is_client_error(),
        "empty JSON body should be rejected, got {empty_status}: {empty_body:?}"
    );
    assert_ne!(empty_status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn protected_routes_fail_closed_for_missing_or_malformed_tokens() {
    let (app, _dir) = test_app();

    let protected_gets = ["/api/api-keys", "/api/secrets", "/api/diagnostics"];
    for path in protected_gets {
        let (missing_status, missing_body) =
            raw_request(&app, Method::GET, path, Body::empty(), None, None).await;
        assert_eq!(
            missing_status,
            StatusCode::UNAUTHORIZED,
            "{path} missing token should be unauthorized: {missing_body:?}"
        );

        let (bad_status, bad_body) =
            raw_request(&app, Method::GET, path, Body::empty(), None, Some("%%%")).await;
        assert_eq!(
            bad_status,
            StatusCode::UNAUTHORIZED,
            "{path} malformed token should be unauthorized: {bad_body:?}"
        );
    }

    let (set_status, set_body) = post_json(
        &app,
        "/api/api-keys/set",
        json!({"key": "probe", "value": "secret"}),
        None,
    )
    .await;
    assert_eq!(
        set_status,
        StatusCode::UNAUTHORIZED,
        "mutating secret route must fail closed without token: {set_body:?}"
    );
}

#[tokio::test]
async fn invalid_lifecycle_and_compartment_values_stay_client_errors() {
    let (app, _dir) = test_app();

    for body in [
        json!({
            "id": 0,
            "label": "zero-threshold",
            "threshold": 0,
            "passphrase": "adversarial-passphrase-123"
        }),
        json!({
            "id": 0,
            "label": "short-passphrase",
            "threshold": 1,
            "passphrase": "short"
        }),
        json!({
            "id": -1,
            "label": "negative-id",
            "threshold": 1,
            "passphrase": "adversarial-passphrase-123"
        }),
    ] {
        let (status, response_body) = post_json(&app, "/api/compartment/init", body, None).await;
        assert!(
            status.is_client_error(),
            "invalid compartment init should be a client error, got {status}: {response_body:?}"
        );
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[tokio::test]
async fn evm_boundary_inputs_reject_bad_hex_and_addresses_before_rpc_use() {
    let (app, _dir) = test_app();
    let token = init_session(&app).await;

    let bad_requests = [
        (
            "/api/evm/balance",
            json!({
                "provider": {"rpc_url": "https://rpc.invalid"},
                "address": "0x123"
            }),
        ),
        (
            "/api/evm/erc20-balance",
            json!({
                "provider": {"rpc_url": "https://rpc.invalid"},
                "token_address": "0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
                "owner_address": "0x0000000000000000000000000000000000000000"
            }),
        ),
        (
            "/api/deposits/eth-stealth/native",
            json!({
                "label": "bad-receiver",
                "chain_id": 1,
                "receiver_address": "0x123",
                "token_symbol": "ETH",
                "amount_wei_hex": "0x1"
            }),
        ),
    ];

    for (path, body) in bad_requests {
        let (status, response_body) = post_json(&app, path, body, Some(&token)).await;
        assert!(
            status.is_client_error(),
            "{path} should reject invalid EVM boundary input, got {status}: {response_body:?}"
        );
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
