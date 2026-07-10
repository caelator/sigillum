#![allow(clippy::unnecessary_mut_passed)]
#![allow(clippy::single_match)]
#![allow(clippy::redundant_pattern_matching)]

//! Integration tests for Sigillum daemon routes (H4) and FIDO2 configuration (H5).
//!
//! H4: Route Handler Integration Tests
//! Tests for lifecycle, secrets, and compartment routes with proper HTTP status codes
//! and JSON response validation.
//!
//! H5: FIDO2 Mock Transport
//! Tests for FIDO2 config and crypto layers without requiring hardware access.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

use sigillum_daemon::{AppState, build_router};

// ============================================================================
// H4: Route Handler Integration Tests
// ============================================================================

/// Helper to build test app with isolated temp state.
fn test_app() -> (axum::Router, TempDir) {
    let (app, _state, dir) = test_app_with_state();
    (app, dir)
}

fn test_app_with_state() -> (axum::Router, Arc<AppState>, TempDir) {
    let dir = TempDir::new().unwrap();
    let (app, state) = build_router(dir.path().to_path_buf(), 0).expect("router should initialize");
    (app, state, dir)
}

/// Helper to make a GET request to the test app.
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

/// Helper to make a POST request to the test app.
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

// ── Lifecycle Tests ─────────────────────────────────────────────────────

/// GET /api/status returns a minimal locked response when no compartments are unlocked.
#[tokio::test]
async fn test_get_status_locked_no_token() {
    let (app, _dir) = test_app();

    let (status, body) = get_request(&app, "/api/status", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["locked"], json!(true));
    assert_eq!(body["unlocked_compartments"], json!([]));
}

/// GET /api/status with an invalid token returns the same minimal locked response.
#[tokio::test]
async fn test_get_status_invalid_token() {
    let (app, _dir) = test_app();

    let (status, body) = get_request(&app, "/api/status", Some("invalid-token")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["locked"], json!(true));
    assert_eq!(body["unlocked_compartments"], json!([]));
}

#[tokio::test]
async fn startup_health_stays_open_while_non_health_routes_are_gated() {
    let (app, state, _dir) = test_app_with_state();
    state.mark_startup_failed("boom");

    let (health_status, health_body) = get_request(&app, "/api/health", None).await;
    assert_eq!(health_status, StatusCode::OK);
    assert_eq!(health_body["status"], json!("starting"));
    assert_eq!(health_body["startup_error"], json!("boom"));

    let (status_status, status_body) = get_request(&app, "/api/status", None).await;
    assert_eq!(status_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        status_body["error"],
        json!("Startup recovery is not ready.")
    );
}

/// POST /api/unlock with wrong passphrase returns 401.
#[tokio::test]
async fn test_post_unlock_wrong_passphrase() {
    let (mut app, _dir) = test_app();

    let (status, body) = post_request(
        &mut app,
        "/api/unlock",
        json!({ "passphrase": "wrong-passphrase" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.get("error").is_some());
}

/// POST /api/compartment/init initializes and returns session token.
#[tokio::test]
async fn test_post_compartment_init_returns_token() {
    let (mut app, _dir) = test_app();

    let (status, body) = post_request(
        &mut app,
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

    assert_eq!(status, StatusCode::OK, "init should succeed: {body:?}");
    let token = body
        .get("session_token")
        .and_then(|v| v.as_str())
        .expect("session_token field missing");
    assert!(!token.is_empty());
}

#[tokio::test]
async fn capability_session_is_default_deny_outside_explicit_scopes() {
    let (mut app, _dir) = test_app();

    let (init_status, init_body) = post_request(
        &mut app,
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
    assert_eq!(
        init_status,
        StatusCode::OK,
        "init should succeed: {init_body:?}"
    );
    let full_token = init_body["session_token"].as_str().unwrap();

    let (provider_status, provider_body) = post_request(
        &mut app,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:1",
            "chain_id": 1
        }),
        Some(full_token),
    )
    .await;
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider setup should succeed: {provider_body:?}"
    );

    let (profile_status, profile_body) = post_request(
        &mut app,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet"
        }),
        Some(full_token),
    )
    .await;
    assert_eq!(
        profile_status,
        StatusCode::OK,
        "wallet profile setup should succeed: {profile_body:?}"
    );

    let (mint_status, mint_body) = post_request(
        &mut app,
        "/api/auth/capability",
        json!({
            "scopes": ["wallet_profiles:read"],
            "ttl_secs": 60
        }),
        Some(full_token),
    )
    .await;
    assert_eq!(
        mint_status,
        StatusCode::OK,
        "mint should succeed: {mint_body:?}"
    );
    let scoped_token = mint_body["session_token"].as_str().unwrap();

    let (wallet_status, _wallet_body) =
        get_request(&app, "/api/profiles/eth-stealth", Some(scoped_token)).await;
    assert_eq!(wallet_status, StatusCode::OK);

    let (provider_status, provider_body) =
        get_request(&app, "/api/profiles/evm", Some(scoped_token)).await;
    assert_eq!(provider_status, StatusCode::FORBIDDEN);
    assert_eq!(
        provider_body["error"],
        json!("Missing daemon capability scope: evm_providers:read")
    );

    let (status_status, status_body) = get_request(&app, "/api/status", Some(scoped_token)).await;
    assert_eq!(status_status, StatusCode::OK);
    assert_eq!(status_body["locked"], json!(true));
    assert_eq!(status_body["unlocked_compartments"], json!([]));
    assert!(status_body["active_compartment"].is_null());

    let sensitive_get_routes = [
        "/api/diagnostics",
        "/api/audit",
        "/api/compartment/list",
        "/api/api-keys",
        "/api/secrets",
        "/api/profiles/eth-xpub",
        "/api/profiles/eth-seed",
        "/api/inventory/wallets",
        "/api/chains",
        "/api/inventory/watch-addresses",
        "/api/inventory/token-registry",
        "/api/discovery/jobs",
        "/api/risk/findings",
        "/api/risk/catalog",
        "/api/plans/consolidation",
        "/api/treasury/overview",
        "/api/receiving/overview",
        "/api/treasury/policy",
        "/api/treasury/receive-addresses",
        "/api/treasury/parties",
        "/api/queue/jobs",
        "/api/fido2/status",
    ];

    for path in sensitive_get_routes {
        let (route_status, route_body) = get_request(&app, path, Some(scoped_token)).await;
        assert_eq!(
            route_status,
            StatusCode::FORBIDDEN,
            "capability session unexpectedly reached {path}: {route_body:?}"
        );
        assert_eq!(
            route_body["error"],
            json!("A full daemon session is required for this operation."),
            "unexpected authorization response for {path}: {route_body:?}"
        );
    }

    let sensitive_post_routes = [
        (
            "/api/secrets/set",
            json!({"key": "capability-cannot-write", "value": "blocked"}),
        ),
        (
            "/api/wallets/eth-xpub/export",
            json!({"wallet_profile": "treasury-receive"}),
        ),
        ("/api/treasury/policy/update", json!({"enabled": false})),
        ("/api/queue/process", json!({})),
        ("/api/queue/pause", json!({})),
        (
            "/api/backup/export",
            json!({"passphrase": "capability-backup-passphrase"}),
        ),
        ("/api/compartment/switch", json!({"id": 0})),
        ("/api/lock", json!({})),
        (
            "/api/auth/capability",
            json!({"scopes": ["wallet_profiles:read"], "ttl_secs": 60}),
        ),
        (
            "/api/biometric/enroll",
            json!({
                "public_key_hex": "02",
                "passphrase": "capability-cannot-enroll"
            }),
        ),
    ];

    for (path, request_body) in sensitive_post_routes {
        let (route_status, route_body) =
            post_request(&app, path, request_body, Some(scoped_token)).await;
        assert_eq!(
            route_status,
            StatusCode::FORBIDDEN,
            "capability session unexpectedly reached {path}: {route_body:?}"
        );
        assert_eq!(
            route_body["error"],
            json!("A full daemon session is required for this operation."),
            "unexpected authorization response for {path}: {route_body:?}"
        );
    }

    let (create_mint_status, create_mint_body) = post_request(
        &app,
        "/api/auth/capability",
        json!({"scopes": ["deposits:create"], "ttl_secs": 60}),
        Some(full_token),
    )
    .await;
    assert_eq!(create_mint_status, StatusCode::OK);
    let create_token = create_mint_body["session_token"].as_str().unwrap();

    let erc20_request = json!({
        "wallet_profile": "payments",
        "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        "expected_amount_hex": "0x1",
        "auto_queue_sweep": false
    });
    let (create_status, create_body) = post_request(
        &app,
        "/api/deposits/eth-stealth/create-erc20",
        erc20_request.clone(),
        Some(create_token),
    )
    .await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "deposits:create should authorize ERC-20 creation: {create_body:?}"
    );

    let mut auto_queue_request = erc20_request.clone();
    auto_queue_request["auto_queue_sweep"] = json!(true);
    let (auto_queue_status, auto_queue_body) = post_request(
        &app,
        "/api/deposits/eth-stealth/create-erc20",
        auto_queue_request,
        Some(create_token),
    )
    .await;
    assert_eq!(auto_queue_status, StatusCode::FORBIDDEN);
    assert_eq!(
        auto_queue_body["error"],
        json!("Missing daemon capability scope: queue:enqueue-sweep")
    );

    let (delete_mint_status, delete_mint_body) = post_request(
        &app,
        "/api/auth/capability",
        json!({"scopes": ["deposits:delete"], "ttl_secs": 60}),
        Some(full_token),
    )
    .await;
    assert_eq!(delete_mint_status, StatusCode::OK);
    let delete_token = delete_mint_body["session_token"].as_str().unwrap();
    let (delete_scope_status, delete_scope_body) = post_request(
        &app,
        "/api/deposits/eth-stealth/create-erc20",
        erc20_request,
        Some(delete_token),
    )
    .await;
    assert_eq!(delete_scope_status, StatusCode::FORBIDDEN);
    assert_eq!(
        delete_scope_body["error"],
        json!("Missing daemon capability scope: deposits:create")
    );
}

/// After init, GET /api/status with valid token returns unlocked status.
#[tokio::test]
async fn test_get_status_unlocked_with_valid_token() {
    let (mut app, _dir) = test_app();

    // Step 1: Initialize compartment
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "test-pass-456"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .unwrap();

    // Step 2: Get status with token
    let (status, body) = get_request(&mut app, "/api/status", Some(token)).await;

    assert_eq!(status, StatusCode::OK);
    // Status should indicate compartments are present/unlocked
    assert!(body.is_object());
}

/// GET /api/secrets without auth token returns 401.
#[tokio::test]
async fn test_get_secrets_no_token() {
    let (mut app, _dir) = test_app();

    let (status, body) = get_request(&mut app, "/api/secrets", None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.get("error").is_some());
}

/// GET /api/secrets with invalid token returns 401.
#[tokio::test]
async fn test_get_secrets_invalid_token() {
    let (mut app, _dir) = test_app();

    let (status, body) = get_request(&mut app, "/api/secrets", Some("invalid-token")).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.get("error").is_some());
}

/// GET /api/secrets with valid token after init succeeds.
#[tokio::test]
async fn test_get_secrets_with_valid_token() {
    let (mut app, _dir) = test_app();

    // Initialize
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "secret-pass-789"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .unwrap();

    // Get secrets
    let (status, body) = get_request(&mut app, "/api/secrets", Some(token)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.is_object() || body.is_array());
}

/// POST /api/fido2/pin/set rejects short PINs before touching hardware.
#[tokio::test]
async fn test_fido2_set_pin_rejects_short_pin() {
    let (mut app, _dir) = test_app();

    let (status, body) = post_request(
        &mut app,
        "/api/fido2/pin/set",
        json!({ "new_pin": "123" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        json!("FIDO2 PIN must be at least 4 characters long.")
    );
}

/// POST /api/fido2/pin/set requires auth once the daemon is initialized.
#[tokio::test]
async fn test_fido2_set_pin_requires_auth_after_init() {
    let (mut app, _dir) = test_app();

    let (init_status, _init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "auth-required-passphrase"
        }),
        None,
    )
    .await;
    assert_eq!(init_status, StatusCode::OK);

    let (status, body) = post_request(
        &mut app,
        "/api/fido2/pin/set",
        json!({ "new_pin": "1234" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("session token")
    );
}

/// POST /api/api-keys/set stores a secret, POST /api/api-keys/get retrieves it.
#[tokio::test]
async fn test_set_and_get_api_key_roundtrip() {
    let (mut app, _dir) = test_app();

    // Initialize
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "api-key-pass"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .unwrap();

    // Set API key
    let (set_status, set_body) = post_request(
        &mut app,
        "/api/api-keys/set",
        json!({ "key": "github_token", "value": "ghp_test_abc123" }),
        Some(token),
    )
    .await;

    assert_eq!(
        set_status,
        StatusCode::OK,
        "set should succeed: {set_body:?}"
    );

    // Get the stored API key
    let (get_status, get_body) = post_request(
        &mut app,
        "/api/api-keys/get",
        json!({ "key": "github_token" }),
        Some(token),
    )
    .await;

    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(
        get_body.get("value").and_then(|v| v.as_str()),
        Some("ghp_test_abc123")
    );
}

/// POST /api/lock locks compartments and invalidates sessions.
#[tokio::test]
async fn test_post_lock_invalidates_session() {
    let (mut app, _dir) = test_app();

    // Initialize
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "lock-test-pass"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .unwrap();

    // Verify token works before lock
    let (pre_lock_status, _) = get_request(&mut app, "/api/secrets", Some(token)).await;
    assert_eq!(pre_lock_status, StatusCode::OK);

    // Lock all compartments
    let (lock_status, lock_body) =
        post_request(&mut app, "/api/lock", json!({}), Some(token)).await;

    assert_eq!(
        lock_status,
        StatusCode::OK,
        "lock should succeed: {lock_body:?}"
    );

    // Verify token no longer works
    let (post_lock_status, _) = get_request(&mut app, "/api/secrets", Some(token)).await;
    assert_eq!(post_lock_status, StatusCode::UNAUTHORIZED);
}

/// POST /api/unlock after init with correct passphrase succeeds.
#[tokio::test]
async fn test_post_unlock_after_lock_succeeds() {
    let (mut app, _dir) = test_app();

    let passphrase = "unlock-test-correct";

    // Initialize
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": passphrase
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .unwrap();

    // Lock
    let (lock_status, _) = post_request(&mut app, "/api/lock", json!({}), Some(token)).await;
    assert_eq!(lock_status, StatusCode::OK);

    // Unlock with correct passphrase
    let (unlock_status, unlock_body) = post_request(
        &mut app,
        "/api/unlock",
        json!({ "passphrase": passphrase }),
        None,
    )
    .await;

    assert_eq!(unlock_status, StatusCode::OK);
    let new_token = unlock_body.get("session_token").and_then(|v| v.as_str());
    assert!(new_token.is_some());
}

/// POST /api/unlock re-authenticates when the vault is still unlocked but the session is gone.
#[tokio::test]
async fn test_post_unlock_reauthenticates_when_session_is_missing() {
    let (mut app, _dir) = test_app();

    // Initialize
    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "dup-unlock-pass"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .expect("session token should be present");

    let (revoke_status, revoke_body) =
        post_request(&mut app, "/api/session/revoke", json!({}), Some(token)).await;
    assert_eq!(revoke_status, StatusCode::OK);
    assert_eq!(revoke_body["requires_reauth"], json!(true));

    let (unlock_status, unlock_body) = post_request(
        &mut app,
        "/api/unlock",
        json!({ "passphrase": "dup-unlock-pass" }),
        None,
    )
    .await;

    assert_eq!(unlock_status, StatusCode::OK);
    assert_eq!(unlock_body["status"], json!("unlocked"));
    assert!(
        unlock_body
            .get("session_token")
            .and_then(|v| v.as_str())
            .is_some()
    );
}

/// POST /api/unlock already unlocked with a valid session returns 409.
#[tokio::test]
async fn test_post_unlock_already_unlocked_with_valid_session_returns_409() {
    let (mut app, _dir) = test_app();

    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "dup-unlock-pass"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .expect("session token should be present");

    let (unlock_status, _) = post_request(
        &mut app,
        "/api/unlock",
        json!({ "passphrase": "dup-unlock-pass" }),
        Some(token),
    )
    .await;

    assert_eq!(unlock_status, StatusCode::CONFLICT);
}

/// POST /api/setup/reset clears partial local setup artifacts before initialization.
#[tokio::test]
async fn test_post_setup_reset_clears_partial_uninitialized_state() {
    let (mut app, dir) = test_app();

    std::fs::create_dir_all(dir.path().join("compartments/0")).unwrap();
    std::fs::create_dir_all(dir.path().join(".ops")).unwrap();
    std::fs::write(dir.path().join("compartments/0/meta.enc"), b"partial").unwrap();
    std::fs::write(dir.path().join("fido2_keys.json"), b"{\"keys\":[]}").unwrap();
    std::fs::write(dir.path().join(".ops/pending.json"), b"{}").unwrap();

    let (status, body) = post_request(
        &mut app,
        "/api/setup/reset",
        json!({ "confirmation": "RESET LOCAL SIGILLUM DATA" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("reset"));
    assert!(dir.path().exists());
    assert!(!dir.path().join("compartments").exists());
    assert!(!dir.path().join("fido2_keys.json").exists());
    assert!(!dir.path().join(".ops").exists());
    assert!(!dir.path().join(".initialized").exists());

    // Reset archives instead of deleting: the partial artifacts must survive
    // in the timestamped sibling directory reported by the response.
    let archived_to = body["archived_to"]
        .as_str()
        .expect("reset response should report the archive path");
    let archive = std::path::PathBuf::from(archived_to);
    assert!(archive.exists());
    assert!(archive.join("compartments/0/meta.enc").exists());
    assert!(archive.join("fido2_keys.json").exists());
    std::fs::remove_dir_all(&archive).unwrap();

    let (status_after, body_after) = get_request(&app, "/api/status", None).await;
    assert_eq!(status_after, StatusCode::OK);
    assert_eq!(body_after["initialized"], json!(false));
}

/// POST /api/setup/reset can recover an initialized local vault back to first-run setup.
#[tokio::test]
async fn test_post_setup_reset_clears_initialized_local_data() {
    let (mut app, dir) = test_app();

    let (init_status, init_body) = post_request(
        &mut app,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "reset-me-now"
        }),
        None,
    )
    .await;

    assert_eq!(init_status, StatusCode::OK);
    let token = init_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .expect("session token should be present");

    let (set_status, _) = post_request(
        &mut app,
        "/api/api-keys/set",
        json!({ "key": "test_key", "value": "test_value" }),
        Some(token),
    )
    .await;
    assert_eq!(set_status, StatusCode::OK);
    assert!(dir.path().join(".initialized").exists());

    let (status, body) = post_request(
        &mut app,
        "/api/setup/reset",
        json!({ "confirmation": "RESET LOCAL SIGILLUM DATA" }),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("reset"));
    assert!(!dir.path().join(".initialized").exists());
    assert!(!dir.path().join("compartments").exists());

    // The initialized vault is archived, not destroyed.
    let archived_to = body["archived_to"]
        .as_str()
        .expect("reset response should report the archive path");
    let archive = std::path::PathBuf::from(archived_to);
    assert!(archive.join(".initialized").exists());
    assert!(archive.join("compartments").exists());
    std::fs::remove_dir_all(&archive).unwrap();

    let (status_after, body_after) = get_request(&app, "/api/status", None).await;
    assert_eq!(status_after, StatusCode::OK);
    assert_eq!(body_after["initialized"], json!(false));
    assert_eq!(body_after["locked"], json!(true));

    let (secrets_status, _) = get_request(&app, "/api/secrets", Some(token)).await;
    assert_eq!(secrets_status, StatusCode::UNAUTHORIZED);
}

// ============================================================================
// H5: FIDO2 Configuration and Crypto Tests
// ============================================================================

#[cfg(test)]
mod fido2_config_tests {
    use sigillum_fido2::config::{
        CompartmentMeta, Fido2Config, RegisteredKey, SHARD_SLOTS, generate_dummy_shards,
        load_config, next_compartment_id, save_config, validate_thresholds,
    };
    use tempfile::TempDir;

    /// Test FIDO2 config serialization and deserialization roundtrip.
    #[test]
    fn test_fido2_config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fido2_keys.json");

        let config = Fido2Config {
            total_shares: 2,
            keys: vec![RegisteredKey {
                label: "yubikey-1".to_string(),
                credential_id_hex: "aabbccdd".to_string(),
                public_key_der_hex: "eeff0011".to_string(),
                public_key_pem: "-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----"
                    .to_string(),
                shards: vec!["shard1".to_string(), "shard2".to_string()],
                registered_at: "2026-03-10T10:00:00Z".to_string(),
            }],
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.total_shares, 2);
        assert_eq!(loaded.keys.len(), 1);
        assert_eq!(loaded.keys[0].label, "yubikey-1");
        assert_eq!(loaded.keys[0].credential_id_hex, "aabbccdd");
    }

    /// Test loading missing config returns default empty config.
    #[test]
    fn test_load_missing_config_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent_fido2.json");

        let config = load_config(&path).unwrap();

        assert!(config.keys.is_empty());
        assert_eq!(config.total_shares, 0);
    }

    /// Test malformed config JSON returns error.
    #[test]
    fn test_malformed_config_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{invalid json").unwrap();

        let result = load_config(&path);

        assert!(matches!(result, Err(_)));
    }

    /// Test validate_thresholds rejects duplicate thresholds.
    #[test]
    fn test_validate_thresholds_rejects_duplicates() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "vault-1".to_string(),
                threshold: 2,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 1,
                label: "vault-2".to_string(),
                threshold: 2,
                passphrase_mode: None,
            },
        ];

        let result = validate_thresholds(&metas);

        assert!(result.is_err());
    }

    /// Test validate_thresholds accepts unique thresholds.
    #[test]
    fn test_validate_thresholds_accepts_unique() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "vault-1".to_string(),
                threshold: 1,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 1,
                label: "vault-2".to_string(),
                threshold: 2,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 2,
                label: "vault-3".to_string(),
                threshold: 3,
                passphrase_mode: None,
            },
        ];

        let result = validate_thresholds(&metas);

        assert!(result.is_ok());
    }

    /// Test next_compartment_id with empty metas returns 0.
    #[test]
    fn test_next_compartment_id_empty() {
        let metas = vec![];

        let next_id = next_compartment_id(&metas);

        assert_eq!(next_id, 0);
    }

    /// Test next_compartment_id returns max_id + 1.
    #[test]
    fn test_next_compartment_id_with_existing() {
        let metas = vec![
            CompartmentMeta {
                id: 0,
                label: "first".to_string(),
                threshold: 1,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 2,
                label: "third".to_string(),
                threshold: 2,
                passphrase_mode: None,
            },
            CompartmentMeta {
                id: 5,
                label: "sixth".to_string(),
                threshold: 3,
                passphrase_mode: None,
            },
        ];

        let next_id = next_compartment_id(&metas);

        assert_eq!(next_id, 6);
    }

    /// Test CompartmentMeta serialization preserves all fields.
    #[test]
    fn test_compartment_meta_roundtrip() {
        let meta = CompartmentMeta {
            id: 3,
            label: "legacy-vault".to_string(),
            threshold: 2,
            passphrase_mode: Some("wrapped".to_string()),
        };

        let json = serde_json::to_string(&meta).unwrap();
        let decoded: CompartmentMeta = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, meta);
    }

    /// Test generate_dummy_shards produces correct count and length.
    #[test]
    fn test_generate_dummy_shards() {
        let byte_len = 65; // Encrypted shard size
        let count = 50;

        let shards = generate_dummy_shards(count, byte_len);

        assert_eq!(shards.len(), count);
        for shard in &shards {
            // Each byte becomes 2 hex characters
            assert_eq!(shard.len(), byte_len * 2);
        }
    }

    /// Test Fido2Config.is_fido2_enabled returns false when empty.
    #[test]
    fn test_fido2_config_not_enabled_when_empty() {
        let config = Fido2Config::default();

        assert!(!config.is_fido2_enabled());
    }

    /// Test Fido2Config.is_fido2_enabled returns true with keys.
    #[test]
    fn test_fido2_config_enabled_with_keys() {
        let config = Fido2Config {
            total_shares: 1,
            keys: vec![RegisteredKey {
                label: "key1".to_string(),
                credential_id_hex: "aabb".to_string(),
                public_key_der_hex: "ccdd".to_string(),
                public_key_pem: "pem".to_string(),
                shards: vec!["ff".to_string(); SHARD_SLOTS],
                registered_at: "2026-03-10".to_string(),
            }],
        };

        assert!(config.is_fido2_enabled());
    }

    /// Test CompartmentMeta with passphrase_mode serializes correctly.
    #[test]
    fn test_compartment_meta_with_passphrase_mode() {
        let meta = CompartmentMeta {
            id: 1,
            label: "hybrid".to_string(),
            threshold: 1,
            passphrase_mode: Some("fido2_wrapped".to_string()),
        };

        let json = serde_json::to_string(&meta).unwrap();

        assert!(json.contains("fido2_wrapped"));

        let decoded: CompartmentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.passphrase_mode, Some("fido2_wrapped".to_string()));
    }

    /// Test multiple config roundtrips with multiple keys.
    #[test]
    fn test_multiple_keys_config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("multi_key.json");

        let config = Fido2Config {
            total_shares: 3,
            keys: vec![
                RegisteredKey {
                    label: "yubikey-1".to_string(),
                    credential_id_hex: "aaaa".to_string(),
                    public_key_der_hex: "bbbb".to_string(),
                    public_key_pem: "pem1".to_string(),
                    shards: vec!["s1".to_string(); SHARD_SLOTS],
                    registered_at: "2026-01-01".to_string(),
                },
                RegisteredKey {
                    label: "solokey-2".to_string(),
                    credential_id_hex: "cccc".to_string(),
                    public_key_der_hex: "dddd".to_string(),
                    public_key_pem: "pem2".to_string(),
                    shards: vec!["s2".to_string(); SHARD_SLOTS],
                    registered_at: "2026-02-01".to_string(),
                },
                RegisteredKey {
                    label: "titankey-3".to_string(),
                    credential_id_hex: "eeee".to_string(),
                    public_key_der_hex: "ffff".to_string(),
                    public_key_pem: "pem3".to_string(),
                    shards: vec!["s3".to_string(); SHARD_SLOTS],
                    registered_at: "2026-03-01".to_string(),
                },
            ],
        };

        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.total_shares, 3);
        assert_eq!(loaded.keys.len(), 3);
        assert_eq!(loaded.keys[0].label, "yubikey-1");
        assert_eq!(loaded.keys[1].label, "solokey-2");
        assert_eq!(loaded.keys[2].label, "titankey-3");
    }
}

#[cfg(test)]
mod fido2_crypto_tests {
    use rand::RngCore;
    use rand::rngs::OsRng;
    use sigillum_fido2::crypto;

    /// Test split and reconstruct roundtrip with threshold = total.
    #[test]
    fn test_split_reconstruct_roundtrip() {
        let mut master_key = [0u8; 32];
        OsRng.fill_bytes(&mut master_key);

        let threshold = 3;
        let total = 3;

        let shards =
            crypto::split_master_key(&master_key, threshold, total).expect("split should succeed");

        assert_eq!(shards.len(), total);

        let reconstructed =
            crypto::reconstruct_master_key(&shards).expect("reconstruct should succeed");

        assert_eq!(*reconstructed, master_key);
    }

    /// Test split and reconstruct with threshold < total.
    #[test]
    fn test_split_reconstruct_with_threshold_less_than_total() {
        let mut master_key = [0u8; 32];
        OsRng.fill_bytes(&mut master_key);

        let threshold = 2;
        let total = 5;

        let shards =
            crypto::split_master_key(&master_key, threshold, total).expect("split should succeed");

        assert_eq!(shards.len(), total);

        // Reconstruct with exactly threshold shards (first 2)
        let selected = &shards[0..threshold];
        let reconstructed =
            crypto::reconstruct_master_key(selected).expect("reconstruct should succeed");

        assert_eq!(*reconstructed, master_key);
    }

    /// Test reconstruct fails with insufficient shards.
    #[test]
    fn test_reconstruct_fails_insufficient_shards() {
        let mut master_key = [0u8; 32];
        OsRng.fill_bytes(&mut master_key);

        let shards = crypto::split_master_key(&master_key, 3, 5).expect("split should succeed");

        // Try to reconstruct with only 1 shard (need 3)
        let insufficient = &shards[0..1];
        let result = crypto::reconstruct_master_key(insufficient);

        match result {
            Ok(reconstructed) => assert_ne!(*reconstructed, master_key),
            Err(_) => {}
        }
    }

    /// Test encrypt_shard_tagged and decrypt_shard_tagged roundtrip.
    #[test]
    fn test_shard_encrypt_decrypt_roundtrip() {
        let mut hmac_secret = [0u8; 32];
        OsRng.fill_bytes(&mut hmac_secret);
        let comp_id = 5;
        let mut shard_data = [0u8; 33];
        OsRng.fill_bytes(&mut shard_data);

        let encrypted = crypto::encrypt_shard_tagged(&hmac_secret, comp_id, &shard_data)
            .expect("encrypt should succeed");

        // Encrypted shard should be decryptable
        let (dec_comp_id, dec_shard) =
            crypto::decrypt_shard_tagged(&hmac_secret, &encrypted).expect("decrypt should succeed");

        assert_eq!(dec_comp_id, comp_id);
        assert_eq!(dec_shard, shard_data);
    }

    /// Test decrypt_shard_tagged fails with wrong key.
    #[test]
    fn test_shard_decrypt_wrong_key_fails() {
        let mut hmac_secret1 = [0u8; 32];
        let mut hmac_secret2 = [0u8; 32];
        OsRng.fill_bytes(&mut hmac_secret1);
        OsRng.fill_bytes(&mut hmac_secret2);

        let comp_id = 3;
        let mut shard_data = [0u8; 33];
        OsRng.fill_bytes(&mut shard_data);

        let encrypted = crypto::encrypt_shard_tagged(&hmac_secret1, comp_id, &shard_data)
            .expect("encrypt should succeed");

        // Decrypt with different key should fail
        let result = crypto::decrypt_shard_tagged(&hmac_secret2, &encrypted);

        assert!(result.is_err());
    }

    /// Test encrypt_compartment_meta and decrypt_compartment_meta roundtrip.
    #[test]
    fn test_compartment_meta_encrypt_decrypt_roundtrip() {
        use sigillum_fido2::config::CompartmentMeta;

        let mut master_key = [0u8; 32];
        OsRng.fill_bytes(&mut master_key);

        let meta = CompartmentMeta {
            id: 7,
            label: "test-vault".to_string(),
            threshold: 2,
            passphrase_mode: Some("fido2".to_string()),
        };

        let encrypted =
            crypto::encrypt_compartment_meta(&master_key, &meta).expect("encrypt should succeed");

        let decrypted = crypto::decrypt_compartment_meta(&master_key, &encrypted)
            .expect("decrypt should succeed");

        assert_eq!(decrypted.id, 7);
        assert_eq!(decrypted.label, "test-vault");
        assert_eq!(decrypted.threshold, 2);
        assert_eq!(decrypted.passphrase_mode, Some("fido2".to_string()));
    }

    /// Test decrypt_compartment_meta fails with wrong key.
    #[test]
    fn test_compartment_meta_decrypt_wrong_key_fails() {
        use sigillum_fido2::config::CompartmentMeta;

        let mut master_key1 = [0u8; 32];
        let mut master_key2 = [0u8; 32];
        OsRng.fill_bytes(&mut master_key1);
        OsRng.fill_bytes(&mut master_key2);

        let meta = CompartmentMeta {
            id: 3,
            label: "vault".to_string(),
            threshold: 1,
            passphrase_mode: None,
        };

        let encrypted =
            crypto::encrypt_compartment_meta(&master_key1, &meta).expect("encrypt should succeed");

        // Decrypt with different key should fail
        let result = crypto::decrypt_compartment_meta(&master_key2, &encrypted);

        assert!(result.is_err());
    }
}
