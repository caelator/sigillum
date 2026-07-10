//! W7.4 integration tests: execution semantics — nonces, receipts, failure
//! classes. Mock-RPC only; every daemon runs in a fresh TempDir on an
//! ephemeral port. Helpers mirror (duplicated, not shared — house style)
//! the patterns in `plan_execution.rs`, extended with a configurable mock
//! RPC provider (broadcast error injection + call counters, receipt-poll
//! response modes) needed to drive the new W7.4 failure paths.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
#[cfg(all(unix, feature = "test-failpoints"))]
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(all(unix, feature = "test-failpoints"))]
use std::os::unix::process::ExitStatusExt;
#[cfg(all(unix, feature = "test-failpoints"))]
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const DESTINATION: &str = "0x9999999999999999999999999999999999999999";
const SEED_ADDRESS: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const SEED_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const ONE_ETH_HEX: &str = "0xde0b6b3a7640000";
const RPC_TOKEN: &str = "rpc-test-token";
const COMPARTMENT_PASSPHRASE: &str = "correct horse battery staple";

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

fn now_unix_test() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

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

// ── Configurable mock EVM provider ──────────────────────────────────────

#[derive(Clone)]
enum ReceiptMode {
    /// The exact shape a provider without receipt support returns for an
    /// unhandled method — matches `plan_execution.rs`'s default mock, so
    /// tests that never touch receipt polling behave exactly as before.
    Unsupported,
    /// JSON-RPC `null`: mined nowhere yet.
    Pending,
    Success {
        block_number_hex: String,
        gas_used_hex: String,
    },
    Reverted {
        block_number_hex: String,
        gas_used_hex: String,
    },
}

#[derive(Clone)]
struct RpcState {
    broadcast_calls: Arc<AtomicUsize>,
    broadcast_raw_hexes: Arc<Mutex<Vec<String>>>,
    broadcast_error: Arc<Mutex<Option<String>>>,
    transaction_count_calls: Arc<AtomicUsize>,
    receipt_mode: Arc<Mutex<ReceiptMode>>,
    block_number_hex: Arc<Mutex<String>>,
}

impl Default for RpcState {
    fn default() -> Self {
        Self {
            broadcast_calls: Arc::new(AtomicUsize::new(0)),
            broadcast_raw_hexes: Arc::new(Mutex::new(Vec::new())),
            broadcast_error: Arc::new(Mutex::new(None)),
            transaction_count_calls: Arc::new(AtomicUsize::new(0)),
            receipt_mode: Arc::new(Mutex::new(ReceiptMode::Unsupported)),
            block_number_hex: Arc::new(Mutex::new("0x20".to_string())),
        }
    }
}

impl RpcState {
    fn set_broadcast_error(&self, message: Option<&str>) {
        *self.broadcast_error.lock().unwrap() = message.map(str::to_string);
    }

    fn set_receipt_mode(&self, mode: ReceiptMode) {
        *self.receipt_mode.lock().unwrap() = mode;
    }

    fn broadcast_call_count(&self) -> usize {
        self.broadcast_calls.load(Ordering::SeqCst)
    }

    fn transaction_count_call_count(&self) -> usize {
        self.transaction_count_calls.load(Ordering::SeqCst)
    }

    fn broadcast_raw_hexes(&self) -> Vec<String> {
        self.broadcast_raw_hexes.lock().unwrap().clone()
    }
}

fn rpc_response(state: &RpcState, request: &Value) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    let id = request.get("id").cloned().unwrap_or(json!(1));
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!(state.block_number_hex.lock().unwrap().clone()),
        "eth_getTransactionCount" => {
            state.transaction_count_calls.fetch_add(1, Ordering::SeqCst);
            json!("0x7")
        }
        "eth_getBalance" => json!(ONE_ETH_HEX),
        "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
        "eth_feeHistory" => json!({
            "oldestBlock": "0x1",
            "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
            "gasUsedRatio": [0.5]
        }),
        "eth_call" => {
            let data = request["params"][0]["data"].as_str().unwrap_or_default();
            if data == "0x" {
                json!("0x")
            } else {
                json!("0x0f4240")
            }
        }
        "eth_getLogs" => json!([]),
        "eth_sendRawTransaction" => {
            state.broadcast_calls.fetch_add(1, Ordering::SeqCst);
            let raw = request["params"][0]
                .as_str()
                .unwrap_or_default()
                .to_string();
            state.broadcast_raw_hexes.lock().unwrap().push(raw);
            let error = state.broadcast_error.lock().unwrap().clone();
            if let Some(message) = error {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": message },
                });
            }
            submitted_raw_transaction_hash(request)
        }
        "eth_getTransactionReceipt" => match state.receipt_mode.lock().unwrap().clone() {
            ReceiptMode::Unsupported => json!({ "unsupported": "eth_getTransactionReceipt" }),
            ReceiptMode::Pending => Value::Null,
            ReceiptMode::Success {
                block_number_hex,
                gas_used_hex,
            } => json!({
                "transactionHash": request["params"][0].clone(),
                "status": "0x1",
                "blockNumber": block_number_hex,
                "gasUsed": gas_used_hex,
            }),
            ReceiptMode::Reverted {
                block_number_hex,
                gas_used_hex,
            } => json!({
                "transactionHash": request["params"][0].clone(),
                "status": "0x0",
                "blockNumber": block_number_hex,
                "gasUsed": gas_used_hex,
            }),
        },
        other => json!({ "unsupported": other }),
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

async fn spawn_mock_evm_provider(state: RpcState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    async fn rpc_handler(
        State(state): State<RpcState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if auth != format!("Bearer {RPC_TOKEN}") {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "missing provider auth" })),
            );
        }
        let payload = if let Some(requests) = body.as_array() {
            Value::Array(
                requests
                    .iter()
                    .map(|request| rpc_response(&state, request))
                    .collect(),
            )
        } else {
            rpc_response(&state, &body)
        };
        (StatusCode::OK, Json(payload))
    }

    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

// ── HTTP helpers ─────────────────────────────────────────────────────────

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

async fn get(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = client.get(format!("http://{addr}{path}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

struct PlanEnv {
    _dir: TempDir,
    base_dir: PathBuf,
    addr: SocketAddr,
    daemon: tokio::task::JoinHandle<()>,
    rpc: tokio::task::JoinHandle<()>,
    rpc_state: RpcState,
    client: reqwest::Client,
    token: String,
}

impl PlanEnv {
    fn shutdown(self) {
        self.daemon.abort();
        self.rpc.abort();
    }
}

/// Compartment + provider + seed profile + inventory scan (mock RPC only).
async fn setup_plan_env() -> PlanEnv {
    let dir = TempDir::new().unwrap();
    let base_dir = dir.path().to_path_buf();
    let (addr, daemon) = spawn_daemon(base_dir.clone()).await;
    let rpc_state = RpcState::default();
    let (rpc_addr, rpc) = spawn_mock_evm_provider(rpc_state.clone()).await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": COMPARTMENT_PASSPHRASE,
        }),
        None,
    )
    .await;
    assert_eq!(init.status(), StatusCode::OK);
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": RPC_TOKEN }),
        Some(&token),
    )
    .await;
    assert_eq!(key.status(), StatusCode::OK);

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x12a05f200",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    let seed = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-main",
            "label": "Seed main",
            "mnemonic": SEED_MNEMONIC,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed.status(), StatusCode::OK);

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": "mainnet",
            "gap_limit": 1,
            "max_index": 0,
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");

    PlanEnv {
        _dir: dir,
        base_dir,
        addr,
        daemon,
        rpc,
        rpc_state,
        client,
        token,
    }
}

fn gates_on_policy_body() -> Value {
    json!({
        "enabled": true,
        "allowed_destinations": [{ "address": DESTINATION, "label": "test-treasury" }],
        "allow_plan_execution": true,
        "allow_sweep_execution": true,
        "allow_revoke_execution": true,
        "allow_exit_execution": true,
        "allow_claim_execution": true,
        "allow_gas_topups": true,
    })
}

async fn update_policy(env: &PlanEnv, body: Value) {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/treasury/policy/update",
        body,
        Some(&env.token),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "policy update response: {body}");
}

/// Generate + simulate a one-step (native sweep) plan; returns (plan_id, step_id).
async fn generate_and_simulate_plan(env: &PlanEnv) -> (String, String) {
    let plan = post_json(
        &env.client,
        env.addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": DESTINATION }),
        Some(&env.token),
    )
    .await;
    assert_eq!(plan.status(), StatusCode::OK);
    let plan_json: Value = plan.json().await.unwrap();
    let plan_id = plan_json["plan"]["id"].as_str().unwrap().to_string();

    let simulate = post_json(
        &env.client,
        env.addr,
        "/api/plans/consolidation/simulate",
        json!({ "plan_id": plan_id }),
        Some(&env.token),
    )
    .await;
    let simulate_status = simulate.status();
    let simulate_json: Value = simulate.json().await.unwrap();
    assert_eq!(
        simulate_status,
        StatusCode::OK,
        "simulate response: {simulate_json}"
    );
    let step = &simulate_json["plan"]["steps"][0];
    assert_eq!(step["action"], json!("sweep_native"), "step: {step}");
    assert_eq!(step["simulation_status"], json!("passed"), "step: {step}");
    let step_id = step["id"].as_str().unwrap().to_string();
    (plan_id, step_id)
}

async fn approve_plan(env: &PlanEnv, plan_id: &str) -> Value {
    let approve = post_json(
        &env.client,
        env.addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": plan_id }),
        Some(&env.token),
    )
    .await;
    let status = approve.status();
    let body: Value = approve.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "approve response: {body}");
    body
}

async fn enqueue_step(env: &PlanEnv, plan_id: &str, step_id: &str) -> (StatusCode, Value) {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/plans/enqueue-step",
        json!({ "plan_id": plan_id, "step_id": step_id, "confirm": true }),
        Some(&env.token),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    (status, body)
}

async fn process_queue(env: &PlanEnv) -> Value {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

async fn queue_jobs(env: &PlanEnv) -> Vec<Value> {
    let jobs = get(&env.client, env.addr, "/api/queue/jobs", Some(&env.token)).await;
    assert_eq!(jobs.status(), StatusCode::OK);
    let jobs_json: Value = jobs.json().await.unwrap();
    jobs_json["jobs"].as_array().unwrap().clone()
}

// ── Direct store surgery (tests craft states the API cannot produce) ───────

fn read_store(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn write_store(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn inventory_path(env: &PlanEnv) -> PathBuf {
    env.base_dir.join("wallet_inventory.json")
}

fn queue_path(env: &PlanEnv) -> PathBuf {
    env.base_dir.join("queue.json")
}

fn queue_job_by_step(env: &PlanEnv, step_id: &str) -> Value {
    let store = read_store(&queue_path(env));
    store["data"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .cloned()
        .unwrap_or_else(|| panic!("no persisted queue job for step {step_id}"))
}

/// Append a crafted step (cloned from an existing one) to the persisted plan.
fn add_plan_step(
    env: &PlanEnv,
    plan_id: &str,
    template_step_id: &str,
    edit: impl FnOnce(&mut Value),
) -> String {
    let path = inventory_path(env);
    let mut store = read_store(&path);
    let plan = store["data"]["consolidation_plans"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|plan| plan["id"] == json!(plan_id))
        .expect("plan exists");
    let steps = plan["steps"].as_array_mut().unwrap();
    let mut crafted = steps
        .iter()
        .find(|step| step["id"] == json!(template_step_id))
        .expect("template step exists")
        .clone();
    let new_id = format!("{template_step_id}-{}", steps.len());
    crafted["id"] = json!(new_id);
    crafted["queued_job_id"] = json!(null);
    crafted["approved"] = json!(false);
    edit(&mut crafted);
    steps.push(crafted);
    write_store(&path, &store);
    new_id
}

fn edit_queue_job(env: &PlanEnv, job_id: &str, edit: impl FnOnce(&mut Value)) {
    let path = queue_path(env);
    let mut store = read_store(&path);
    let job = store["data"]["jobs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|job| job["id"] == json!(job_id))
        .expect("job exists");
    edit(job);
    write_store(&path, &store);
}

// ── 1. Per-source serialization ─────────────────────────────────────────

#[tokio::test]
async fn per_source_serialization_defers_second_job_until_first_confirms() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_a_id) = generate_and_simulate_plan(&env).await;
    // A second, INDEPENDENT step on the SAME source address (no
    // depends_on) — distinct from the W6.4 dependency-chain case, which
    // must NOT be serialized (covered by plan_execution.rs's
    // `dependency_chain_executes_in_order_with_full_audit_trail`).
    let step_b_id = add_plan_step(&env, &plan_id, &step_a_id, |step| {
        step["depends_on"] = json!([]);
    });

    approve_plan(&env, &plan_id).await;
    let (status_a, body_a) = enqueue_step(&env, &plan_id, &step_a_id).await;
    assert_eq!(status_a, StatusCode::OK, "{body_a}");
    let (status_b, body_b) = enqueue_step(&env, &plan_id, &step_b_id).await;
    assert_eq!(status_b, StatusCode::OK, "{body_b}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["processed"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "only ONE of the two same-source jobs may broadcast"
    );

    let jobs = queue_jobs(&env).await;
    let job_a = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_a_id))
        .unwrap();
    let job_b = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_b_id))
        .unwrap();
    assert_eq!(job_a["state"], json!("sent"), "{job_a}");
    // E1-consistent representation (see the W7.4 report / plan_steps.rs
    // module doc): the deferred job stays `queued` — NOT the legacy
    // `deferred` wire string, NOT a new persisted state — with a visible,
    // transient skip reason.
    assert_eq!(job_b["state"], json!("queued"), "{job_b}");
    assert!(
        job_b["last_error"]
            .as_str()
            .unwrap()
            .starts_with("source_serialization:"),
        "{job_b}"
    );

    // The source frees up once job_a's receipt confirms (finality_blocks
    // defaults to 0 for the builtin mainnet profile) — job_b broadcasts in
    // the SAME subsequent drain call.
    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x2a".into(),
        gas_used_hex: "0x5208".into(),
    });
    let process_json_2 = process_queue(&env).await;
    assert_eq!(
        process_json_2["confirmed"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(
        process_json_2["succeeded"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(env.rpc_state.broadcast_call_count(), 2);

    let jobs_2 = queue_jobs(&env).await;
    let job_a_2 = jobs_2
        .iter()
        .find(|job| job["step_id"] == json!(step_a_id))
        .unwrap();
    let job_b_2 = jobs_2
        .iter()
        .find(|job| job["step_id"] == json!(step_b_id))
        .unwrap();
    assert_eq!(job_a_2["state"], json!("confirmed"), "{job_a_2}");
    assert_eq!(job_a_2["receipt_status"], json!("success"), "{job_a_2}");
    assert_eq!(job_a_2["receipt_block_number"], json!(42), "{job_a_2}");
    assert_eq!(job_b_2["state"], json!("sent"), "{job_b_2}");

    env.shutdown();
}

// ── 2. Nonce race: park without re-signing ──────────────────────────────

#[tokio::test]
async fn nonce_too_low_parks_without_resigning() {
    let env = setup_plan_env().await;
    env.rpc_state.set_broadcast_error(Some("nonce too low"));
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Setup (inventory scan, simulation) also calls `eth_getTransactionCount`
    // — count only the calls made DURING this drain.
    let nonce_calls_before_drain = env.rpc_state.transaction_count_call_count();

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["failures_by_cause"]["broadcast_rejected"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "a prepared transaction is submitted once and never re-signed in place"
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count() - nonce_calls_before_drain,
        1,
        "the nonce is fetched once while preparing the durable signed bytes"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("operator_action_required"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("broadcast_rejected: prepared transaction nonce"),
        "{job}"
    );
    assert!(
        job["transaction_hash_hex"].is_string(),
        "operator review retains the prepared hash: {job}"
    );
    assert!(
        job.get("signed_raw_transaction_hex").is_none(),
        "queue APIs redact replayable signed bytes: {job}"
    );
    let persisted_job = queue_job_by_step(&env, &step_id);
    assert!(
        persisted_job.get("signed_raw_transaction_hex").is_none(),
        "terminal jobs discard replayable signed bytes from storage: {persisted_job}"
    );

    env.shutdown();
}

// ── 3. On-chain revert: never retried ───────────────────────────────────

#[tokio::test]
async fn on_chain_revert_at_broadcast_parks_without_retry() {
    let env = setup_plan_env().await;
    env.rpc_state
        .set_broadcast_error(Some("execution reverted"));
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["failures_by_cause"]["on_chain_revert"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "a revert must NEVER be retried (generalizes the W7.3 claim-only rule)"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("operator_action_required"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("on_chain_revert:"),
        "{job}"
    );

    env.shutdown();
}

/// Bonus coverage: a revert discovered LATER via receipt polling (the tx
/// broadcast fine, was mined, but the receipt's status is failure) — the
/// OTHER way a revert is discovered, distinct from the broadcast-time
/// rejection above. Gas used and block number ARE available here (the tx
/// was actually mined) and must be recorded truthfully.
#[tokio::test]
async fn on_chain_revert_discovered_via_receipt_records_gas_and_block() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("sent"), "{job}");
    assert_eq!(job["source_address"], json!(SEED_ADDRESS), "{job}");

    env.rpc_state.set_receipt_mode(ReceiptMode::Reverted {
        block_number_hex: "0x64".into(),
        gas_used_hex: "0x5208".into(),
    });
    let process_json_2 = process_queue(&env).await;
    assert_eq!(
        process_json_2["operator_action_required"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(
        process_json_2["failures_by_cause"]["on_chain_revert"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "a receipt-discovered revert must never trigger a re-broadcast"
    );

    let jobs_2 = queue_jobs(&env).await;
    let job_2 = jobs_2
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job_2["state"], json!("operator_action_required"), "{job_2}");
    assert_eq!(job_2["receipt_status"], json!("reverted"), "{job_2}");
    assert_eq!(job_2["receipt_block_number"], json!(100), "{job_2}");
    assert_eq!(job_2["receipt_gas_used_hex"], json!("0x5208"), "{job_2}");
    assert!(
        job_2["last_error"]
            .as_str()
            .unwrap()
            .starts_with("on_chain_revert:"),
        "{job_2}"
    );

    env.shutdown();
}

// ── 4. Underpriced: park without re-signing ─────────────────────────────

#[tokio::test]
async fn underpriced_broadcast_parks_without_resigning() {
    let env = setup_plan_env().await;
    env.rpc_state
        .set_broadcast_error(Some("replacement transaction underpriced"));
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Setup (inventory scan, simulation) also calls `eth_getTransactionCount`
    // — count only the calls made DURING this drain.
    let nonce_calls_before_drain = env.rpc_state.transaction_count_call_count();

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["failures_by_cause"]["broadcast_rejected"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "underpriced prepared bytes are never replaced by an implicit re-sign"
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count() - nonce_calls_before_drain,
        1,
        "an underpriced rejection never re-fetches the nonce"
    );

    let raw_hexes = env.rpc_state.broadcast_raw_hexes();
    assert_eq!(raw_hexes.len(), 1);

    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("operator_action_required"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("broadcast_rejected: prepared transaction is underpriced"),
        "{job}"
    );
    assert!(
        job.get("signed_raw_transaction_hex").is_none(),
        "queue APIs redact replayable signed bytes: {job}"
    );
    let persisted_job = queue_job_by_step(&env, &step_id);
    assert!(
        persisted_job.get("signed_raw_transaction_hex").is_none(),
        "terminal jobs discard replayable signed bytes from storage: {persisted_job}"
    );

    env.shutdown();
}

// ── 5. Receipt timeout: never assume failure ────────────────────────────

#[tokio::test]
async fn receipt_timeout_parks_with_tx_hash_never_assuming_failure() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("sent"), "{job}");
    let job_id = job["id"].as_str().unwrap().to_string();
    let tx_hash = job["transaction_hash_hex"].as_str().unwrap().to_string();

    // Simulate the confirmation wall-clock budget already elapsed (store
    // surgery — a real test cannot wait out a 3600s window). The chain
    // still has no receipt for it (`Pending`): the broadcast is NEVER
    // assumed to have failed just because it hasn't confirmed yet.
    edit_queue_job(&env, &job_id, |job| {
        job["broadcast_at_unix"] = json!(now_unix_test() - 4000);
    });
    env.rpc_state.set_receipt_mode(ReceiptMode::Pending);

    let process_json_2 = process_queue(&env).await;
    assert_eq!(
        process_json_2["operator_action_required"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(
        process_json_2["failures_by_cause"]["receipt_timeout"],
        json!(1),
        "process: {process_json_2}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "a receipt timeout must NEVER re-broadcast"
    );

    let jobs_2 = queue_jobs(&env).await;
    let job_2 = jobs_2
        .iter()
        .find(|job| job["id"] == json!(job_id))
        .unwrap();
    assert_eq!(job_2["state"], json!("operator_action_required"), "{job_2}");
    let last_error = job_2["last_error"].as_str().unwrap();
    assert!(last_error.starts_with("receipt_timeout:"), "{last_error}");
    assert!(
        last_error.contains(&tx_hash),
        "reason must carry the tx hash: {last_error}"
    );
    // Truthful: the job's OWN recorded tx hash is untouched — the
    // broadcast is not un-recorded just because it hasn't confirmed.
    assert_eq!(job_2["transaction_hash_hex"], json!(tx_hash));

    env.shutdown();
}

// ── 6. Crash resumption: restart resumes polling, never duplicates ─────

#[tokio::test]
async fn restart_resumes_receipt_polling_without_duplicate_broadcast() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(env.rpc_state.broadcast_call_count(), 1);

    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job["state"], json!("sent"), "{job}");
    let tx_hash = job["transaction_hash_hex"].as_str().unwrap().to_string();

    // "kill -9" the daemon mid-in-flight (job already broadcast, awaiting
    // confirmation) — abort with no clean shutdown.
    env.daemon.abort();

    // Restart: a FRESH daemon process on the SAME base_dir. The mock RPC
    // provider is the "chain" — untouched by the daemon restart.
    let (addr2, daemon2) = spawn_daemon(env.base_dir.clone()).await;

    // A real restart loses in-memory sessions; re-authenticate.
    let unlock = post_json(
        &env.client,
        addr2,
        "/api/unlock",
        json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
        None,
    )
    .await;
    assert_eq!(unlock.status(), StatusCode::OK, "{unlock:?}");
    let unlock_json: Value = unlock.json().await.unwrap();
    let token2 = unlock_json["session_token"].as_str().unwrap().to_string();

    // The chain now confirms the tx that was ALREADY broadcast before the
    // crash.
    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x2a".into(),
        gas_used_hex: "0x5208".into(),
    });

    let process2 = post_json(
        &env.client,
        addr2,
        "/api/queue/process",
        json!({}),
        Some(&token2),
    )
    .await;
    assert_eq!(process2.status(), StatusCode::OK);
    let process2_json: Value = process2.json().await.unwrap();
    assert_eq!(
        process2_json["confirmed"],
        json!(1),
        "process: {process2_json}"
    );

    // NO duplicate broadcast: the mock RPC's counter (shared across both
    // daemon processes — it is the chain, not daemon state) is still
    // exactly the ONE call from before the crash.
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "restart must resume receipt polling, never re-sign or re-broadcast"
    );

    let jobs_after = get(&env.client, addr2, "/api/queue/jobs", Some(&token2)).await;
    assert_eq!(jobs_after.status(), StatusCode::OK);
    let jobs_after_json: Value = jobs_after.json().await.unwrap();
    let job_after = jobs_after_json["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert_eq!(job_after["state"], json!("confirmed"), "{job_after}");
    assert_eq!(
        job_after["transaction_hash_hex"],
        json!(tx_hash),
        "same transaction, never re-signed"
    );
    assert_eq!(job_after["receipt_status"], json!("success"), "{job_after}");

    daemon2.abort();
    env.shutdown();
}

#[tokio::test]
async fn execution_gate_flips_preserve_every_in_flight_plan_step_state() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let first = process_queue(&env).await;
    assert_eq!(first["succeeded"], json!(1), "{first}");
    let sent_job = queue_job_by_step(&env, &step_id);
    let job_id = sent_job["id"].as_str().unwrap().to_string();
    let signed_raw = env
        .rpc_state
        .broadcast_raw_hexes()
        .last()
        .expect("initial process broadcasts one prepared transaction")
        .trim_start_matches("0x")
        .to_string();
    let broadcasts_after_sign = env.rpc_state.broadcast_call_count();
    let nonce_calls_after_sign = env.rpc_state.transaction_count_call_count();

    env.rpc_state.set_receipt_mode(ReceiptMode::Pending);
    update_policy(&env, json!({ "enabled": true })).await;

    // `sent` is irrevocably receipt-only. Disabling the action gate cannot
    // demote it to `blocked`, restore replay bytes, or enter the signer.
    let sent_gate_off = process_queue(&env).await;
    assert_eq!(sent_gate_off["jobs"][0]["state"], json!("sent"));
    assert_eq!(env.rpc_state.broadcast_call_count(), broadcasts_after_sign);
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    let sent_on_disk = queue_job_by_step(&env, &step_id);
    assert_eq!(sent_on_disk["state"], json!("sent"));
    assert!(sent_on_disk["signed_raw_transaction_hex"].is_null());

    // A pre-I/O crash snapshot holds its exact bytes while the gate is off.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("prepared");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = Value::Null;
        job["next_attempt_after_unix"] = Value::Null;
        job["receipt_status"] = Value::Null;
    });
    let prepared_gate_off = process_queue(&env).await;
    assert_eq!(prepared_gate_off["jobs"][0]["state"], json!("prepared"));
    assert_eq!(env.rpc_state.broadcast_call_count(), broadcasts_after_sign);
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    let prepared_on_disk = queue_job_by_step(&env, &step_id);
    assert_eq!(
        prepared_on_disk["signed_raw_transaction_hex"],
        json!(signed_raw)
    );

    // Re-enabling permits exactly one first submission of the original bytes.
    update_policy(&env, gates_on_policy_body()).await;
    let prepared_gate_on = process_queue(&env).await;
    assert_eq!(
        prepared_gate_on["succeeded"],
        json!(1),
        "{prepared_gate_on}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_after_sign + 1
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    assert_eq!(
        env.rpc_state
            .broadcast_raw_hexes()
            .last()
            .unwrap()
            .trim_start_matches("0x"),
        signed_raw
    );

    // An ambiguous submission always checks its receipt first. With the gate
    // denied and no receipt it must retain `submitted_unknown` and its bytes.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("submitted_unknown");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = json!(now_unix_test());
        job["next_attempt_after_unix"] = Value::Null;
        job["receipt_status"] = Value::Null;
    });
    update_policy(&env, json!({ "enabled": true })).await;
    let broadcasts_before_unknown_hold = env.rpc_state.broadcast_call_count();
    let unknown_gate_off = process_queue(&env).await;
    assert_eq!(
        unknown_gate_off["jobs"][0]["state"],
        json!("submitted_unknown")
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_before_unknown_hold
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    let unknown_on_disk = queue_job_by_step(&env, &step_id);
    assert_eq!(unknown_on_disk["state"], json!("submitted_unknown"));
    assert_eq!(
        unknown_on_disk["signed_raw_transaction_hex"],
        json!(signed_raw)
    );

    // Once the gate is restored, a missing receipt permits exactly one
    // byte-identical resubmission and still never re-enters signing.
    edit_queue_job(&env, &job_id, |job| {
        job["next_attempt_after_unix"] = Value::Null;
    });
    update_policy(&env, gates_on_policy_body()).await;
    let unknown_gate_on = process_queue(&env).await;
    assert_eq!(unknown_gate_on["succeeded"], json!(1), "{unknown_gate_on}");
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_before_unknown_hold + 1
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    assert_eq!(
        env.rpc_state
            .broadcast_raw_hexes()
            .last()
            .unwrap()
            .trim_start_matches("0x"),
        signed_raw
    );

    // Repair the exact legacy state created by the old ordering: `blocked`
    // with raw bytes plus a broadcast marker. Even with the gate off, a
    // visible receipt confirms by hash without a new broadcast or signature.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("blocked");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = json!(now_unix_test());
        job["next_attempt_after_unix"] = Value::Null;
        job["receipt_status"] = Value::Null;
    });
    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x1".into(),
        gas_used_hex: "0x5208".into(),
    });
    update_policy(&env, json!({ "enabled": true })).await;
    let broadcasts_before_legacy_recovery = env.rpc_state.broadcast_call_count();
    let recovered = process_queue(&env).await;
    assert_eq!(recovered["confirmed"], json!(1), "{recovered}");
    assert_eq!(recovered["jobs"][0]["state"], json!("confirmed"));
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_before_legacy_recovery
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    let confirmed_on_disk = queue_job_by_step(&env, &step_id);
    assert_eq!(confirmed_on_disk["state"], json!("confirmed"));
    assert!(confirmed_on_disk["signed_raw_transaction_hex"].is_null());

    env.shutdown();
}

#[tokio::test]
async fn prepared_and_submitted_unknown_snapshots_never_resign() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Produce one real signed transaction so the fixture can model the two
    // durable crash snapshots exactly as they appear on disk.
    let first = process_queue(&env).await;
    assert_eq!(first["succeeded"], json!(1), "{first}");
    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .unwrap();
    assert!(
        job.get("signed_raw_transaction_hex").is_none(),
        "queue APIs redact replayable signed bytes: {job}"
    );
    let job_id = job["id"].as_str().unwrap().to_string();
    let transaction_hash_hex = job["transaction_hash_hex"].as_str().unwrap().to_string();
    let submitted_raw_hexes = env.rpc_state.broadcast_raw_hexes();
    let signed_raw = submitted_raw_hexes
        .last()
        .expect("the initial process submitted one prepared transaction")
        .trim_start_matches("0x")
        .to_string();
    let nonce_calls_after_sign = env.rpc_state.transaction_count_call_count();

    // Crash snapshot A: signed bytes reached stable storage, but no network
    // call was durably recorded. Restart must submit these bytes verbatim and
    // must not touch the signer/nonce path.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("prepared");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = Value::Null;
        job["receipt_status"] = Value::Null;
    });
    env.daemon.abort();
    let (addr2, daemon2) = spawn_daemon(env.base_dir.clone()).await;
    let unlock2 = post_json(
        &env.client,
        addr2,
        "/api/unlock",
        json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
        None,
    )
    .await;
    assert_eq!(unlock2.status(), StatusCode::OK, "{unlock2:?}");
    let token2 = unlock2.json::<Value>().await.unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_string();
    let process2 = post_json(
        &env.client,
        addr2,
        "/api/queue/process",
        json!({}),
        Some(&token2),
    )
    .await;
    assert_eq!(process2.status(), StatusCode::OK);
    let process2_json: Value = process2.json().await.unwrap();
    assert_eq!(process2_json["succeeded"], json!(1), "{process2_json}");
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    assert_eq!(
        env.rpc_state
            .broadcast_raw_hexes()
            .last()
            .unwrap()
            .trim_start_matches("0x"),
        signed_raw
    );

    // Crash snapshot B with no receipt: query first, then idempotently submit
    // the exact bytes. This still must not re-enter signing.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("submitted_unknown");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = json!(now_unix_test() - 4000);
    });
    env.rpc_state.set_receipt_mode(ReceiptMode::Pending);
    let broadcasts_before_unknown = env.rpc_state.broadcast_call_count();
    let process_unknown = post_json(
        &env.client,
        addr2,
        "/api/queue/process",
        json!({}),
        Some(&token2),
    )
    .await;
    assert_eq!(process_unknown.status(), StatusCode::OK);
    let process_unknown_json: Value = process_unknown.json().await.unwrap();
    assert_eq!(
        process_unknown_json["succeeded"],
        json!(1),
        "{process_unknown_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_before_unknown + 1
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    assert_eq!(
        env.rpc_state
            .broadcast_raw_hexes()
            .last()
            .unwrap()
            .trim_start_matches("0x"),
        signed_raw
    );

    // The same submitted-unknown snapshot with a now-visible receipt is
    // resolved by hash only: no re-broadcast and still no signing.
    edit_queue_job(&env, &job_id, |job| {
        job["state"] = json!("submitted_unknown");
        job["signed_raw_transaction_hex"] = json!(signed_raw.clone());
        job["broadcast_transaction_hash_hex"] = Value::Null;
        job["broadcast_at_unix"] = json!(now_unix_test());
    });
    daemon2.abort();
    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x2a".into(),
        gas_used_hex: "0x5208".into(),
    });
    let broadcasts_before_receipt = env.rpc_state.broadcast_call_count();
    let (addr3, daemon3) = spawn_daemon(env.base_dir.clone()).await;
    let unlock3 = post_json(
        &env.client,
        addr3,
        "/api/unlock",
        json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
        None,
    )
    .await;
    assert_eq!(unlock3.status(), StatusCode::OK, "{unlock3:?}");
    let token3 = unlock3.json::<Value>().await.unwrap()["session_token"]
        .as_str()
        .unwrap()
        .to_string();
    let process3 = post_json(
        &env.client,
        addr3,
        "/api/queue/process",
        json!({}),
        Some(&token3),
    )
    .await;
    assert_eq!(process3.status(), StatusCode::OK);
    let process3_json: Value = process3.json().await.unwrap();
    assert_eq!(process3_json["confirmed"], json!(1), "{process3_json}");
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        broadcasts_before_receipt
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_sign
    );
    assert_eq!(
        process3_json["jobs"][0]["transaction_hash_hex"],
        json!(transaction_hash_hex)
    );

    daemon3.abort();
    env.shutdown();
}

/// Earlier in-process abort coverage retained as a fast complement to the
/// feature-gated real SIGKILL proof below.
#[tokio::test]
async fn legacy_in_process_abort_resumes_without_duplicate_broadcast() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        0,
        "enqueue must not broadcast before queue processing"
    );
    let jobs = queue_jobs(&env).await;
    let step_jobs: Vec<_> = jobs
        .iter()
        .filter(|job| job["step_id"] == json!(step_id))
        .collect();
    assert_eq!(step_jobs.len(), 1, "jobs: {jobs:?}");
    assert_eq!(step_jobs[0]["state"], json!("queued"), "{}", step_jobs[0]);

    // Kill #1: pre-broadcast window. The queued job is persisted as queued,
    // but the daemon dies before any raw transaction reaches the mock chain.
    env.daemon.abort();

    let (addr2, daemon2) = spawn_daemon(env.base_dir.clone()).await;
    let unlock2 = post_json(
        &env.client,
        addr2,
        "/api/unlock",
        json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
        None,
    )
    .await;
    assert_eq!(unlock2.status(), StatusCode::OK, "{unlock2:?}");
    let unlock2_json: Value = unlock2.json().await.unwrap();
    let token2 = unlock2_json["session_token"].as_str().unwrap().to_string();

    let process2 = post_json(
        &env.client,
        addr2,
        "/api/queue/process",
        json!({}),
        Some(&token2),
    )
    .await;
    assert_eq!(process2.status(), StatusCode::OK);
    let process2_json: Value = process2.json().await.unwrap();
    assert_eq!(
        process2_json["succeeded"],
        json!(1),
        "process: {process2_json}"
    );
    assert_eq!(env.rpc_state.broadcast_call_count(), 1);

    let jobs2_response = get(&env.client, addr2, "/api/queue/jobs", Some(&token2)).await;
    assert_eq!(jobs2_response.status(), StatusCode::OK);
    let jobs2_json: Value = jobs2_response.json().await.unwrap();
    let jobs2 = jobs2_json["jobs"].as_array().unwrap();
    let step_jobs2: Vec<_> = jobs2
        .iter()
        .filter(|job| job["step_id"] == json!(step_id))
        .collect();
    assert_eq!(step_jobs2.len(), 1, "jobs: {jobs2:?}");
    assert_eq!(step_jobs2[0]["state"], json!("sent"), "{}", step_jobs2[0]);
    let tx_hash = step_jobs2[0]["transaction_hash_hex"]
        .as_str()
        .unwrap()
        .to_string();

    // Kill #2: post-broadcast window. The transaction hash is persisted and
    // only receipt polling should resume after restart.
    daemon2.abort();

    let (addr3, daemon3) = spawn_daemon(env.base_dir.clone()).await;
    let unlock3 = post_json(
        &env.client,
        addr3,
        "/api/unlock",
        json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
        None,
    )
    .await;
    assert_eq!(unlock3.status(), StatusCode::OK, "{unlock3:?}");
    let unlock3_json: Value = unlock3.json().await.unwrap();
    let token3 = unlock3_json["session_token"].as_str().unwrap().to_string();

    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x2a".into(),
        gas_used_hex: "0x5208".into(),
    });

    let process3 = post_json(
        &env.client,
        addr3,
        "/api/queue/process",
        json!({}),
        Some(&token3),
    )
    .await;
    assert_eq!(process3.status(), StatusCode::OK);
    let process3_json: Value = process3.json().await.unwrap();
    assert_eq!(
        process3_json["confirmed"],
        json!(1),
        "process: {process3_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "post-broadcast restart must poll receipts without re-signing"
    );

    let jobs3_response = get(&env.client, addr3, "/api/queue/jobs", Some(&token3)).await;
    assert_eq!(jobs3_response.status(), StatusCode::OK);
    let jobs3_json: Value = jobs3_response.json().await.unwrap();
    let jobs3 = jobs3_json["jobs"].as_array().unwrap();
    let step_jobs3: Vec<_> = jobs3
        .iter()
        .filter(|job| job["step_id"] == json!(step_id))
        .collect();
    assert_eq!(step_jobs3.len(), 1, "jobs: {jobs3:?}");
    assert_eq!(
        step_jobs3[0]["state"],
        json!("confirmed"),
        "{}",
        step_jobs3[0]
    );
    assert_eq!(
        step_jobs3[0]["transaction_hash_hex"],
        json!(tx_hash),
        "terminal job must keep the original transaction hash"
    );
    assert_eq!(
        step_jobs3[0]["receipt_status"],
        json!("success"),
        "{}",
        step_jobs3[0]
    );

    daemon3.abort();
    env.shutdown();
}

#[cfg(all(unix, feature = "test-failpoints"))]
const FAILPOINT_HELPER_ENV: &str = "SIGILLUM_TEST_QUEUE_HELPER";
#[cfg(all(unix, feature = "test-failpoints"))]
const FAILPOINT_BASE_DIR_ENV: &str = "SIGILLUM_TEST_QUEUE_BASE_DIR";
#[cfg(all(unix, feature = "test-failpoints"))]
const FAILPOINT_ACTIVE_ENV: &str = "SIGILLUM_TEST_FAILPOINT";
#[cfg(all(unix, feature = "test-failpoints"))]
const FAILPOINT_READY_PATH_ENV: &str = "SIGILLUM_TEST_FAILPOINT_READY_PATH";
#[cfg(all(unix, feature = "test-failpoints"))]
const AFTER_PREPARED_PERSIST: &str = "queue_after_prepared_persist";
#[cfg(all(unix, feature = "test-failpoints"))]
const AFTER_SUBMITTED_UNKNOWN_PERSIST: &str = "queue_after_submitted_unknown_persist";

#[cfg(all(unix, feature = "test-failpoints"))]
fn spawn_queue_failpoint_helper(base_dir: &Path, failpoint: Option<(&str, &Path)>) -> Child {
    let executable = std::env::current_exe().expect("current test executable is available");
    let mut command = Command::new(executable);
    command
        .arg("--exact")
        .arg("queue_failpoint_subprocess_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(FAILPOINT_HELPER_ENV, "1")
        .env(FAILPOINT_BASE_DIR_ENV, base_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some((name, ready_path)) = failpoint {
        command
            .env(FAILPOINT_ACTIVE_ENV, name)
            .env(FAILPOINT_READY_PATH_ENV, ready_path);
    } else {
        command
            .env_remove(FAILPOINT_ACTIVE_ENV)
            .env_remove(FAILPOINT_READY_PATH_ENV);
    }
    command.spawn().expect("queue failpoint helper starts")
}

#[cfg(all(unix, feature = "test-failpoints"))]
async fn wait_for_failpoint_marker(child: &mut Child, marker: &Path) {
    let wait = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if marker.exists() {
                return;
            }
            if let Some(status) = child.try_wait().expect("helper status is readable") {
                panic!("queue failpoint helper exited before marker: {status}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if wait.is_err() {
        if child
            .try_wait()
            .expect("helper status is readable")
            .is_none()
        {
            kill_helper(child);
        }
        panic!(
            "timed out waiting for failpoint marker {}",
            marker.display()
        );
    }
}

#[cfg(all(unix, feature = "test-failpoints"))]
async fn wait_for_helper_success(child: &mut Child) -> ExitStatus {
    match tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(status) = child.try_wait().expect("helper status is readable") {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    {
        Ok(status) => {
            assert!(status.success(), "queue helper failed: {status}");
            status
        }
        Err(_) => {
            kill_helper(child);
            panic!("timed out waiting for queue helper to exit");
        }
    }
}

#[cfg(all(unix, feature = "test-failpoints"))]
fn kill_helper(child: &mut Child) {
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGKILL) };
    assert_eq!(result, 0, "SIGKILL should reach queue helper");
    let status = child.wait().expect("killed queue helper is reaped");
    assert_eq!(status.signal(), Some(libc::SIGKILL));
}

#[cfg(all(unix, feature = "test-failpoints"))]
fn required_string(job: &Value, field: &str) -> String {
    job[field]
        .as_str()
        .unwrap_or_else(|| panic!("persisted job field {field} is missing: {job}"))
        .to_string()
}

/// Subprocess entry point used by the crash-boundary proof. A normal test
/// process returns immediately; only an explicitly marked child drains.
#[cfg(all(unix, feature = "test-failpoints"))]
#[test]
fn queue_failpoint_subprocess_helper() {
    if std::env::var_os(FAILPOINT_HELPER_ENV).is_none() {
        return;
    }
    let base_dir = PathBuf::from(
        std::env::var_os(FAILPOINT_BASE_DIR_ENV)
            .expect("queue helper base directory is configured"),
    );
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("queue helper runtime builds")
        .block_on(async move {
            let (addr, daemon) = spawn_daemon(base_dir).await;
            let client = reqwest::Client::new();
            let unlock = post_json(
                &client,
                addr,
                "/api/unlock",
                json!({ "passphrase": COMPARTMENT_PASSPHRASE }),
                None,
            )
            .await;
            assert_eq!(unlock.status(), StatusCode::OK);
            let unlock_json: Value = unlock.json().await.unwrap();
            let token = unlock_json["session_token"]
                .as_str()
                .expect("unlock returns a session token")
                .to_string();
            let process =
                post_json(&client, addr, "/api/queue/process", json!({}), Some(&token)).await;
            assert_eq!(process.status(), StatusCode::OK);
            let process_json: Value = process.json().await.unwrap();
            assert_eq!(process_json["processed"], json!(1), "{process_json}");
            daemon.abort();
        });
}

/// Real OS-kill proof for both queue write-ahead barriers. The daemon child
/// is SIGKILLed only after a synced marker proves the selected durable state
/// exists and before any RPC submission can begin.
#[cfg(all(unix, feature = "test-failpoints"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chaos_kill_in_flight_plan_step_resumes_terminal_without_duplication() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    env.daemon.abort();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let nonce_calls_before_prepare = env.rpc_state.transaction_count_call_count();
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        0,
        "enqueue must not broadcast"
    );

    // Crash boundary A: exact signed bytes and all integrity bindings are on
    // disk as `prepared`, but the submission marker and RPC call do not exist.
    let prepared_marker = env.base_dir.join("queue-prepared.ready");
    let mut prepared_child = spawn_queue_failpoint_helper(
        &env.base_dir,
        Some((AFTER_PREPARED_PERSIST, &prepared_marker)),
    );
    wait_for_failpoint_marker(&mut prepared_child, &prepared_marker).await;
    let prepared = queue_job_by_step(&env, &step_id);
    assert_eq!(prepared["state"], json!("prepared"), "{prepared}");
    let raw = required_string(&prepared, "signed_raw_transaction_hex");
    let transaction_hash = required_string(&prepared, "transaction_hash_hex");
    let payload_hash = required_string(&prepared, "prepared_payload_hash_hex");
    let binding_hash = required_string(&prepared, "prepared_binding_hash_hex");
    assert!(prepared["prepared_at_unix"].is_number(), "{prepared}");
    assert!(prepared["broadcast_at_unix"].is_null(), "{prepared}");
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_before_prepare + 1,
        "only initial preparation resolves a nonce"
    );
    let nonce_calls_after_prepare = env.rpc_state.transaction_count_call_count();
    assert_eq!(env.rpc_state.broadcast_call_count(), 0);
    kill_helper(&mut prepared_child);

    // Crash boundary B: `submitted_unknown` is durable, but the failpoint is
    // still before RPC. Recovery must carry the exact prepared identity over
    // without resolving a signer or nonce again.
    let submitted_marker = env.base_dir.join("queue-submitted-unknown.ready");
    let mut submitted_child = spawn_queue_failpoint_helper(
        &env.base_dir,
        Some((AFTER_SUBMITTED_UNKNOWN_PERSIST, &submitted_marker)),
    );
    wait_for_failpoint_marker(&mut submitted_child, &submitted_marker).await;
    let submitted = queue_job_by_step(&env, &step_id);
    assert_eq!(
        submitted["state"],
        json!("submitted_unknown"),
        "{submitted}"
    );
    assert_eq!(
        required_string(&submitted, "signed_raw_transaction_hex"),
        raw
    );
    assert_eq!(
        required_string(&submitted, "transaction_hash_hex"),
        transaction_hash
    );
    assert_eq!(
        required_string(&submitted, "prepared_payload_hash_hex"),
        payload_hash
    );
    assert_eq!(
        required_string(&submitted, "prepared_binding_hash_hex"),
        binding_hash
    );
    assert!(submitted["broadcast_at_unix"].is_number(), "{submitted}");
    assert_eq!(env.rpc_state.broadcast_call_count(), 0);
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_prepare,
        "submitted_unknown recovery must not re-sign"
    );
    kill_helper(&mut submitted_child);

    // With the hooks disabled, recovery polls once, finds no receipt, and
    // submits the exact bytes once. Provider acceptance clears replay bytes.
    env.rpc_state.set_receipt_mode(ReceiptMode::Pending);
    let mut broadcast_child = spawn_queue_failpoint_helper(&env.base_dir, None);
    wait_for_helper_success(&mut broadcast_child).await;
    let sent = queue_job_by_step(&env, &step_id);
    assert_eq!(sent["state"], json!("sent"), "{sent}");
    assert_eq!(sent["transaction_hash_hex"], json!(transaction_hash));
    assert_eq!(
        sent["broadcast_transaction_hash_hex"],
        json!(transaction_hash)
    );
    assert!(
        sent.get("signed_raw_transaction_hex").is_none(),
        "accepted submissions discard replayable raw bytes: {sent}"
    );
    assert_eq!(sent["prepared_payload_hash_hex"], json!(payload_hash));
    assert_eq!(sent["prepared_binding_hash_hex"], json!(binding_hash));
    assert_eq!(env.rpc_state.broadcast_call_count(), 1);
    assert_eq!(
        env.rpc_state.broadcast_raw_hexes()[0].trim_start_matches("0x"),
        raw
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_prepare,
        "exact-byte recovery must not resolve another nonce"
    );

    // A final restart confirms by receipt identity only. It cannot broadcast
    // or sign again, and terminal storage remains free of replayable bytes.
    env.rpc_state.set_receipt_mode(ReceiptMode::Success {
        block_number_hex: "0x2a".into(),
        gas_used_hex: "0x5208".into(),
    });
    let mut confirmation_child = spawn_queue_failpoint_helper(&env.base_dir, None);
    wait_for_helper_success(&mut confirmation_child).await;
    let confirmed = queue_job_by_step(&env, &step_id);
    assert_eq!(confirmed["state"], json!("confirmed"), "{confirmed}");
    assert_eq!(confirmed["transaction_hash_hex"], json!(transaction_hash));
    assert_eq!(confirmed["receipt_status"], json!("success"));
    assert_eq!(
        env.rpc_state.broadcast_call_count(),
        1,
        "confirmation restart must not duplicate the broadcast"
    );
    assert_eq!(
        env.rpc_state.transaction_count_call_count(),
        nonce_calls_after_prepare
    );
    assert!(confirmed.get("signed_raw_transaction_hex").is_none());
    env.shutdown();
}
