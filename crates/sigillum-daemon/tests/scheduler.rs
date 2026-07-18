//! Background scheduler tests (plan task 1.6).
//!
//! These tests run in their own binary because they override
//! `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS` / `SIGILLUM_SCHEDULER_REFRESH_SECS`
//! / `SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS` process-wide before the daemon's
//! startup config is read (the `events_idle.rs` precedent). The
//! `SIGILLUM_SCHEDULER_DISABLE=1` counterpart lives in
//! `scheduler_disabled.rs` — its env would conflict with the scheduler
//! being enabled here.
//!
//! Covered scenarios:
//! (a) a `retrying` job with elapsed backoff drains to broadcast with NO
//!     client driving `queue/process` or `maintenance/run`;
//! (b) the scheduler skips all work while the vault is locked and skips the
//!     drain stage while `execution_paused` is latched;
//! (d) a client-driven operation holding the operation guard makes the
//!     scheduler cycle skip instead of double-processing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const DEFAULT_DESTINATION: &str = "0x1111111111111111111111111111111111111111";
const RPC_TOKEN: &str = "rpc-test-token";
const RPC_AUTH: &str = "Bearer rpc-test-token";

/// Must land before `build_router` reads the startup config. Same values in
/// every test of this binary, so the parallel test threads never disagree.
fn set_scheduler_env() {
    unsafe {
        std::env::set_var("SIGILLUM_SCHEDULER_QUEUE_TICK_SECS", "1");
        std::env::set_var("SIGILLUM_SCHEDULER_REFRESH_SECS", "1");
        std::env::set_var("SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS", "1");
    }
}

async fn spawn_daemon_with_state(
    base_dir: PathBuf,
) -> (
    SocketAddr,
    Arc<sigillum_daemon::AppState>,
    tokio::task::JoinHandle<()>,
) {
    spawn_daemon_with_scheduler(base_dir, true).await
}

async fn spawn_daemon_without_scheduler(
    base_dir: PathBuf,
) -> (
    SocketAddr,
    Arc<sigillum_daemon::AppState>,
    tokio::task::JoinHandle<()>,
) {
    spawn_daemon_with_scheduler(base_dir, false).await
}

async fn spawn_daemon_with_scheduler(
    base_dir: PathBuf,
    start_scheduler: bool,
) -> (
    SocketAddr,
    Arc<sigillum_daemon::AppState>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    // The scheduler spawns with the server entry point in production
    // (`run_inner`), so tests opt in explicitly — exactly the call the
    // production path makes.
    if start_scheduler {
        sigillum_daemon::spawn_scheduler(state.clone());
    }
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state, handle)
}

#[derive(Clone, Default)]
struct RpcControl {
    /// When set, EVERY request fails with HTTP 500 — a full provider outage
    /// that makes queue-job preparation fail retryably.
    fail_all: Arc<AtomicBool>,
    broadcast_count: Arc<AtomicUsize>,
    /// Raw transactions the provider accepted, in arrival order.
    accepted_raw_transactions: Arc<Mutex<Vec<String>>>,
}

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, RpcControl) {
    let control = RpcControl::default();
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(control.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, control)
}

async fn rpc_handler(
    State(control): State<RpcControl>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if auth != RPC_AUTH {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing provider auth" })),
        );
    }
    if control.fail_all.load(Ordering::SeqCst) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "mock provider outage" })),
        );
    }

    let payload = if let Some(requests) = body.as_array() {
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            responses.push(rpc_response(&control, request));
        }
        Value::Array(responses)
    } else {
        rpc_response(&control, &body)
    };
    (StatusCode::OK, Json(payload))
}

fn rpc_response(control: &RpcControl, request: &Value) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!("0x20"),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_getBalance" => json!("0xde0b6b3a7640000"),
        "eth_feeHistory" => json!({
            "oldestBlock": "0x1",
            "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
            "gasUsedRatio": [0.5]
        }),
        "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
        "eth_call" => json!("0x0f4240"),
        "eth_getLogs" => json!([]),
        "eth_sendRawTransaction" => {
            let raw = request["params"][0]
                .as_str()
                .expect("eth_sendRawTransaction carries raw transaction hex");
            control.broadcast_count.fetch_add(1, Ordering::SeqCst);
            control
                .accepted_raw_transactions
                .lock()
                .unwrap()
                .push(raw.to_string());
            let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
                .expect("submitted raw transaction is valid hex");
            json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
        }
        other => json!({ "unsupported": other }),
    };
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(json!(1)),
        "result": result,
    })
}

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

async fn get_json(client: &reqwest::Client, addr: SocketAddr, path: &str, token: &str) -> Value {
    let response = client
        .get(format!("http://{addr}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {path}");
    response.json().await.unwrap()
}

/// Poll `f` until it returns true or the deadline elapses (test-failing).
async fn wait_for<F, Fut>(what: &str, timeout: Duration, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if f().await {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct StealthSetup {
    token: String,
    stealth_address: Value,
    ephemeral_public_key_hex: Value,
    view_tag_hex: Value,
}

async fn setup_stealth_queue(
    client: &reqwest::Client,
    addr: SocketAddr,
    rpc_addr: SocketAddr,
) -> StealthSetup {
    let init = post_json(
        client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    assert_eq!(init.status(), StatusCode::OK);
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let key = post_json(
        client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": RPC_TOKEN }),
        Some(&token),
    )
    .await;
    assert_eq!(key.status(), StatusCode::OK);

    // Plan task 2.5: stealth transfers/sweeps gate under the Sweep execution
    // family — every enqueue/drain in this suite needs an enabled policy with
    // the master + sweep gates open and the default destination allow-listed.
    let policy = post_json(
        client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "allowed_destinations": [{ "address": DEFAULT_DESTINATION }],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    let provider_profile = post_json(
        client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider_profile.status(), StatusCode::OK);

    let wallet_profile = post_json(
        client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": DEFAULT_DESTINATION,
            "execution_enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(wallet_profile.status(), StatusCode::OK);

    let export = post_json(
        client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: Value = export.json().await.unwrap();

    let generate = post_json(
        client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": export_json["stealth_meta_address"],
            "ephemeral_private_key_hex": hex::encode([7u8; 32]),
        }),
        None,
    )
    .await;
    assert_eq!(generate.status(), StatusCode::OK);
    let generate_json: Value = generate.json().await.unwrap();

    StealthSetup {
        token,
        stealth_address: generate_json["stealth_address"].clone(),
        ephemeral_public_key_hex: generate_json["ephemeral_public_key_hex"].clone(),
        view_tag_hex: generate_json["view_tag_hex"].clone(),
    }
}

async fn enqueue_stealth_transfer(
    client: &reqwest::Client,
    addr: SocketAddr,
    setup: &StealthSetup,
) -> Value {
    let enqueue = post_json(
        client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": setup.stealth_address,
            "ephemeral_public_key_hex": setup.ephemeral_public_key_hex,
            "view_tag_hex": setup.view_tag_hex,
            "value_wei_hex": "0xde0b6b3a7640000"
        }),
        Some(&setup.token),
    )
    .await;
    assert_eq!(enqueue.status(), StatusCode::OK);
    enqueue.json().await.unwrap()
}

async fn first_queue_job(client: &reqwest::Client, addr: SocketAddr, token: &str) -> Value {
    let list = get_json(client, addr, "/api/queue/jobs", token).await;
    let jobs = list["jobs"].as_array().unwrap().clone();
    assert_eq!(jobs.len(), 1, "expected exactly one queue job: {list}");
    jobs.into_iter().next().unwrap()
}

// ── (a) plan acceptance: retrying job drains with no client connected ─────

#[tokio::test]
async fn retrying_job_drains_to_broadcast_without_a_client() {
    set_scheduler_env();
    let dir = TempDir::new().unwrap();
    let (addr, _state, handle) = spawn_daemon_with_state(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, control) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();

    // Provider down: the scheduler (no client process call at all) drives
    // the job into `retrying` with a 1s backoff.
    control.fail_all.store(true, Ordering::SeqCst);
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;
    enqueue_stealth_transfer(&client, addr, &setup).await;

    wait_for(
        "the job to reach retrying",
        Duration::from_secs(20),
        || async {
            first_queue_job(&client, addr, &setup.token).await["state"] == json!("retrying")
        },
    )
    .await;
    let retrying = first_queue_job(&client, addr, &setup.token).await;
    assert!(retrying["next_attempt_after_unix"].is_u64(), "{retrying}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 0);

    // Provider back. From here NO client request mutates anything: only the
    // scheduler can advance the job.
    control.fail_all.store(false, Ordering::SeqCst);
    wait_for(
        "the scheduler to drain the job to sent",
        Duration::from_secs(20),
        || async { first_queue_job(&client, addr, &setup.token).await["state"] == json!("sent") },
    )
    .await;

    let sent = first_queue_job(&client, addr, &setup.token).await;
    assert_eq!(
        sent["broadcast_transaction_hash_hex"], sent["transaction_hash_hex"],
        "{sent}"
    );
    // Exactly one broadcast — the scheduler must not double-process.
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 1);
    assert_eq!(control.accepted_raw_transactions.lock().unwrap().len(), 1);

    // Diagnostics expose the scheduler block: it ticked, it advanced work,
    // and nothing is due any more.
    let diagnostics = get_json(&client, addr, "/api/diagnostics", &setup.token).await;
    let scheduler = &diagnostics["scheduler"];
    assert_eq!(scheduler["enabled"], json!(true), "{scheduler}");
    assert_eq!(scheduler["queue_tick_secs"], json!(1), "{scheduler}");
    assert!(scheduler["last_tick_at_unix"].is_u64(), "{scheduler}");
    assert_eq!(scheduler["consecutive_failures"], json!(0), "{scheduler}");
    assert_eq!(scheduler["due_queue_job_count"], json!(0), "{scheduler}");
    assert_eq!(
        scheduler["last_cycle_outcome"],
        json!("advanced"),
        "{scheduler}"
    );

    // The advancing cycle is registered as a completed summary operation...
    let operations = get_json(&client, addr, "/api/operations", &setup.token).await;
    let scheduler_ops: Vec<&Value> = operations["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["kind"] == json!("scheduler_cycle"))
        .collect();
    assert!(
        !scheduler_ops.is_empty(),
        "advancing cycles register operations: {operations}"
    );
    assert!(
        scheduler_ops
            .iter()
            .all(|op| op["state"] == json!("completed")),
        "{scheduler_ops:?}"
    );
    assert!(
        scheduler_ops.iter().any(|op| op["related_ids"]
            .as_array()
            .unwrap()
            .contains(&json!("stage:queue_drain"))),
        "{scheduler_ops:?}"
    );

    // ...and audited in the maintenance-run shape for accountability.
    let audit = get_json(
        &client,
        addr,
        "/api/audit?limit=100&kind=maintenance.run",
        &setup.token,
    )
    .await;
    let maintenance_events = audit["events"].as_array().unwrap();
    assert!(
        maintenance_events
            .iter()
            .any(|event| event["details"]["succeeded"] == json!(1)),
        "a scheduler cycle must be audited: {audit}"
    );

    handle.abort();
    rpc_handle.abort();
}

// ── (b) lock state and the kill switch gate every cycle ───────────────────

#[tokio::test]
async fn scheduler_skips_everything_when_locked_and_when_execution_paused() {
    set_scheduler_env();
    let dir = TempDir::new().unwrap();
    // Defer scheduler startup until both safety gates are established. Starting
    // it during setup would leave an unlock -> pause-request window in which a
    // legitimate tick could cross its final pause check before the handler set
    // the in-memory latch.
    let (addr, state, handle) = spawn_daemon_without_scheduler(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, control) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;
    enqueue_stealth_transfer(&client, addr, &setup).await;

    // Persist and latch pause through the real HTTP path before the scheduler
    // exists. The dedicated mid-drain pause test covers preemption after a
    // drain has started; this test covers the scheduler respecting an already
    // authoritative pause across ticks.
    let pause = post_json(
        &client,
        addr,
        "/api/queue/pause",
        json!({}),
        Some(&setup.token),
    )
    .await;
    assert_eq!(pause.status(), StatusCode::OK);
    let pause_json: Value = pause.json().await.unwrap();
    assert_eq!(pause_json["status"], json!("paused"));
    assert_eq!(pause_json["execution_paused"], json!(true));

    // Locked: several ticks pass; nothing may run (no vault access without
    // unlock), and the status block says why it skipped.
    let lock = post_json(&client, addr, "/api/lock", json!({}), Some(&setup.token)).await;
    assert_eq!(lock.status(), StatusCode::OK);
    sigillum_daemon::spawn_scheduler(state.clone());
    wait_for(
        "a locked scheduler tick",
        Duration::from_secs(10),
        || async {
            state.scheduler_status().last_cycle_outcome.as_deref() == Some("skipped_locked")
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 0);
    let locked_tick_at_unix = state
        .scheduler_status()
        .last_tick_at_unix
        .expect("the locked scheduler cycle recorded a tick");

    let unlock = post_json(
        &client,
        addr,
        "/api/unlock",
        json!({ "passphrase": "correct horse battery staple" }),
        None,
    )
    .await;
    assert_eq!(unlock.status(), StatusCode::OK);
    let unlock_json: Value = unlock.json().await.unwrap();
    let token = unlock_json["session_token"].as_str().unwrap().to_string();

    // Paused: unlocked scheduler ticks continue, but the drain stage must not
    // start the due job. Confirm the loop actually advanced after unlock so a
    // stalled scheduler cannot make the assertion pass vacuously. Poll instead
    // of assuming a fixed sleep is enough on a loaded test host.
    wait_for(
        "an unlocked paused scheduler tick",
        Duration::from_secs(10),
        || async {
            let status = state.scheduler_status();
            status
                .last_tick_at_unix
                .is_some_and(|tick| tick > locked_tick_at_unix)
                && status.last_cycle_outcome.as_deref() == Some("idle")
        },
    )
    .await;
    let paused_status = state.scheduler_status();
    assert!(
        paused_status
            .last_tick_at_unix
            .is_some_and(|tick| tick > locked_tick_at_unix),
        "an unlocked paused scheduler cycle must run: {paused_status:?}"
    );
    assert_eq!(paused_status.last_cycle_outcome.as_deref(), Some("idle"));
    let job = first_queue_job(&client, addr, &token).await;
    assert_eq!(job["state"], json!("queued"), "{job}");
    assert_eq!(job["attempts"], json!(0), "{job}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 0);

    // Resume: the scheduler picks the job up on its own.
    let resume = post_json(&client, addr, "/api/queue/resume", json!({}), Some(&token)).await;
    assert_eq!(resume.status(), StatusCode::OK);
    wait_for(
        "the scheduler to drain after resume",
        Duration::from_secs(20),
        || async { first_queue_job(&client, addr, &token).await["state"] == json!("sent") },
    )
    .await;
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 1);

    handle.abort();
    rpc_handle.abort();
}

// ── (d) guard contention skips the cycle instead of double-processing ─────

#[tokio::test]
async fn client_operation_holding_the_guard_skips_the_scheduler_cycle() {
    set_scheduler_env();
    let dir = TempDir::new().unwrap();
    let (addr, state, handle) = spawn_daemon_with_state(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, control) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;
    enqueue_stealth_transfer(&client, addr, &setup).await;

    // Hold the operation guard exactly like a client-driven drain does.
    let guard = state.operation_guard().await;
    wait_for(
        "the scheduler to skip a cycle behind the held guard",
        Duration::from_secs(10),
        || async {
            state.scheduler_status().last_cycle_outcome.as_deref() == Some("skipped_guard_busy")
        },
    )
    .await;
    // The skipped cycle processed nothing.
    let job = first_queue_job(&client, addr, &setup.token).await;
    assert_eq!(job["state"], json!("queued"), "{job}");
    assert_eq!(job["attempts"], json!(0), "{job}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 0);

    // Releasing the guard lets the next cycle drain the job exactly once —
    // per-(source, chain) serialization was never violated.
    drop(guard);
    wait_for(
        "the scheduler to drain once the guard is free",
        Duration::from_secs(20),
        || async { first_queue_job(&client, addr, &setup.token).await["state"] == json!("sent") },
    )
    .await;
    let job = first_queue_job(&client, addr, &setup.token).await;
    assert_eq!(job["attempts"], json!(1), "{job}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 1);

    handle.abort();
    rpc_handle.abort();
}
