//! The important half of plan task 1.3: an SSE-connected session must NOT
//! have its idle clock refreshed — otherwise an always-open events tab
//! defeats the vault auto-lock (decision D-D).
//!
//! This lives in its own test binary because it overrides
//! `SIGILLUM_IDLE_LOCK_SECS` process-wide before the daemon's
//! `RuntimePolicy::from_env` runs; keep it the ONLY test in this file.

use std::net::SocketAddr;
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

const IDLE_LOCK_SECS: u64 = 3;

async fn spawn_daemon(base_dir: std::path::PathBuf) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

/// Two sessions: one only ever opens the SSE stream (passive reads), the
/// other keeps making ACTIVE requests. Past the idle timeout the passive
/// session is evicted — the open stream did not extend its life — while the
/// active session survives.
#[tokio::test]
async fn sse_connection_does_not_extend_session_idle_life() {
    // Must land before `AppState::new` reads the policy env.
    unsafe { std::env::set_var("SIGILLUM_IDLE_LOCK_SECS", IDLE_LOCK_SECS.to_string()) };

    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    // Session 1 (will be the passive, SSE-only one).
    let init = client
        .post(format!("http://{addr}/api/compartment/init"))
        .json(&json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }))
        .send()
        .await
        .unwrap();
    let init_json: Value = init.json().await.unwrap();
    let passive_token = init_json["session_token"].as_str().unwrap().to_string();

    // Session 2 (the active control): re-authenticating an unlocked vault
    // mints a fresh session.
    let unlock = client
        .post(format!("http://{addr}/api/unlock"))
        .json(&json!({ "passphrase": "correct horse battery staple" }))
        .send()
        .await
        .unwrap();
    let unlock_json: Value = unlock.json().await.unwrap();
    let active_token = unlock_json["session_token"].as_str().unwrap().to_string();

    // The passive session opens the stream (accepted, 200) and KEEPS IT OPEN.
    let stream = client
        .get(format!("http://{addr}/api/events"))
        .bearer_auth(&passive_token)
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);

    // ~1s in: the active session makes one ACTIVE request (idle clock
    // refreshed for it alone). The stream keeps flowing in the background.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let active_probe = client
        .get(format!("http://{addr}/api/operations"))
        .bearer_auth(&active_token)
        .send()
        .await
        .unwrap();
    assert_eq!(active_probe.status(), StatusCode::OK);

    // At ~3.5s (> idle 3s since session creation): the passive session must
    // be evicted even though its stream is still open...
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let passive_probe = client
        .get(format!("http://{addr}/api/operations"))
        .bearer_auth(&passive_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        passive_probe.status(),
        StatusCode::UNAUTHORIZED,
        "an open SSE stream must not extend session life"
    );

    // ...while the active session (~2.5s since its last ACTIVE request) is
    // still valid.
    let active_probe = client
        .get(format!("http://{addr}/api/operations"))
        .bearer_auth(&active_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        active_probe.status(),
        StatusCode::OK,
        "active requests still defer the idle lock for their own session"
    );

    drop(stream);
    handle.abort();
}
