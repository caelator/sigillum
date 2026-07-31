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
    let (app, _state) = sigillum_daemon::build_router(dir.path().to_path_buf(), 0)
        .expect("router should initialize");
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

fn strip_generated_at(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(generated_at) = object.get_mut("generated_at_unix") {
                *generated_at = Value::Null;
            }
            for value in object.values_mut() {
                strip_generated_at(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                strip_generated_at(value);
            }
        }
        _ => {}
    }
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

async fn get_json(app: &axum::Router, path: &str, token: &str) -> Value {
    let (status, body) =
        raw_request(app, Method::GET, path, Body::empty(), None, Some(token)).await;
    assert_eq!(status, StatusCode::OK, "{path} should succeed: {body:?}");
    body
}

fn assert_client_error(path: &str, status: StatusCode, body: &Value) {
    assert!(
        status.is_client_error(),
        "{path} should reject with a client error, got {status}: {body:?}"
    );
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{path} must not reject adversarial input as 500"
    );
}

fn minimal_chain_profile(name: &str) -> Value {
    json!({
        "name": name,
        "chain_family": "evm",
        "chain_id": 184467,
        "native_symbol": "ETH",
        "native_decimals": 18,
        "dormancy_block_window": 100
    })
}

fn minimal_policy_update() -> Value {
    json!({
        "enabled": true,
        "allowed_destinations": [],
        "max_step_native_wei_hex": "0x1",
        "max_plan_native_wei_hex": "0x2",
        "simulation_freshness_secs": 900,
        "hot_floor_wei_hex": "0x1",
        "hot_target_wei_hex": "0x2"
    })
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

// ── Chains API ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn chains_routes_reject_adversarial_inputs_without_state_changes() {
    let (app, _dir) = test_app();
    let token = init_session(&app).await;

    for (method, path, body) in [
        (Method::GET, "/api/chains", Value::Null),
        (
            Method::POST,
            "/api/chains/upsert",
            minimal_chain_profile("auth-probe"),
        ),
    ] {
        let raw_body = if body == Value::Null {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        };
        for token in [None, Some("%%%")] {
            let (status, response_body) = raw_request(
                &app,
                method.clone(),
                path,
                raw_body.clone(),
                if raw_body.is_empty() {
                    None
                } else {
                    Some("application/json")
                },
                token,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} should fail closed for token {token:?}: {response_body:?}"
            );
            assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    let before = get_json(&app, "/api/chains", &token).await;

    for (label, body, content_type) in [
        (
            "malformed JSON",
            r#"{"unterminated": "#.as_bytes().to_vec(),
            "application/json",
        ),
        (
            "wrong content type",
            serde_json::to_vec(&minimal_chain_profile("wrong-content-type")).unwrap(),
            "text/plain",
        ),
    ] {
        let (status, body) = raw_request(
            &app,
            Method::POST,
            "/api/chains/upsert",
            body,
            Some(content_type),
            Some(&token),
        )
        .await;
        assert_client_error(&format!("/api/chains/upsert {label}"), status, &body);
    }

    for (label, body, expected) in [
        (
            "chain_id zero",
            json!({
                "name": "zero-chain-id",
                "chain_family": "evm",
                "chain_id": 0
            }),
            Some(StatusCode::BAD_REQUEST),
        ),
        (
            "native_decimals overflow",
            json!({
                "name": "native-decimals-overflow",
                "chain_family": "evm",
                "chain_id": 184468,
                "native_decimals": 300
            }),
            None,
        ),
        (
            "dormancy_block_window zero",
            json!({
                "name": "zero-dormancy-window",
                "chain_family": "evm",
                "chain_id": 184469,
                "dormancy_block_window": 0
            }),
            Some(StatusCode::BAD_REQUEST),
        ),
        (
            "builtin refused",
            json!({
                "name": "builtin-refused",
                "chain_family": "evm",
                "chain_id": 184470,
                "builtin": true
            }),
            Some(StatusCode::BAD_REQUEST),
        ),
        (
            "invalid permit2_address",
            json!({
                "name": "bad-permit2",
                "chain_family": "evm",
                "chain_id": 184471,
                "permit2_address": "0x123"
            }),
            Some(StatusCode::BAD_REQUEST),
        ),
    ] {
        let (status, body) = post_json(&app, "/api/chains/upsert", body, Some(&token)).await;
        if let Some(expected) = expected {
            assert_eq!(
                status, expected,
                "/api/chains/upsert {label} should return {expected}: {body:?}"
            );
        }
        assert_client_error(&format!("/api/chains/upsert {label}"), status, &body);
    }

    for (label, raw) in [
        (
            "negative chain_id",
            r#"{"name":"negative-chain-id","chain_family":"evm","chain_id":-1}"#.to_string(),
        ),
        (
            "chain_id overflow",
            r#"{"name":"overflow-chain-id","chain_family":"evm","chain_id":18446744073709551616}"#
                .to_string(),
        ),
    ] {
        let (status, body) = raw_request(
            &app,
            Method::POST,
            "/api/chains/upsert",
            raw,
            Some("application/json"),
            Some(&token),
        )
        .await;
        assert_client_error(&format!("/api/chains/upsert {label}"), status, &body);
    }

    let (unknown_delete_status, unknown_delete_body) = post_json(
        &app,
        "/api/chains/delete",
        json!({"name": "chain-profile-does-not-exist"}),
        Some(&token),
    )
    .await;
    assert_eq!(
        unknown_delete_status,
        StatusCode::NOT_FOUND,
        "unknown chain profile delete should be 404: {unknown_delete_body:?}"
    );
    assert_client_error(
        "/api/chains/delete unknown",
        unknown_delete_status,
        &unknown_delete_body,
    );

    let builtin_name = before["profiles"]
        .as_array()
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| profile["builtin"].as_bool() == Some(true))
        })
        .and_then(|profile| profile["name"].as_str())
        .expect("builtin chain profile should be seeded")
        .to_string();
    let (builtin_delete_status, builtin_delete_body) = post_json(
        &app,
        "/api/chains/delete",
        json!({"name": builtin_name}),
        Some(&token),
    )
    .await;
    assert_eq!(
        builtin_delete_status,
        StatusCode::BAD_REQUEST,
        "builtin chain profile delete should be 400: {builtin_delete_body:?}"
    );
    assert_client_error(
        "/api/chains/delete builtin",
        builtin_delete_status,
        &builtin_delete_body,
    );

    let oversized_name = "x".repeat((2 * 1024 * 1024) + 1);
    let mut oversized_body = minimal_chain_profile(&oversized_name);
    oversized_body["chain_id"] = json!(184472);
    let (oversized_status, oversized_response) =
        post_json(&app, "/api/chains/upsert", oversized_body, Some(&token)).await;
    assert_eq!(
        oversized_status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized chain profile should be rejected by body limit: {oversized_response:?}"
    );
    assert_client_error(
        "/api/chains/upsert oversized name",
        oversized_status,
        &oversized_response,
    );

    let after = get_json(&app, "/api/chains", &token).await;
    assert_eq!(
        after, before,
        "rejected chain mutations must not change state"
    );
}

// ── Treasury API ────────────────────────────────────────────────────────────

#[tokio::test]
async fn treasury_routes_reject_adversarial_inputs_without_state_changes() {
    let (app, _dir) = test_app();
    let token = init_session(&app).await;

    for (method, path, body) in [
        (Method::GET, "/api/treasury/policy", Value::Null),
        (
            Method::POST,
            "/api/treasury/policy/update",
            minimal_policy_update(),
        ),
        (
            Method::POST,
            "/api/treasury/parties",
            json!({"name": "auth-probe"}),
        ),
    ] {
        let raw_body = if body == Value::Null {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        };
        let (status, response_body) = raw_request(
            &app,
            method,
            path,
            raw_body.clone(),
            if raw_body.is_empty() {
                None
            } else {
                Some("application/json")
            },
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{path} should fail closed without a token: {response_body:?}"
        );
        assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    let policy_before = get_json(&app, "/api/treasury/policy", &token).await;
    let parties_before = get_json(&app, "/api/treasury/parties", &token).await;

    for address in ["0x123", "not-an-address"] {
        let mut body = minimal_policy_update();
        body["allowed_destinations"] = json!([{"address": address}]);
        let (status, response_body) =
            post_json(&app, "/api/treasury/policy/update", body, Some(&token)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "invalid allowlist address {address} should be 400: {response_body:?}"
        );
        assert_client_error(
            &format!("/api/treasury/policy/update invalid allowlist {address}"),
            status,
            &response_body,
        );
    }

    let over_uint256 = format!("0x{}", "f".repeat(65));
    for (label, patch) in [
        (
            "negative max step",
            json!({"max_step_native_wei_hex": "-0x1"}),
        ),
        (
            "over uint256 max step",
            json!({"max_step_native_wei_hex": over_uint256}),
        ),
        (
            "hot floor greater than target",
            json!({"hot_floor_wei_hex": "0x2", "hot_target_wei_hex": "0x1"}),
        ),
        (
            "zero simulation freshness",
            json!({"simulation_freshness_secs": 0}),
        ),
        (
            "negative simulation freshness",
            json!({"simulation_freshness_secs": -5}),
        ),
    ] {
        let mut body = minimal_policy_update();
        let patch = patch.as_object().unwrap();
        for (key, value) in patch {
            body[key] = value.clone();
        }
        let (status, response_body) =
            post_json(&app, "/api/treasury/policy/update", body, Some(&token)).await;
        assert_client_error(
            &format!("/api/treasury/policy/update {label}"),
            status,
            &response_body,
        );
    }

    for (path, body, expected) in [
        (
            "/api/treasury/receive-addresses/allocate",
            json!({"wallet_profile": "wallet-profile-does-not-exist", "purpose": "adversarial"}),
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/treasury/receive-addresses/rotate",
            json!({"allocation_id": "alloc-does-not-exist"}),
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/treasury/parties/update",
            json!({"id": "party-does-not-exist", "name": "Nobody"}),
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/treasury/parties/delete",
            json!({"id": "party-does-not-exist"}),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (status, response_body) = post_json(&app, path, body, Some(&token)).await;
        assert_eq!(
            status, expected,
            "{path} should return {expected}: {response_body:?}"
        );
        assert_client_error(path, status, &response_body);
    }

    let oversized_name = "x".repeat((2 * 1024 * 1024) + 1);
    let (oversized_status, oversized_body) = post_json(
        &app,
        "/api/treasury/parties",
        json!({"name": oversized_name}),
        Some(&token),
    )
    .await;
    assert_eq!(
        oversized_status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized party name should be rejected by body limit: {oversized_body:?}"
    );
    assert_client_error(
        "/api/treasury/parties oversized name",
        oversized_status,
        &oversized_body,
    );

    for (label, body, content_type) in [
        (
            "malformed JSON",
            r#"{"unterminated": "#.as_bytes().to_vec(),
            "application/json",
        ),
        (
            "wrong content type",
            serde_json::to_vec(&minimal_policy_update()).unwrap(),
            "text/plain",
        ),
    ] {
        let (status, response_body) = raw_request(
            &app,
            Method::POST,
            "/api/treasury/policy/update",
            body,
            Some(content_type),
            Some(&token),
        )
        .await;
        assert_client_error(
            &format!("/api/treasury/policy/update {label}"),
            status,
            &response_body,
        );
    }

    let policy_after = get_json(&app, "/api/treasury/policy", &token).await;
    let parties_after = get_json(&app, "/api/treasury/parties", &token).await;
    assert_eq!(
        policy_after, policy_before,
        "rejected treasury policy mutations must not change state"
    );
    assert_eq!(
        parties_after, parties_before,
        "rejected treasury party mutations must not change state"
    );
}

#[tokio::test]
async fn counterparty_update_omission_retains_and_blank_clears_sweep_destination() {
    const DESTINATION: &str = "0x1111111111111111111111111111111111111111";

    let (app, _dir) = test_app();
    let token = init_session(&app).await;
    let (create_status, created) = post_json(
        &app,
        "/api/treasury/parties",
        json!({
            "name": "Clearable destination",
            "sweep_destination_address": DESTINATION
        }),
        Some(&token),
    )
    .await;
    assert_eq!(create_status, StatusCode::OK, "party create: {created:?}");
    let party_id = created["party"]["id"]
        .as_str()
        .expect("created party id")
        .to_string();

    let (omitted_status, omitted) = post_json(
        &app,
        "/api/treasury/parties/update",
        json!({"id": party_id, "name": "Destination retained"}),
        Some(&token),
    )
    .await;
    assert_eq!(
        omitted_status,
        StatusCode::OK,
        "omitted update: {omitted:?}"
    );
    assert_eq!(
        omitted["party"]["sweep_destination_address"],
        json!(DESTINATION),
        "omitting the patch field must retain the stored destination"
    );

    let (malformed_status, malformed) = post_json(
        &app,
        "/api/treasury/parties/update",
        json!({
            "id": party_id,
            "name": "Malformed rejected",
            "sweep_destination_address": "0x123"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(
        malformed_status,
        StatusCode::BAD_REQUEST,
        "malformed nonblank destination must be rejected: {malformed:?}"
    );
    assert_eq!(malformed["code"], json!("validation_failed"));
    assert_eq!(
        malformed["fields"],
        json!([{
            "field": "sweep_destination_address",
            "message": "sweep_destination_address must be a valid ethereum address (optional 0x prefix plus 40 hex characters)"
        }]),
        "the HTTP validation envelope must identify the editable destination field"
    );

    let parties = get_json(&app, "/api/treasury/parties", &token).await;
    let persisted = parties["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|party| party["id"] == json!(party_id))
        .expect("persisted party after malformed update");
    assert_eq!(persisted["name"], json!("Destination retained"));
    assert_eq!(persisted["sweep_destination_address"], json!(DESTINATION));

    let (blank_status, blank) = post_json(
        &app,
        "/api/treasury/parties/update",
        json!({
            "id": party_id,
            "name": "Destination cleared",
            "sweep_destination_address": "   "
        }),
        Some(&token),
    )
    .await;
    assert_eq!(
        blank_status,
        StatusCode::OK,
        "blank clear update: {blank:?}"
    );
    assert_eq!(blank["party"]["sweep_destination_address"], Value::Null);

    let parties = get_json(&app, "/api/treasury/parties", &token).await;
    let persisted = parties["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|party| party["id"] == json!(party_id))
        .expect("persisted party after blank clear");
    assert_eq!(persisted["name"], json!("Destination cleared"));
    assert_eq!(persisted["sweep_destination_address"], Value::Null);
}

// ── Receiving API ───────────────────────────────────────────────────────────

#[tokio::test]
async fn receiving_routes_reject_adversarial_inputs_without_state_changes() {
    let (app, _dir) = test_app();
    let token = init_session(&app).await;

    for (method, path, body) in [
        (Method::GET, "/api/receiving/overview", Value::Null),
        (Method::POST, "/api/receiving/refresh-balances", json!({})),
        (
            Method::POST,
            "/api/receiving/deposits/tag",
            json!({"deposit_id": "auth-probe"}),
        ),
    ] {
        let raw_body = if body == Value::Null {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        };
        for token in [None, Some("%%%")] {
            let (status, response_body) = raw_request(
                &app,
                method.clone(),
                path,
                raw_body.clone(),
                if raw_body.is_empty() {
                    None
                } else {
                    Some("application/json")
                },
                token,
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} should fail closed for token {token:?}: {response_body:?}"
            );
            assert_ne!(status, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    let before = get_json(&app, "/api/receiving/overview", &token).await;

    for (label, body, content_type) in [
        (
            "malformed JSON",
            r#"{"unterminated": "#.as_bytes().to_vec(),
            "application/json",
        ),
        (
            "wrong content type",
            serde_json::to_vec(&json!({"deposit_id": "deposit-does-not-exist"})).unwrap(),
            "text/plain",
        ),
    ] {
        let (status, response_body) = raw_request(
            &app,
            Method::POST,
            "/api/receiving/deposits/tag",
            body,
            Some(content_type),
            Some(&token),
        )
        .await;
        assert_client_error(
            &format!("/api/receiving/deposits/tag {label}"),
            status,
            &response_body,
        );
    }

    let (unknown_status, unknown_body) = post_json(
        &app,
        "/api/receiving/deposits/tag",
        json!({"deposit_id": "deposit-does-not-exist"}),
        Some(&token),
    )
    .await;
    assert_eq!(
        unknown_status,
        StatusCode::NOT_FOUND,
        "unknown deposit tag should be 404: {unknown_body:?}"
    );
    assert_client_error(
        "/api/receiving/deposits/tag unknown",
        unknown_status,
        &unknown_body,
    );

    let (empty_status, empty_body) = post_json(
        &app,
        "/api/receiving/deposits/tag",
        json!({"deposit_id": ""}),
        Some(&token),
    )
    .await;
    assert_client_error(
        "/api/receiving/deposits/tag empty deposit_id",
        empty_status,
        &empty_body,
    );

    let oversized_note = "x".repeat((2 * 1024 * 1024) + 1);
    let (oversized_status, oversized_body) = post_json(
        &app,
        "/api/receiving/deposits/tag",
        json!({
            "deposit_id": "deposit-does-not-exist",
            "counterparty_id": oversized_note
        }),
        Some(&token),
    )
    .await;
    assert_eq!(
        oversized_status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized deposit tag should be rejected by body limit: {oversized_body:?}"
    );
    assert_client_error(
        "/api/receiving/deposits/tag oversized body",
        oversized_status,
        &oversized_body,
    );

    let after = get_json(&app, "/api/receiving/overview", &token).await;
    let mut before_norm = before.clone();
    let mut after_norm = after.clone();
    strip_generated_at(&mut before_norm);
    strip_generated_at(&mut after_norm);
    assert_eq!(
        after_norm, before_norm,
        "rejected tag mutations must not change state"
    );
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
