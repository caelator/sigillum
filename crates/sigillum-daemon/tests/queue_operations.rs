//! Async queue-drain and maintenance-cycle operations (plan task 1.1b):
//! start an async drain over several queued jobs, cancel it mid-run between
//! jobs, and prove the remaining jobs stay untouched; async maintenance
//! cycles report per-stage progress and honor cancel between stages (never
//! mid-stage). Also covers the sync-path contracts (no `operation` key)
//! and the terminal-cancel conflict.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const ONE_ETH_HEX: &str = "0xde0b6b3a7640000";
const DESTINATION: &str = "0x1111111111111111111111111111111111111111";

// ── Daemon + gated provider fixtures ─────────────────────────────

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

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

/// Stub EVM provider with deterministic mid-run control: it can park the Nth
/// `eth_sendRawTransaction` or `eth_getBalance` call until released, so a
/// test can cancel while a drain broadcast / deposit refresh is definitely
/// in flight.
struct GatedRpcState {
    send_raw_calls: AtomicUsize,
    balance_calls: AtomicUsize,
    /// 1-based call ordinals to park at; 0 disarms the gate.
    gate_at_send_raw_call: AtomicUsize,
    gate_at_balance_call: AtomicUsize,
    gate_release: tokio::sync::Notify,
    gate_waiting: AtomicBool,
}

impl GatedRpcState {
    fn new() -> Self {
        Self {
            send_raw_calls: AtomicUsize::new(0),
            balance_calls: AtomicUsize::new(0),
            gate_at_send_raw_call: AtomicUsize::new(0),
            gate_at_balance_call: AtomicUsize::new(0),
            gate_release: tokio::sync::Notify::new(),
            gate_waiting: AtomicBool::new(false),
        }
    }

    /// Park when `call` matches `gate_at`; disarm so only one call waits.
    async fn maybe_gate(&self, call: usize, gate_at: &AtomicUsize) {
        if gate_at.load(Ordering::SeqCst) != 0 && gate_at.load(Ordering::SeqCst) == call {
            // Register the waiter before announcing the gate so a release
            // can never be missed.
            let notified = self.gate_release.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            self.gate_waiting.store(true, Ordering::SeqCst);
            // A broken test must fail, not hang forever.
            let _ = tokio::time::timeout(Duration::from_secs(30), notified).await;
            self.gate_waiting.store(false, Ordering::SeqCst);
            gate_at.store(0, Ordering::SeqCst);
        }
    }
}

async fn spawn_gated_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, Arc<GatedRpcState>)
{
    let state = Arc::new(GatedRpcState::new());

    async fn rpc_handler(
        State(state): State<Arc<GatedRpcState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer rpc-test-token" {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing provider auth" })),
            );
        }
        let requests = body.as_array().cloned().unwrap_or_else(|| vec![body]);
        let mut responses = Vec::new();
        for request in &requests {
            let method = request["method"].as_str().unwrap_or_default();
            let result: Value = match method {
                "eth_chainId" => json!("0x1"),
                "eth_blockNumber" => json!("0x20"),
                "eth_getTransactionCount" => json!("0x7"),
                "eth_getBalance" => {
                    let call = state.balance_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    state.maybe_gate(call, &state.gate_at_balance_call).await;
                    json!(ONE_ETH_HEX)
                }
                "eth_feeHistory" => json!({
                    "oldestBlock": "0x1",
                    "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
                    "gasUsedRatio": [0.5]
                }),
                "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
                "eth_call" => json!("0x"),
                "eth_getLogs" => json!([]),
                "eth_sendRawTransaction" => {
                    let call = state.send_raw_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    state.maybe_gate(call, &state.gate_at_send_raw_call).await;
                    submitted_raw_transaction_hash(request)
                }
                _ => json!("0x0"),
            };
            responses.push(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": result
            }));
        }
        (StatusCode::OK, Json(Value::Array(responses)))
    }

    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, state)
}

// ── Test rig ─────────────────────────────────────────────────────

struct Rig {
    client: reqwest::Client,
    addr: SocketAddr,
    token: String,
    rpc: Arc<GatedRpcState>,
    handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    _dir: TempDir,
    stealth_address: String,
    ephemeral_public_key_hex: String,
    view_tag_hex: String,
}

impl Rig {
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .client
            .post(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
    }

    async fn get(&self, path: &str) -> Value {
        let response = self
            .client
            .get(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "GET {path}");
        response.json().await.unwrap()
    }

    async fn wait_for_operation(&self, operation_id: &str, want: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let body = self.get(&format!("/api/operations/{operation_id}")).await;
            if body["operation"]["state"] == want {
                return body;
            }
            assert!(
                Instant::now() < deadline,
                "operation {operation_id} never reached {want}: {body}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_gate(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.rpc.gate_waiting.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "provider gate was never reached");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn release_gate(&self) {
        self.rpc.gate_release.notify_waiters();
    }

    /// Enqueue one stealth transfer job; returns its job id.
    async fn enqueue_transfer(&self) -> String {
        let (status, body) = self
            .post(
                "/api/queue/enqueue/eth-stealth-transfer",
                json!({
                    "wallet_profile": "payments-mainnet",
                    "stealth_address": self.stealth_address,
                    "ephemeral_public_key_hex": self.ephemeral_public_key_hex,
                    "view_tag_hex": self.view_tag_hex,
                    "value_wei_hex": "0x1"
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "enqueue response: {body}");
        body["job"]["id"].as_str().unwrap().to_string()
    }

    async fn queue_jobs(&self) -> Vec<Value> {
        self.get("/api/queue/jobs").await["jobs"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn shutdown(self) {
        self.handle.abort();
        self.rpc_handle.abort();
    }
}

async fn spawn_rig() -> Rig {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, rpc) = spawn_gated_evm_provider().await;
    let client = reqwest::Client::new();

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
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let rig = Rig {
        client,
        addr,
        token,
        rpc,
        handle,
        rpc_handle,
        _dir: dir,
        stealth_address: String::new(),
        ephemeral_public_key_hex: String::new(),
        view_tag_hex: String::new(),
    };

    let (status, body) = rig
        .post(
            "/api/api-keys/set",
            json!({ "key": "alchemy", "value": "rpc-test-token" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "api key set: {body}");

    let (status, body) = rig
        .post(
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
        )
        .await;
    assert_eq!(status, StatusCode::OK, "provider upsert: {body}");

    let (status, body) = rig
        .post(
            "/api/profiles/eth-stealth/upsert",
            json!({
                "name": "payments-mainnet",
                "wallet": "payments",
                "short_name": "eth",
                "provider_profile": "mainnet",
                "default_destination_address": DESTINATION,
                "execution_enabled": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "wallet upsert: {body}");

    // Plan task 2.5: stealth transfers/sweeps gate under the Sweep execution
    // family — every job in this suite needs the master + sweep gates open
    // and the transfer destination allow-listed.
    let (status, body) = rig
        .post(
            "/api/treasury/policy/update",
            json!({
                "enabled": true,
                "allow_plan_execution": true,
                "allow_sweep_execution": true,
                "allowed_destinations": [{ "address": DESTINATION }],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "policy update: {body}");

    let (status, export) = rig
        .post(
            "/api/wallets/eth-stealth/export",
            json!({ "wallet": "payments", "short_name": "eth" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "wallet export: {export}");

    let (status, generate) = rig
        .post(
            "/api/wallets/eth-stealth/generate",
            json!({
                "stealth_meta_address": export["stealth_meta_address"],
                "ephemeral_private_key_hex": hex::encode([7u8; 32]),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "stealth generate: {generate}");

    let mut rig = rig;
    rig.stealth_address = generate["stealth_address"].as_str().unwrap().to_string();
    rig.ephemeral_public_key_hex = generate["ephemeral_public_key_hex"]
        .as_str()
        .unwrap()
        .to_string();
    rig.view_tag_hex = generate["view_tag_hex"].as_str().unwrap().to_string();
    rig
}

fn job_by_id(jobs: &[Value], id: &str) -> Value {
    jobs.iter()
        .find(|job| job["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("job {id} missing: {jobs:?}"))
}

// ── Tests ────────────────────────────────────────────────────────

/// An async drain processes every selected job to completion and reports
/// exact progress counts (jobs attempted vs jobs selected).
#[tokio::test]
async fn async_queue_drain_completes_with_progress_counts() {
    let rig = spawn_rig().await;
    let first = rig.enqueue_transfer().await;
    let second = rig.enqueue_transfer().await;

    let (status, drain) = rig
        .post("/api/queue/process", json!({ "run_async": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "drain response: {drain}");
    // The accepted response carries zeroed tallies and no jobs — the
    // operation is the live view of the run.
    assert_eq!(drain["processed"], json!(0));
    assert_eq!(drain["succeeded"], json!(0));
    assert_eq!(drain["jobs"], json!([]));
    let operation = &drain["operation"];
    assert_eq!(operation["kind"], "queue_process");
    assert_eq!(operation["state"], "running");
    assert!(
        operation.get("related_ids").is_none(),
        "drain operations carry no related_ids: {operation}"
    );
    let operation_id = operation["id"].as_str().unwrap().to_string();

    let operation = rig.wait_for_operation(&operation_id, "completed").await;
    assert_eq!(operation["operation"]["progress"]["processed"], json!(2));
    assert_eq!(operation["operation"]["progress"]["total"], json!(2));
    assert!(operation["operation"]["completed_at_unix"].is_number());
    assert!(operation["operation"].get("error").is_none());

    let jobs = rig.queue_jobs().await;
    for id in [&first, &second] {
        let job = job_by_id(&jobs, id);
        assert_eq!(job["state"], "sent", "job {id}: {job}");
        assert_eq!(job["attempts"], json!(1));
    }
    assert_eq!(
        rig.rpc.send_raw_calls.load(Ordering::SeqCst),
        2,
        "each job broadcast exactly once"
    );

    rig.shutdown();
}

/// The headline scenario: cancel an async drain while a job's broadcast is
/// in flight. The in-flight job finishes its attempt (never canceled
/// mid-broadcast); the drain then stops before the next job and reports
/// processed vs remaining counts.
#[tokio::test]
async fn async_queue_drain_cancel_between_jobs_leaves_remainder_untouched() {
    let rig = spawn_rig().await;
    let first = rig.enqueue_transfer().await;
    let second = rig.enqueue_transfer().await;
    let third = rig.enqueue_transfer().await;

    // Park the drain inside the SECOND job's broadcast.
    rig.rpc.gate_at_send_raw_call.store(2, Ordering::SeqCst);
    let (status, drain) = rig
        .post("/api/queue/process", json!({ "run_async": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "drain response: {drain}");
    let operation_id = drain["operation"]["id"].as_str().unwrap().to_string();

    rig.wait_for_gate().await;

    // Mid-run: exactly one job attempted, three selected for the run.
    let operation = rig.get(&format!("/api/operations/{operation_id}")).await;
    assert_eq!(operation["operation"]["state"], "running");
    assert_eq!(operation["operation"]["progress"]["processed"], json!(1));
    assert_eq!(operation["operation"]["progress"]["total"], json!(3));

    let (status, cancel) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel response: {cancel}");
    assert_eq!(cancel["status"], "cancel_requested");

    // Release the gate: the in-flight broadcast completes, then the loop
    // honors the cancel before the third job.
    rig.release_gate();
    let operation = rig.wait_for_operation(&operation_id, "canceled").await;
    assert_eq!(operation["operation"]["progress"]["processed"], json!(2));
    assert_eq!(
        operation["operation"]["progress"]["total"],
        json!(3),
        "total stays the full selection so remaining = 3 - 2 = 1"
    );
    assert!(operation["operation"]["completed_at_unix"].is_number());

    // The first two jobs finished their attempts; the third was never
    // touched (state, attempts, and error all intact).
    let jobs = rig.queue_jobs().await;
    for id in [&first, &second] {
        let job = job_by_id(&jobs, id);
        assert_eq!(job["state"], "sent", "job {id}: {job}");
        assert_eq!(job["attempts"], json!(1));
    }
    let remaining = job_by_id(&jobs, &third);
    assert_eq!(remaining["state"], "queued", "remainder: {remaining}");
    assert_eq!(remaining["attempts"], json!(0));
    assert!(remaining["last_error"].is_null());
    assert_eq!(
        rig.rpc.send_raw_calls.load(Ordering::SeqCst),
        2,
        "the third job was never broadcast"
    );

    // A second cancel of the terminal operation conflicts with the right code.
    let (status, second_cancel) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "second cancel: {second_cancel}"
    );
    assert_eq!(second_cancel["code"], "conflict");

    rig.shutdown();
}

/// The synchronous drain is contract-identical for existing clients (no
/// `operation` key) while still registering an operation for observability.
#[tokio::test]
async fn sync_queue_drain_response_shape_unchanged_and_registers_operation() {
    let rig = spawn_rig().await;
    rig.enqueue_transfer().await;

    let (status, drain) = rig.post("/api/queue/process", json!({})).await;
    assert_eq!(status, StatusCode::OK, "drain response: {drain}");
    assert!(
        drain.as_object().unwrap().get("operation").is_none(),
        "sync response must not carry an operation field: {drain}"
    );
    assert_eq!(drain["processed"], json!(1));
    assert_eq!(drain["succeeded"], json!(1));
    assert_eq!(drain["jobs"][0]["state"], "sent");
    assert!(drain.get("paused_reason").is_none());

    let list = rig.get("/api/operations").await;
    let operations = list["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["kind"], "queue_process");
    assert_eq!(operations[0]["state"], "completed");
    assert_eq!(operations[0]["progress"]["processed"], json!(1));
    assert_eq!(operations[0]["progress"]["total"], json!(1));

    rig.shutdown();
}

/// An async maintenance cycle reports per-stage progress through the
/// `stage:<name>` encoding and completes all three stages; the synchronous
/// path keeps its exact response contract.
#[tokio::test]
async fn async_maintenance_run_reports_stage_progress_and_sync_is_unchanged() {
    let rig = spawn_rig().await;
    let job_id = rig.enqueue_transfer().await;

    let (status, run) = rig
        .post("/api/maintenance/run", json!({ "run_async": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "maintenance response: {run}");
    assert_eq!(run["status"], "accepted");
    assert_eq!(run["processed"], json!(0));
    assert_eq!(run["deposits"], json!([]));
    assert_eq!(run["jobs"], json!([]));
    let operation = &run["operation"];
    assert_eq!(operation["kind"], "maintenance_run");
    assert_eq!(operation["state"], "running");
    assert_eq!(
        operation["related_ids"],
        json!([
            "stage:treasury_automation",
            "stage:deposit_refresh",
            "stage:queue_drain"
        ])
    );
    assert_eq!(operation["progress"]["total"], json!(3));
    let operation_id = operation["id"].as_str().unwrap().to_string();

    let operation = rig.wait_for_operation(&operation_id, "completed").await;
    assert_eq!(operation["operation"]["progress"]["processed"], json!(3));

    let job = job_by_id(&rig.queue_jobs().await, &job_id);
    assert_eq!(job["state"], "sent", "maintenance drained the job: {job}");

    // The synchronous cycle is contract-identical (no `operation` key) and
    // still registers a maintenance operation; the drain stage inside a
    // maintenance cycle must NOT leak a nested queue_process operation.
    let (status, sync) = rig.post("/api/maintenance/run", json!({})).await;
    assert_eq!(status, StatusCode::OK, "sync maintenance: {sync}");
    assert_eq!(sync["status"], "ok");
    assert!(
        sync.as_object().unwrap().get("operation").is_none(),
        "sync response must not carry an operation field: {sync}"
    );
    let list = rig.get("/api/operations").await;
    let operations = list["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 2, "operations: {operations:?}");
    assert!(
        operations
            .iter()
            .all(|operation| operation["kind"] == "maintenance_run"
                && operation["state"] == "completed"),
        "operations: {operations:?}"
    );

    rig.shutdown();
}

/// Maintenance cancellation is a between-stages boundary: a cancel landing
/// while a stage is in flight (here the deposit refresh, parked inside its
/// balance call) is honored only after that stage completes — the cycle
/// stops before the queue-drain stage with progress at 2/3.
#[tokio::test]
async fn maintenance_cancel_is_honored_between_stages_not_mid_stage() {
    let rig = spawn_rig().await;

    // A native deposit gives the refresh stage real provider work.
    let (status, deposit) = rig
        .post(
            "/api/deposits/eth-stealth/create-native",
            json!({
                "wallet_profile": "payments-mainnet",
                "expected_value_wei_hex": "0x1",
                "auto_queue_sweep": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "deposit create: {deposit}");

    // Park the cycle inside stage 2's balance call (the first RPC balance
    // call this daemon makes — nothing else runs beforehand).
    rig.rpc.gate_at_balance_call.store(1, Ordering::SeqCst);
    let (status, run) = rig
        .post("/api/maintenance/run", json!({ "run_async": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "maintenance response: {run}");
    let operation_id = run["operation"]["id"].as_str().unwrap().to_string();

    rig.wait_for_gate().await;

    // Mid-stage-2: only the treasury-automation stage has completed.
    let operation = rig.get(&format!("/api/operations/{operation_id}")).await;
    assert_eq!(operation["operation"]["state"], "running");
    assert_eq!(operation["operation"]["progress"]["processed"], json!(1));
    assert_eq!(operation["operation"]["progress"]["total"], json!(3));

    let (status, cancel) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel response: {cancel}");
    assert_eq!(cancel["status"], "cancel_requested");

    // Release the gate: the in-flight refresh stage completes (cancel is
    // NOT honored mid-stage), then the cycle stops before the queue drain.
    rig.release_gate();
    let operation = rig.wait_for_operation(&operation_id, "canceled").await;
    assert_eq!(
        operation["operation"]["progress"]["processed"],
        json!(2),
        "the refresh stage completed before the cancel was honored"
    );
    assert_eq!(operation["operation"]["progress"]["total"], json!(3));
    assert!(operation["operation"]["completed_at_unix"].is_number());

    rig.shutdown();
}
