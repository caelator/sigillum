//! Crash-recovery and snapshot restore tests.
//!
//! Verifies snapshot export/restore roundtrip, wrong passphrase rejection,
//! truncated/invalid snapshot handling, and fresh directory startup.

use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

#[derive(Clone, Copy)]
enum StoreKind {
    Profiles,
    Deposits,
    Queue,
}

impl StoreKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Profiles => "profiles.json",
            Self::Deposits => "deposits.json",
            Self::Queue => "queue.json",
        }
    }

    fn schema(self) -> &'static str {
        match self {
            Self::Profiles => "sigillum.profiles",
            Self::Deposits => "sigillum.deposits",
            Self::Queue => "sigillum.queue",
        }
    }

    fn schema_version(self) -> u32 {
        match self {
            Self::Profiles | Self::Deposits => 1,
            Self::Queue => 2,
        }
    }

    fn route(self) -> &'static str {
        match self {
            Self::Profiles => "/api/profiles/evm",
            Self::Deposits => "/api/deposits/eth-stealth",
            Self::Queue => "/api/queue/jobs",
        }
    }

    fn response_items(self, body: &Value) -> &Vec<Value> {
        let key = match self {
            Self::Profiles => "profiles",
            Self::Deposits => "deposits",
            Self::Queue => "jobs",
        };
        body[key].as_array().expect("response items")
    }
}

fn store_path(base_dir: &Path, kind: StoreKind) -> PathBuf {
    base_dir.join(kind.file_name())
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().unwrap().to_string_lossy();
    path.with_file_name(format!("{file_name}.bak"))
}

fn store_envelope(kind: StoreKind, data: Value) -> Value {
    json!({
        "schema": kind.schema(),
        "schema_version": kind.schema_version(),
        "data": data,
    })
}

fn store_data(kind: StoreKind, label: &str) -> Value {
    match kind {
        StoreKind::Profiles => json!({
            "evm_providers": [{
                "name": format!("provider-{label}"),
                "rpc_url": format!("https://{label}.example.invalid"),
                "compartment_id": 0,
                "chain_id": if label == "live" { 8453 } else { 1 },
            }],
            "eth_stealth_wallets": [],
            "eth_xpub_wallets": [],
            "eth_seed_wallets": [],
        }),
        StoreKind::Deposits => json!({
            "eth_stealth": [{
                "id": format!("deposit-{label}"),
                "status": "pending",
                "asset_kind": "native",
                "wallet_profile": "wallet-a",
                "wallet_compartment_id": 0,
                "provider_compartment_id": 0,
                "wallet": "wallet-a",
                "short_name": "eth",
                "stealth_meta_address": "st:eth:example",
                "stealth_address": "0x0000000000000000000000000000000000000001",
                "ephemeral_public_key_hex": "0x02",
                "view_tag_hex": "0xaa",
                "auto_queue_sweep": false,
                "created_at_unix": 1,
                "updated_at_unix": 1,
            }],
        }),
        StoreKind::Queue => json!({
            "jobs": [
                {
                    "id": format!("job-{label}"),
                    "state": "queued",
                    "attempts": 0,
                    "created_at_unix": 1,
                    "updated_at_unix": 1,
                    "kind": "eth_stealth_native_sweep",
                    "wallet_profile": "wallet-a",
                    "stealth_address": "0x0000000000000000000000000000000000000001",
                    "ephemeral_public_key_hex": "0x02",
                    "destination_address": "0x0000000000000000000000000000000000000002"
                },
                {
                    "id": format!("job-operator-{label}"),
                    "state": "operator_action_required",
                    "attempts": 0,
                    "created_at_unix": 1,
                    "updated_at_unix": 1,
                    "kind": "eth_stealth_native_sweep",
                    "wallet_profile": "wallet-a",
                    "stealth_address": "0x0000000000000000000000000000000000000001",
                    "ephemeral_public_key_hex": "0x02",
                    "destination_address": "0x0000000000000000000000000000000000000002",
                    "last_error": "operator review required"
                }
            ],
        }),
    }
}

fn write_store(base_dir: &Path, kind: StoreKind, label: &str) -> Vec<u8> {
    let path = store_path(base_dir, kind);
    let body = serde_json::to_vec_pretty(&store_envelope(kind, store_data(kind, label))).unwrap();
    fs::write(&path, &body).unwrap();
    fs::write(backup_path(&path), &body).unwrap();
    body
}

fn write_live_store(base_dir: &Path, kind: StoreKind, label: &str) -> Vec<u8> {
    let path = store_path(base_dir, kind);
    let body = serde_json::to_vec_pretty(&store_envelope(kind, store_data(kind, label))).unwrap();
    fs::write(&path, &body).unwrap();
    body
}

fn write_backup_store(base_dir: &Path, kind: StoreKind, label: &str) -> Vec<u8> {
    let path = store_path(base_dir, kind);
    let body = serde_json::to_vec_pretty(&store_envelope(kind, store_data(kind, label))).unwrap();
    fs::write(backup_path(&path), &body).unwrap();
    body
}

fn corrupt_files(base_dir: &Path, kind: StoreKind) -> Vec<PathBuf> {
    let marker = format!("{}.corrupt-", kind.file_name());
    let mut files = fs::read_dir(base_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&marker)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

async fn init_compartment(client: &reqwest::Client, addr: SocketAddr) -> String {
    let init = post_json(
        client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "recover-test"
        }),
        None,
    )
    .await;
    assert!(
        init.status().is_success(),
        "compartment init should succeed: {:?}",
        init.text().await.ok()
    );
    init.json::<Value>().await.unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn assert_health_ready(client: &reqwest::Client, addr: SocketAddr) {
    let health = get(client, addr, "/api/health", None).await;
    assert!(
        health.status().is_success(),
        "health endpoint should be reachable"
    );
    let body: Value = health.json().await.unwrap();
    assert_eq!(body["ready"], json!(true));
    assert_eq!(body["startup_error"], Value::Null);
}

async fn read_store_response(
    client: &reqwest::Client,
    addr: SocketAddr,
    kind: StoreKind,
    token: &str,
) -> Value {
    let response = get(client, addr, kind.route(), Some(token)).await;
    assert!(
        response.status().is_success(),
        "{} should load after recovery: {:?}",
        kind.route(),
        response.text().await.ok()
    );
    response.json().await.unwrap()
}

fn assert_response_label(kind: StoreKind, body: &Value, label: &str) {
    let items = kind.response_items(body);
    assert!(!items.is_empty(), "recovered response should include items");
    match kind {
        StoreKind::Profiles => assert_eq!(items[0]["name"], json!(format!("provider-{label}"))),
        StoreKind::Deposits => assert_eq!(items[0]["id"], json!(format!("deposit-{label}"))),
        StoreKind::Queue => {
            assert!(
                items
                    .iter()
                    .any(|item| item["id"] == json!(format!("job-{label}"))),
                "recovered queue should include queued job for {label}"
            );
            assert!(
                items
                    .iter()
                    .any(|item| item["id"] == json!(format!("job-operator-{label}"))),
                "recovered queue should include operator-action job for {label}"
            );
        }
    }
}

async fn assert_temp_not_renamed_window(kind: StoreKind) {
    let tmp = TempDir::new().unwrap();
    let path = store_path(tmp.path(), kind);
    let live_bytes = write_store(tmp.path(), kind, "seed");
    let backup_bytes = fs::read(backup_path(&path)).unwrap();
    let orphaned_tmp = tmp.path().join(".tmp_123456_deadbeef");
    fs::write(&orphaned_tmp, b"orphaned atomic write temp").unwrap();

    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    assert_health_ready(&client, addr).await;
    let token = init_compartment(&client, addr).await;
    let body = read_store_response(&client, addr, kind, &token).await;

    assert_response_label(kind, &body, "seed");
    assert_eq!(fs::read(&path).unwrap(), live_bytes);
    assert_eq!(fs::read(backup_path(&path)).unwrap(), backup_bytes);
    assert!(
        orphaned_tmp.exists(),
        "orphaned temp file should be ignored"
    );
    assert!(corrupt_files(tmp.path(), kind).is_empty());
}

async fn assert_renamed_with_stale_bak_window(kind: StoreKind) {
    let tmp = TempDir::new().unwrap();
    let path = store_path(tmp.path(), kind);
    let live_bytes = write_live_store(tmp.path(), kind, "live");
    let backup_bytes = write_backup_store(tmp.path(), kind, "backup");
    assert_ne!(live_bytes, backup_bytes);

    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    assert_health_ready(&client, addr).await;
    let token = init_compartment(&client, addr).await;
    let body = read_store_response(&client, addr, kind, &token).await;

    assert_response_label(kind, &body, "live");
    assert_eq!(fs::read(&path).unwrap(), live_bytes);
    assert_eq!(fs::read(backup_path(&path)).unwrap(), live_bytes);
    assert!(corrupt_files(tmp.path(), kind).is_empty());
}

async fn assert_truncated_live_window(kind: StoreKind) {
    let tmp = TempDir::new().unwrap();
    let path = store_path(tmp.path(), kind);
    let backup_bytes = write_store(tmp.path(), kind, "backup");
    let truncated = br#"{"schema":"sigillum.truncated","schema_version":"#;
    fs::write(&path, truncated).unwrap();

    let (addr, _handle) = spawn_daemon(tmp.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    assert_health_ready(&client, addr).await;
    let token = init_compartment(&client, addr).await;
    let body = read_store_response(&client, addr, kind, &token).await;

    assert_response_label(kind, &body, "backup");
    assert_eq!(fs::read(&path).unwrap(), backup_bytes);
    assert_eq!(fs::read(backup_path(&path)).unwrap(), backup_bytes);
    let quarantined = corrupt_files(tmp.path(), kind);
    assert_eq!(quarantined.len(), 1);
    assert_eq!(fs::read(&quarantined[0]).unwrap(), truncated);
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
async fn profiles_temp_not_renamed_ignores_orphaned_atomic_temp_file() {
    assert_temp_not_renamed_window(StoreKind::Profiles).await;
}

#[tokio::test]
async fn deposits_temp_not_renamed_ignores_orphaned_atomic_temp_file() {
    assert_temp_not_renamed_window(StoreKind::Deposits).await;
}

#[tokio::test]
async fn queue_temp_not_renamed_ignores_orphaned_atomic_temp_file() {
    assert_temp_not_renamed_window(StoreKind::Queue).await;
}

#[tokio::test]
async fn profiles_renamed_with_stale_bak_refreshes_backup_from_live() {
    assert_renamed_with_stale_bak_window(StoreKind::Profiles).await;
}

#[tokio::test]
async fn deposits_renamed_with_stale_bak_refreshes_backup_from_live() {
    assert_renamed_with_stale_bak_window(StoreKind::Deposits).await;
}

#[tokio::test]
async fn queue_renamed_with_stale_bak_refreshes_backup_from_live() {
    assert_renamed_with_stale_bak_window(StoreKind::Queue).await;
}

#[tokio::test]
async fn profiles_truncated_live_quarantines_and_restores_backup() {
    assert_truncated_live_window(StoreKind::Profiles).await;
}

#[tokio::test]
async fn deposits_truncated_live_quarantines_and_restores_backup() {
    assert_truncated_live_window(StoreKind::Deposits).await;
}

#[tokio::test]
async fn queue_truncated_live_quarantines_and_restores_backup() {
    assert_truncated_live_window(StoreKind::Queue).await;
}

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
