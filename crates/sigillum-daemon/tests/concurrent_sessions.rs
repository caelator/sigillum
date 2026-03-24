//! Concurrent session tests — verify daemon handles simultaneous operations.
//!
//! Tests multiple concurrent writes, session revocation, lock behavior,
//! and token rejection under concurrent load.

use std::net::SocketAddr;
use std::path::PathBuf;
use tempfile::TempDir;

async fn spawn_daemon(base_dir: PathBuf) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _state) = sigillum_daemon::build_router(base_dir, addr.port());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: serde_json::Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

async fn get(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.get(format!("http://{addr}{path}"));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

/// Setup: initialize a compartment and return (addr, session_token, tmp_dir).
async fn setup_daemon() -> (SocketAddr, String, TempDir) {
    let tmp = TempDir::new().unwrap();
    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        serde_json::json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "test-passphrase"
        }),
        None,
    )
    .await;
    assert!(
        init.status().is_success(),
        "Init should succeed: {}",
        init.status()
    );
    let init_body: serde_json::Value = init.json().await.unwrap();
    let token = init_body["session_token"]
        .as_str()
        .expect("should have session_token")
        .to_string();

    (addr, token, tmp)
}

// ── Happy Paths ────────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_api_key_writes_do_not_corrupt() {
    let (addr, token, _tmp) = setup_daemon().await;
    let client = reqwest::Client::new();

    // Concurrently write 10 different API keys
    let mut handles = tokio::task::JoinSet::new();
    for i in 0..10 {
        let client = client.clone();
        let token = token.clone();
        handles.spawn(async move {
            let resp = post_json(
                &client,
                addr,
                "/api/api-keys/set",
                serde_json::json!({
                    "key": format!("concurrent-key-{i}"),
                    "value": format!("concurrent-value-{i}")
                }),
                Some(&token),
            )
            .await;
            assert!(
                resp.status().is_success(),
                "concurrent write {i} should succeed: {}",
                resp.status()
            );
        });
    }

    while let Some(result) = handles.join_next().await {
        result.unwrap();
    }

    // Verify all 10 keys exist by listing
    let list = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert!(list.status().is_success());
    let body: serde_json::Value = list.json().await.unwrap();
    let keys = body["keys"].as_array().unwrap();
    for i in 0..10 {
        assert!(
            keys.iter()
                .any(|k| k.as_str() == Some(&format!("concurrent-key-{i}"))),
            "concurrent-key-{i} should exist"
        );
    }
}

#[tokio::test]
async fn concurrent_same_key_writes_last_writer_wins() {
    let (addr, token, _tmp) = setup_daemon().await;
    let client = reqwest::Client::new();

    let mut handles = tokio::task::JoinSet::new();
    for i in 0..5 {
        let client = client.clone();
        let token = token.clone();
        handles.spawn(async move {
            let resp = post_json(
                &client,
                addr,
                "/api/api-keys/set",
                serde_json::json!({
                    "key": "shared-key",
                    "value": format!("value-{i}")
                }),
                Some(&token),
            )
            .await;
            assert!(resp.status().is_success());
        });
    }

    while let Some(result) = handles.join_next().await {
        result.unwrap();
    }

    // Key should exist (one of the writes should have won)
    let resp = post_json(
        &client,
        addr,
        "/api/api-keys/get",
        serde_json::json!({ "key": "shared-key" }),
        Some(&token),
    )
    .await;
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["value"].as_str().unwrap().starts_with("value-"));
}

// ── Adversarial Paths ──────────────────────────────────────────────

#[tokio::test]
async fn revoked_session_rejects_subsequent_requests() {
    let (addr, token, _tmp) = setup_daemon().await;
    let client = reqwest::Client::new();

    // Revoke session
    let resp = post_json(
        &client,
        addr,
        "/api/session/revoke",
        serde_json::json!({}),
        Some(&token),
    )
    .await;
    assert!(resp.status().is_success(), "revoke should succeed");

    // Subsequent request with revoked token should fail with 401
    let resp = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert_eq!(resp.status().as_u16(), 401, "revoked token should get 401");
}

#[tokio::test]
async fn expired_token_format_rejected() {
    let (addr, _token, _tmp) = setup_daemon().await;
    let client = reqwest::Client::new();

    let resp = get(&client, addr, "/api/api-keys", Some("totally-fake-token")).await;
    assert_eq!(resp.status().as_u16(), 401, "fake token should get 401");
}

#[tokio::test]
async fn no_token_on_protected_routes_returns_401() {
    let (addr, _token, _tmp) = setup_daemon().await;
    let client = reqwest::Client::new();

    let resp = get(&client, addr, "/api/api-keys", None).await;
    assert_eq!(resp.status().as_u16(), 401, "no token should get 401");
}

#[tokio::test]
async fn wrong_passphrase_rejected() {
    let tmp = TempDir::new().unwrap();
    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    // First init
    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        serde_json::json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct-passphrase"
        }),
        None,
    )
    .await;
    assert!(init.status().is_success());
    let init_body: serde_json::Value = init.json().await.unwrap();
    let token = init_body["session_token"].as_str().unwrap();

    // Lock
    let lock = post_json(
        &client,
        addr,
        "/api/lock",
        serde_json::json!({}),
        Some(token),
    )
    .await;
    assert!(lock.status().is_success());

    // Try to unlock with wrong passphrase
    let resp = post_json(
        &client,
        addr,
        "/api/unlock",
        serde_json::json!({ "passphrase": "wrong-passphrase" }),
        None,
    )
    .await;
    assert!(
        !resp.status().is_success(),
        "wrong passphrase should be rejected"
    );
}
