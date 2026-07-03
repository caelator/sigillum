//! Crash-recovery and snapshot restore tests.
//!
//! Verifies snapshot export/restore roundtrip, wrong passphrase rejection,
//! truncated/invalid snapshot handling, and fresh directory startup.

use std::net::SocketAddr;
use std::path::PathBuf;
use tempfile::TempDir;

async fn spawn_daemon(base_dir: PathBuf) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
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

/// Setup: init compartment, set API keys, return (addr, token, tmp_dir).
async fn setup_with_data() -> (SocketAddr, String, TempDir) {
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
            "passphrase": "recover-test"
        }),
        None,
    )
    .await;
    assert!(init.status().is_success());
    let init_body: serde_json::Value = init.json().await.unwrap();
    let token = init_body["session_token"].as_str().unwrap().to_string();

    // Seed API keys
    for i in 0..3 {
        let resp = post_json(
            &client,
            addr,
            "/api/api-keys/set",
            serde_json::json!({
                "key": format!("recovery-key-{i}"),
                "value": format!("recovery-value-{i}")
            }),
            Some(&token),
        )
        .await;
        assert!(resp.status().is_success());
    }

    (addr, token, tmp)
}

// ── Happy Paths ────────────────────────────────────────────────────

#[tokio::test]
async fn snapshot_export_and_restore_preserves_api_keys() {
    let (addr, token, _tmp) = setup_with_data().await;
    let client = reqwest::Client::new();

    // Export snapshot
    let resp = post_json(
        &client,
        addr,
        "/api/backup/export",
        serde_json::json!({ "passphrase": "snapshot-pass" }),
        Some(&token),
    )
    .await;
    assert!(resp.status().is_success(), "export should succeed");
    let export_body: serde_json::Value = resp.json().await.unwrap();
    let snapshot_hex = export_body["snapshot_hex"]
        .as_str()
        .expect("should have snapshot_hex");
    assert!(!snapshot_hex.is_empty());

    // Start a fresh daemon on a new directory
    let tmp2 = TempDir::new().unwrap();
    let (addr2, _handle2) = spawn_daemon(tmp2.path().to_path_buf()).await;

    // Setup fresh vault
    let init2 = post_json(
        &client,
        addr2,
        "/api/compartment/init",
        serde_json::json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "fresh-pass"
        }),
        None,
    )
    .await;
    assert!(init2.status().is_success());
    let init2_body: serde_json::Value = init2.json().await.unwrap();
    let token2 = init2_body["session_token"].as_str().unwrap().to_string();

    // Restore snapshot
    let restore = post_json(
        &client,
        addr2,
        "/api/backup/restore",
        serde_json::json!({
            "snapshot_hex": snapshot_hex,
            "passphrase": "snapshot-pass"
        }),
        Some(&token2),
    )
    .await;
    assert!(
        restore.status().is_success(),
        "restore should succeed: {:?}",
        restore.text().await.ok()
    );

    // Re-unlock with original passphrase
    let unlock = post_json(
        &client,
        addr2,
        "/api/unlock",
        serde_json::json!({ "passphrase": "recover-test" }),
        None,
    )
    .await;
    assert!(
        unlock.status().is_success(),
        "should unlock with original passphrase"
    );
    let unlock_body: serde_json::Value = unlock.json().await.unwrap();
    let token3 = unlock_body["session_token"].as_str().unwrap().to_string();

    // Verify API keys are restored
    let list = get(&client, addr2, "/api/api-keys", Some(&token3)).await;
    assert!(list.status().is_success());
    let body: serde_json::Value = list.json().await.unwrap();
    let keys = body["keys"].as_array().unwrap();
    for i in 0..3 {
        assert!(
            keys.iter()
                .any(|k| k.as_str() == Some(&format!("recovery-key-{i}"))),
            "recovery-key-{i} should be restored"
        );
    }
}

// ── Adversarial Paths ──────────────────────────────────────────────

#[tokio::test]
async fn restore_with_wrong_passphrase_fails() {
    let (addr, token, _tmp) = setup_with_data().await;
    let client = reqwest::Client::new();

    let resp = post_json(
        &client,
        addr,
        "/api/backup/export",
        serde_json::json!({ "passphrase": "correct-pass" }),
        Some(&token),
    )
    .await;
    let export_body: serde_json::Value = resp.json().await.unwrap();
    let snapshot_hex = export_body["snapshot_hex"].as_str().unwrap();

    let restore = post_json(
        &client,
        addr,
        "/api/backup/restore",
        serde_json::json!({
            "snapshot_hex": snapshot_hex,
            "passphrase": "wrong-passphrase"
        }),
        Some(&token),
    )
    .await;
    assert!(
        !restore.status().is_success(),
        "restore with wrong passphrase should fail"
    );
}

#[tokio::test]
async fn restore_truncated_snapshot_fails_cleanly() {
    let (addr, token, _tmp) = setup_with_data().await;
    let client = reqwest::Client::new();

    let resp = post_json(
        &client,
        addr,
        "/api/backup/export",
        serde_json::json!({ "passphrase": "trunc-test" }),
        Some(&token),
    )
    .await;
    let export_body: serde_json::Value = resp.json().await.unwrap();
    let snapshot_hex = export_body["snapshot_hex"].as_str().unwrap();
    let truncated = &snapshot_hex[..snapshot_hex.len() / 2];

    let restore = post_json(
        &client,
        addr,
        "/api/backup/restore",
        serde_json::json!({
            "snapshot_hex": truncated,
            "passphrase": "trunc-test"
        }),
        Some(&token),
    )
    .await;
    assert!(
        !restore.status().is_success(),
        "truncated snapshot restore should fail"
    );

    // Verify original data still intact
    let list = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert!(
        list.status().is_success(),
        "original data should survive failed restore"
    );
}

#[tokio::test]
async fn restore_with_invalid_hex_fails_cleanly() {
    let (addr, token, _tmp) = setup_with_data().await;
    let client = reqwest::Client::new();

    let restore = post_json(
        &client,
        addr,
        "/api/backup/restore",
        serde_json::json!({
            "snapshot_hex": "not-valid-hex-XYZGGG",
            "passphrase": "any"
        }),
        Some(&token),
    )
    .await;
    assert!(
        !restore.status().is_success(),
        "invalid hex should be rejected"
    );
}

#[tokio::test]
async fn daemon_starts_on_fresh_directory() {
    let tmp = TempDir::new().unwrap();
    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let resp = get(&client, addr, "/api/status", None).await;
    assert!(
        resp.status().is_success(),
        "status should work on fresh dir"
    );
}
