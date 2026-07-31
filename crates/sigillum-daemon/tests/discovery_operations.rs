//! Adversarial coverage for background discovery-scan operations (plan task
//! 1.2): start an async EVM scan over multiple wallet indices, cancel it
//! mid-run, prove the persisted inventory contains exactly the processed
//! indices, resume, and prove the remainder completes with zero duplicate
//! observations. Also covers the operations API surface (list/get/cancel,
//! terminal-conflict error codes) and the sync-path contract.

use std::collections::BTreeMap;
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
use tempfile::TempDir;

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

// ── Daemon + gated provider fixtures ─────────────────────────────

async fn spawn_daemon(
    base_dir: PathBuf,
) -> (
    SocketAddr,
    Arc<sigillum_daemon::AppState>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state, handle)
}

/// Stub EVM provider with deterministic mid-run control: it can park the
/// Nth `eth_getBalance` call until released (so a test can cancel while the
/// scan is definitely in flight), fail calls from the Nth onward (to force
/// a mid-run scan error), and mark one call's balance as funded.
struct GatedRpcState {
    balance_calls: AtomicUsize,
    /// 1-based balance call to park at; 0 disarms the gate.
    gate_at_balance_call: AtomicUsize,
    gate_release: tokio::sync::Notify,
    gate_waiting: AtomicBool,
    /// 1-based balance call from which to return JSON-RPC errors; 0 = never.
    fail_from_balance_call: AtomicUsize,
    /// 1-based balance call that answers a nonzero balance; 0 = none.
    funded_balance_call: AtomicUsize,
}

impl GatedRpcState {
    fn new() -> Self {
        Self {
            balance_calls: AtomicUsize::new(0),
            gate_at_balance_call: AtomicUsize::new(0),
            gate_release: tokio::sync::Notify::new(),
            gate_waiting: AtomicBool::new(false),
            fail_from_balance_call: AtomicUsize::new(0),
            funded_balance_call: AtomicUsize::new(0),
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
                "eth_getBalance" => {
                    let call = state.balance_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    let fail_from = state.fail_from_balance_call.load(Ordering::SeqCst);
                    if fail_from != 0 && call >= fail_from {
                        responses.push(json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": { "code": -32000, "message": "stub boom" }
                        }));
                        continue;
                    }
                    let gate_at = state.gate_at_balance_call.load(Ordering::SeqCst);
                    if gate_at != 0 && call == gate_at {
                        // Register the waiter before announcing the gate so a
                        // release can never be missed.
                        let notified = state.gate_release.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        state.gate_waiting.store(true, Ordering::SeqCst);
                        // A broken test must fail, not hang forever.
                        let _ = tokio::time::timeout(Duration::from_secs(30), notified).await;
                        state.gate_waiting.store(false, Ordering::SeqCst);
                        state.gate_at_balance_call.store(0, Ordering::SeqCst);
                    }
                    if state.funded_balance_call.load(Ordering::SeqCst) == call {
                        json!("0xde0b6b3a7640000")
                    } else {
                        json!("0x0")
                    }
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
    state: Arc<sigillum_daemon::AppState>,
    rpc: Arc<GatedRpcState>,
    handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    _dir: TempDir,
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

    async fn get_with_status(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
    }

    async fn get(&self, path: &str) -> Value {
        let (status, body) = self.get_with_status(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
        body
    }

    async fn get_unauthenticated(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(format!("http://{}{path}", self.addr))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
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

    fn balance_calls(&self) -> usize {
        self.rpc.balance_calls.load(Ordering::SeqCst)
    }

    async fn discovery_job(&self, job_id: &str) -> Value {
        let jobs = self.get("/api/discovery/jobs").await;
        jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == job_id)
            .cloned()
            .unwrap_or_else(|| panic!("job {job_id} missing: {jobs}"))
    }
}

async fn spawn_rig() -> Rig {
    let dir = TempDir::new().unwrap();
    let (addr, state, handle) = spawn_daemon(dir.path().to_path_buf()).await;
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
        state,
        rpc,
        handle,
        rpc_handle,
        _dir: dir,
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
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "provider upsert: {body}");

    let account_xpub =
        sigillum_core::derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
    let (status, body) = rig
        .post(
            "/api/profiles/eth-xpub/upsert",
            json!({
                "name": "account-xpub",
                "project_account": 0,
                "provider_profile": "mainnet",
                "external_account_xpub": account_xpub,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "xpub upsert: {body}");

    rig
}

fn address_indices(inventory: &Value) -> Vec<u64> {
    let mut indices: Vec<u64> = inventory["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|address| address["address_index"].as_u64().unwrap())
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn first_seen_by_address(inventory: &Value) -> BTreeMap<String, u64> {
    inventory["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|address| {
            (
                address["address"].as_str().unwrap().to_ascii_lowercase(),
                address["first_seen_at_unix"].as_u64().unwrap(),
            )
        })
        .collect()
}

async fn wait_for_failed_operation(
    state: &sigillum_daemon::AppState,
    operation_id: &str,
) -> sigillum_api::Operation {
    for _ in 0..100 {
        let operation = state
            .get_operation(operation_id)
            .expect("operation remains registered");
        if operation.state == "failed" {
            return operation;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "operation never failed: {:?}",
        state.get_operation(operation_id)
    );
}

// ── Tests ────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn async_scan_wait_rejects_lock_latch_without_persisting_job() {
    let rig = spawn_rig().await;
    let inventory_path = rig._dir.path().join("wallet_inventory.json");
    let inventory_before = std::fs::read(&inventory_path).ok();

    let held_operation = rig.state.operation_guard().await;
    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 1,
                "gap_limit": 5,
                "run_async": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    let operation_id = scan["operation"]["id"].as_str().unwrap().to_string();
    assert!(rig.state.begin_locking());
    drop(held_operation);

    let operation = wait_for_failed_operation(&rig.state, &operation_id).await;
    assert!(
        operation
            .error
            .as_deref()
            .is_some_and(|error| error.contains("locking")),
        "operation must report the lock latch: {operation:?}"
    );
    assert_eq!(
        std::fs::read(&inventory_path).ok(),
        inventory_before,
        "rejected scan must leave the durable inventory byte-identical"
    );
    assert_eq!(
        rig.balance_calls(),
        0,
        "rejected scan must not reach the provider"
    );

    rig.state.lock_all();
    rig.handle.abort();
    rig.rpc_handle.abort();
}

/// The headline adversarial scenario: cancel an async scan mid-run, verify
/// the persisted partial state, resume, and verify zero duplicate
/// observations.
#[tokio::test]
async fn async_scan_cancel_mid_run_and_resume_completes_without_duplicates() {
    let rig = spawn_rig().await;
    // Index 2 is funded; park the scan inside index 2's balance call.
    rig.rpc.funded_balance_call.store(3, Ordering::SeqCst);
    rig.rpc.gate_at_balance_call.store(3, Ordering::SeqCst);

    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 4,
                "gap_limit": 10,
                "run_async": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    assert_eq!(scan["job"]["status"], "running");
    assert_eq!(scan["addresses"], json!([]));
    assert_eq!(scan["holdings"], json!([]));
    let job_id = scan["job"]["id"].as_str().unwrap().to_string();
    let operation_id = scan["operation"]["id"].as_str().unwrap().to_string();
    assert_eq!(scan["operation"]["kind"], "inventory_scan_evm");
    assert_eq!(scan["operation"]["state"], "running");
    assert_eq!(scan["operation"]["related_ids"], json!([job_id]));

    // Wait until the runner is parked inside the third address index.
    rig.wait_for_gate().await;

    // Mid-run: exactly two indices fully processed and persisted.
    let operation = rig.get(&format!("/api/operations/{operation_id}")).await;
    assert_eq!(operation["operation"]["state"], "running");
    assert_eq!(operation["operation"]["progress"]["processed"], json!(2));
    let inventory = rig.get("/api/inventory/wallets").await;
    assert_eq!(
        address_indices(&inventory),
        vec![0, 1],
        "mid-run inventory: {inventory}"
    );

    // Cancel through the discovery-job verb: it signals the linked
    // operation rather than touching the operation mutex.
    let (status, cancel) = rig
        .post("/api/discovery/jobs/cancel", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel response: {cancel}");
    assert_eq!(cancel["status"], "cancel_requested");
    assert_eq!(cancel["operation"]["state"], "cancel_requested");

    // Release the gate: the provider response returns, then the prompt
    // post-await checkpoint honors the cancel before persisting that
    // in-flight index.
    rig.release_gate();
    let operation = rig.wait_for_operation(&operation_id, "canceled").await;
    assert_eq!(operation["operation"]["progress"]["processed"], json!(2));
    assert!(operation["operation"]["completed_at_unix"].is_number());

    // (a) The job is durably canceled, its checkpoint parked at the next
    // unprocessed index.
    let job = rig.discovery_job(&job_id).await;
    assert_eq!(job["status"], "canceled");
    assert_eq!(job["addresses_scanned"], json!(2));
    assert!(job["completed_at_unix"].is_number());
    let checkpoint = job["checkpoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider_profile"] == "mainnet")
        .expect("mainnet checkpoint present");
    assert_eq!(checkpoint["next_index"], json!(2));
    assert_eq!(checkpoint["completed"], json!(false));

    // (b) Persisted inventory contains exactly the processed indices.
    let inventory = rig.get("/api/inventory/wallets").await;
    assert_eq!(address_indices(&inventory), vec![0, 1]);
    let holdings = inventory["holdings"].as_array().unwrap();
    assert!(
        holdings.is_empty(),
        "canceled in-flight index must not leak a holding: {inventory}"
    );
    let provenance = first_seen_by_address(&inventory);
    assert_eq!(rig.balance_calls(), 3);

    // (d) A second cancel of the finished operation returns the right
    // error code; canceling the canceled job conflicts too.
    let (status, second) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "second cancel: {second}");
    assert_eq!(second["code"], "conflict");
    let (status, recancel) = rig
        .post("/api/discovery/jobs/cancel", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "re-cancel: {recancel}");
    assert_eq!(recancel["code"], "conflict");

    // The mock keys funding to call number rather than address. Move the
    // funded response to the retry call so index 2 remains deterministically
    // funded after its canceled first attempt.
    rig.rpc.funded_balance_call.store(4, Ordering::SeqCst);

    // Resume: a new operation and job continue from the checkpoint.
    let (status, resume) = rig
        .post("/api/discovery/jobs/resume", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "resume response: {resume}");
    assert_eq!(resume["status"], "running");
    let resume_job_id = resume["job"]["id"].as_str().unwrap().to_string();
    assert_ne!(resume_job_id, job_id);
    let resume_operation_id = resume["operation"]["id"].as_str().unwrap().to_string();
    assert_eq!(resume["operation"]["related_ids"], json!([resume_job_id]));

    rig.wait_for_operation(&resume_operation_id, "completed")
        .await;

    // (c) The remainder completed with zero duplicate durable observations.
    // The canceled in-flight index is retried because its provider response
    // was deliberately discarded before the checkpoint.
    let resumed = rig.discovery_job(&resume_job_id).await;
    assert_eq!(resumed["status"], "completed");
    assert_eq!(
        resumed["addresses_scanned"],
        json!(3),
        "resumed job must only scan the missing indices: {resumed}"
    );
    assert_eq!(
        rig.balance_calls(),
        6,
        "only the canceled in-flight index may be retried"
    );

    let inventory = rig.get("/api/inventory/wallets").await;
    let addresses = inventory["addresses"].as_array().unwrap();
    assert_eq!(addresses.len(), 5, "no duplicate address rows: {inventory}");
    assert_eq!(address_indices(&inventory), vec![0, 1, 2, 3, 4]);
    // Per-index provenance: the indices processed before the cancel kept
    // their original first_seen timestamps (never re-observed).
    for address in addresses {
        let key = address["address"].as_str().unwrap().to_ascii_lowercase();
        if address["address_index"].as_u64().unwrap() <= 1 {
            assert_eq!(
                address["first_seen_at_unix"].as_u64().unwrap(),
                provenance[&key],
                "address at index {} was re-observed",
                address["address_index"]
            );
        }
    }
    let holdings = inventory["holdings"].as_array().unwrap();
    assert_eq!(
        holdings.len(),
        1,
        "no duplicate holdings after resume: {inventory}"
    );

    // Resuming a completed job is a conflict.
    let (status, conflict) = rig
        .post("/api/discovery/jobs/resume", json!({ "id": resume_job_id }))
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "resume of completed: {conflict}"
    );
    assert_eq!(conflict["code"], "conflict");

    rig.handle.abort();
    rig.rpc_handle.abort();
}

/// The operations API surface: unknown ids, auth, list ordering, direct
/// cancel, idempotent re-cancel, and terminal conflict.
#[tokio::test]
async fn operation_endpoints_list_get_cancel_and_conflict() {
    let rig = spawn_rig().await;

    let (status, body) = rig.get_with_status("/api/operations/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "get unknown: {body}");
    assert_eq!(body["code"], "not_found");
    let (status, body) = rig
        .post("/api/operations/does-not-exist/cancel", json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "cancel unknown: {body}");
    assert_eq!(body["code"], "not_found");
    let (status, body) = rig.get_unauthenticated("/api/operations").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "unauthenticated: {body}");
    assert_eq!(body["code"], "unauthorized");

    // Park the scan inside index 1's balance call.
    rig.rpc.gate_at_balance_call.store(2, Ordering::SeqCst);
    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 3,
                "gap_limit": 10,
                "run_async": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    let operation_id = scan["operation"]["id"].as_str().unwrap().to_string();
    rig.wait_for_gate().await;

    // The list is most-recent-first and reports live progress.
    let list = rig.get("/api/operations").await;
    let operations = list["operations"].as_array().unwrap();
    assert_eq!(operations[0]["id"], json!(operation_id));
    assert_eq!(operations[0]["state"], "running");
    assert_eq!(operations[0]["progress"]["processed"], json!(1));

    let (status, cancel) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel: {cancel}");
    assert_eq!(cancel["status"], "cancel_requested");
    assert_eq!(cancel["operation"]["state"], "cancel_requested");

    // Re-cancel while cancel_requested is an idempotent success.
    let (status, again) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "re-cancel: {again}");
    assert_eq!(again["status"], "cancel_requested");

    rig.release_gate();
    rig.wait_for_operation(&operation_id, "canceled").await;

    // Cancel of a terminal operation conflicts with the right code.
    let (status, third) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "terminal cancel: {third}");
    assert_eq!(third["code"], "conflict");

    // The linked discovery job was durably marked canceled by the runner.
    let job_id = scan["job"]["id"].as_str().unwrap().to_string();
    let job = rig.discovery_job(&job_id).await;
    assert_eq!(job["status"], "canceled");

    rig.handle.abort();
    rig.rpc_handle.abort();
}

/// The synchronous path is contract-identical for existing clients (no
/// `operation` key in the response) while still registering an operation
/// for observability and mid-run cancel.
#[tokio::test]
async fn sync_scan_response_shape_unchanged_and_registers_operation() {
    let rig = spawn_rig().await;

    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 1,
                "gap_limit": 5,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    assert_eq!(scan["job"]["status"], "completed");
    assert!(
        scan.as_object().unwrap().get("operation").is_none(),
        "sync response must not carry an operation field: {scan}"
    );
    assert_eq!(scan["addresses"].as_array().unwrap().len(), 2);

    let list = rig.get("/api/operations").await;
    let operations = list["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0]["kind"], "inventory_scan_evm");
    assert_eq!(operations[0]["state"], "completed");
    assert_eq!(
        operations[0]["related_ids"],
        json!([scan["job"]["id"].as_str().unwrap()])
    );
    assert_eq!(operations[0]["progress"]["processed"], json!(2));

    rig.handle.abort();
    rig.rpc_handle.abort();
}

/// A mid-run provider failure persists the job as `failed` (resumable)
/// instead of leaking a permanently `running` record.
#[tokio::test]
async fn failed_scan_persists_failed_job_and_resume_completes() {
    let rig = spawn_rig().await;
    // Index 2's balance call fails (third call), interrupting the scan.
    rig.rpc.fail_from_balance_call.store(3, Ordering::SeqCst);

    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 4,
                "gap_limit": 10,
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "scan response: {scan}"
    );

    let jobs = rig.get("/api/discovery/jobs").await;
    let job = &jobs["jobs"][0];
    let job_id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["status"], "failed");
    assert!(
        job["last_error"].as_str().unwrap().contains("stub boom"),
        "job last_error: {job}"
    );
    let checkpoint = job["checkpoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider_profile"] == "mainnet")
        .expect("mainnet checkpoint present");
    assert_eq!(checkpoint["next_index"], json!(2));
    assert_eq!(checkpoint["completed"], json!(false));

    let list = rig.get("/api/operations").await;
    let operation = &list["operations"][0];
    assert_eq!(operation["state"], "failed");
    assert!(
        operation["error"].as_str().unwrap().contains("stub boom"),
        "operation error: {operation}"
    );
    assert_eq!(operation["related_ids"], json!([job_id]));

    let inventory = rig.get("/api/inventory/wallets").await;
    assert_eq!(address_indices(&inventory), vec![0, 1]);

    // The provider "recovers"; resuming the failed job completes the rest.
    rig.rpc.fail_from_balance_call.store(0, Ordering::SeqCst);
    let (status, resume) = rig
        .post("/api/discovery/jobs/resume", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "resume response: {resume}");
    let resume_operation_id = resume["operation"]["id"].as_str().unwrap().to_string();
    rig.wait_for_operation(&resume_operation_id, "completed")
        .await;

    let resume_job_id = resume["job"]["id"].as_str().unwrap().to_string();
    let resumed = rig.discovery_job(&resume_job_id).await;
    assert_eq!(resumed["status"], "completed");
    assert_eq!(resumed["addresses_scanned"], json!(3));
    let inventory = rig.get("/api/inventory/wallets").await;
    assert_eq!(address_indices(&inventory), vec![0, 1, 2, 3, 4]);

    rig.handle.abort();
    rig.rpc_handle.abort();
}
