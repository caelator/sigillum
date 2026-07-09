//! W7.2 integration tests: plan-step enqueue routes, validation negatives,
//! idempotency, dependency chaining, typed confirmation, drain-time hard
//! block, and gates-off preservation. Mock-RPC only; every daemon runs in a
//! fresh TempDir on an ephemeral port.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

const DESTINATION: &str = "0x9999999999999999999999999999999999999999";
const SEED_ADDRESS: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const ONE_ETH_HEX: &str = "0xde0b6b3a7640000";
const ONE_ETH_DECIMAL: &str = "1000000000000000000";

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

#[derive(Clone)]
struct RpcState;

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn rpc_response(request: &Value) -> Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getTransactionCount" => json!("0x7"),
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
            "eth_sendRawTransaction" => json!(format!("0x{}", "11".repeat(32))),
            other => json!({ "unsupported": other }),
        };
        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(json!(1)),
            "result": result,
        })
    }

    async fn rpc_handler(
        State(_state): State<RpcState>,
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
        let payload = if let Some(requests) = body.as_array() {
            Value::Array(requests.iter().map(rpc_response).collect())
        } else {
            rpc_response(&body)
        };
        (StatusCode::OK, Json(payload))
    }

    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(RpcState);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

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
    let (rpc_addr, rpc) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
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
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
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
            "mnemonic": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
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
        client,
        token,
    }
}

/// Treasury policy body with every W7.1 execution gate ON and the sweep
/// destination allowlisted. Tests flip individual fields off from here.
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

/// Full happy setup: gates-on policy, generated + simulated + approved plan.
async fn approved_plan_env() -> (PlanEnv, String, String) {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    let approved = approve_plan(&env, &plan_id).await;
    assert_eq!(
        approved["plan"]["steps"][0]["approved"],
        json!(true),
        "approve response: {approved}"
    );
    (env, plan_id, step_id)
}

async fn enqueue_step(
    env: &PlanEnv,
    plan_id: &str,
    step_id: &str,
    confirm: bool,
) -> (StatusCode, Value) {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/plans/enqueue-step",
        json!({ "plan_id": plan_id, "step_id": step_id, "confirm": confirm }),
        Some(&env.token),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    (status, body)
}

async fn enqueue_plan(env: &PlanEnv, plan_id: &str, confirmation: &str) -> (StatusCode, Value) {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/plans/enqueue-plan",
        json!({ "plan_id": plan_id, "confirmation": confirmation }),
        Some(&env.token),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    (status, body)
}

async fn queue_jobs(env: &PlanEnv) -> Vec<Value> {
    let jobs = get(&env.client, env.addr, "/api/queue/jobs", Some(&env.token)).await;
    assert_eq!(jobs.status(), StatusCode::OK);
    let jobs_json: Value = jobs.json().await.unwrap();
    jobs_json["jobs"].as_array().unwrap().clone()
}

async fn plan_step(env: &PlanEnv, plan_id: &str, step_id: &str) -> Value {
    let plans = get(
        &env.client,
        env.addr,
        "/api/plans/consolidation",
        Some(&env.token),
    )
    .await;
    assert_eq!(plans.status(), StatusCode::OK);
    let plans_json: Value = plans.json().await.unwrap();
    plans_json["plans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["id"] == json!(plan_id))
        .and_then(|plan| {
            plan["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["id"] == json!(step_id))
        })
        .cloned()
        .unwrap_or_else(|| panic!("step {step_id} not found in plan {plan_id}"))
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

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Mutate one plan step inside the persisted wallet-inventory store.
fn edit_plan_step(env: &PlanEnv, plan_id: &str, step_id: &str, edit: impl FnOnce(&mut Value)) {
    let path = inventory_path(env);
    let mut store = read_store(&path);
    let plan = store["data"]["consolidation_plans"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|plan| plan["id"] == json!(plan_id))
        .expect("plan exists");
    let step = plan["steps"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|step| step["id"] == json!(step_id))
        .expect("step exists");
    edit(step);
    write_store(&path, &store);
}

/// Append a crafted step (cloned from an existing one) to the persisted plan.
fn add_plan_step(
    env: &PlanEnv,
    plan_id: &str,
    template_step_id: &str,
    edit: impl FnOnce(&mut Value),
) {
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
    edit(&mut crafted);
    steps.push(crafted);
    write_store(&path, &store);
}

fn edit_inventory(env: &PlanEnv, edit: impl FnOnce(&mut Value)) {
    let path = inventory_path(env);
    let mut store = read_store(&path);
    edit(&mut store["data"]);
    write_store(&path, &store);
}

fn set_queue_job_state(env: &PlanEnv, job_id: &str, state: &str) {
    let path = queue_path(env);
    let mut store = read_store(&path);
    let job = store["data"]["jobs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|job| job["id"] == json!(job_id))
        .expect("job exists");
    job["state"] = json!(state);
    write_store(&path, &store);
}

fn queue_job_state(env: &PlanEnv, job_id: &str) -> String {
    let store = read_store(&queue_path(env));
    store["data"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == json!(job_id))
        .expect("job exists")["state"]
        .as_str()
        .unwrap()
        .to_string()
}

// ── Happy path ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn enqueue_step_happy_path_persists_job_marker_evidence_hash_and_audit() {
    let (env, plan_id, step_id) = approved_plan_env().await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "enqueue response: {body}");
    assert_eq!(body["status"], json!("queued"));
    assert_eq!(body["plan_id"], json!(plan_id));
    assert_eq!(body["step_id"], json!(step_id));
    let job = &body["job"];
    let job_id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["kind"], json!("plan_step_execution"));
    assert_eq!(job["state"], json!("queued"));
    assert_eq!(job["action"], json!("sweep_native"));
    assert_eq!(job["source_address"], json!(SEED_ADDRESS));
    assert_eq!(job["derivation_path"], json!("m/44'/60'/0'/0/0"));
    assert_eq!(job["wallet_family"], json!("eth-seed"));
    assert_eq!(job["chain_id"], json!(1));
    assert_eq!(job["destination_address"], json!(DESTINATION));
    assert_eq!(job["call_target_address"], json!(DESTINATION));
    assert_eq!(job["call_data_hex"], json!("0x"));
    assert_eq!(job["call_value_wei_hex"], json!(ONE_ETH_HEX));
    // W6.2 fee basis captured from simulation evidence.
    assert_eq!(job["fee_basis"], json!("static_profile"));
    assert_eq!(job["max_priority_fee_per_gas_hex"], json!("0x59682f00"));
    assert_eq!(job["max_fee_per_gas_hex"], json!("0x12a05f200"));
    // Evidence hash present on the enqueued job (W7.3 consumes it).
    let hash = job["simulation_evidence_hash_hex"].as_str().unwrap();
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));

    // Job persisted in the queue store with the same evidence hash.
    let jobs = queue_jobs(&env).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["id"], json!(job_id.clone()));
    assert_eq!(jobs[0]["simulation_evidence_hash_hex"], json!(hash));
    // No secrets in the persisted job: session token never appears.
    let raw_queue = std::fs::read_to_string(queue_path(&env)).unwrap();
    assert!(!raw_queue.contains(&env.token));

    // Persistent idempotency marker set on the step.
    let step = plan_step(&env, &plan_id, &step_id).await;
    assert_eq!(step["queued_job_id"], json!(job_id.clone()));

    // Typed audit event with ids, family, and session fingerprint only.
    let audit = get(
        &env.client,
        env.addr,
        "/api/audit?limit=20&kind=wallet_consolidation.plan.enqueue_step",
        Some(&env.token),
    )
    .await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: Value = audit.json().await.unwrap();
    let events = audit_json["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "audit events: {audit_json}");
    let details = &events[0]["details"];
    assert_eq!(details["plan_id"], json!(plan_id));
    assert_eq!(details["step_id"], json!(step_id));
    assert_eq!(details["job_id"], json!(job_id));
    assert_eq!(details["action_family"], json!("sweep"));
    let fingerprint = details["session_fingerprint_hex"].as_str().unwrap();
    assert_eq!(fingerprint.len(), 16);
    assert!(!env.token.contains(fingerprint));

    env.shutdown();
}

// ── Validation negatives (one per named check) ─────────────────────────────

#[tokio::test]
async fn enqueue_step_refuses_unknown_plan() {
    let (env, _plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, "no-such-plan", &step_id, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("Consolidation plan not found."));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_unknown_step() {
    let (env, plan_id, _step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, "no-such-step", true).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("Consolidation plan step not found."));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_requires_explicit_confirm_flag() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, false).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        json!("confirm must be true to enqueue this step")
    );
    assert!(queue_jobs(&env).await.is_empty());
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_unapproved_step() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    // No approval.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().starts_with("not_approved:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_unsimulated_step() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
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
    let step_id = plan_json["plan"]["steps"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    approve_plan(&env, &plan_id).await;
    // Approved but never simulated.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_unsimulated"));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_stale_simulation_and_demands_resimulate() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    edit_plan_step(&env, &plan_id, &step_id, |step| {
        let evidence = step["simulation_evidence"].as_array_mut().unwrap();
        for item in evidence.iter_mut() {
            if item
                .as_str()
                .unwrap_or("")
                .starts_with("simulated_at_unix=")
            {
                *item = json!("simulated_at_unix=1");
            }
        }
    });
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let message = body["error"].as_str().unwrap();
    assert!(message.starts_with("simulation_stale:"), "error: {body}");
    assert!(message.contains("re-simulate"), "error: {body}");
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_blocked_step() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    edit_plan_step(&env, &plan_id, &step_id, |step| {
        step["blockers"] = json!(["manual_test_blocker"]);
    });
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let message = body["error"].as_str().unwrap();
    assert!(message.starts_with("step_blocked:"), "error: {body}");
    assert!(message.contains("manual_test_blocker"), "error: {body}");
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_non_executable_review_asset_action() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    edit_plan_step(&env, &plan_id, &step_id, |step| {
        step["action"] = json!("review_asset");
    });
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("action_not_executable:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_when_execution_paused() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let pause = post_json(
        &env.client,
        env.addr,
        "/api/queue/pause",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(pause.status(), StatusCode::OK);
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("execution_paused:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_when_master_gate_disabled() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["allow_plan_execution"] = json!(false);
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"],
        json!("execution_gate: allow_plan_execution is disabled")
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_when_family_gate_disabled() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["allow_sweep_execution"] = json!(false);
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"],
        json!("execution_gate: allow_sweep_execution is disabled")
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_destination_no_longer_allowlisted() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    // Policy flip between approval and enqueue: destination removed.
    let mut policy = gates_on_policy_body();
    policy["allowed_destinations"] = json!([{
        "address": "0x7777777777777777777777777777777777777777",
        "label": "other",
    }]);
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_destination"));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_step_value_above_cap() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["max_step_native_wei_hex"] = json!("0x1");
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_step_cap"));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_plan_value_above_plan_cap() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["max_plan_native_wei_hex"] = json!("0x1");
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_plan_cap"));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_cross_party_linkage() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    // Two counterparties' allocated addresses sweeping to one destination.
    let second_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_linked");
        step["sequence"] = json!(1);
        step["address"] = json!(second_address);
    });
    edit_inventory(&env, |data| {
        data["parties"] = json!([
            { "id": "party_a", "name": "Party A", "created_at_unix": 1 },
            { "id": "party_b", "name": "Party B", "created_at_unix": 1 },
        ]);
        data["receive_allocations"] = json!([
            {
                "id": "alloc_a",
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "chain_id": 1,
                "chain_id_assumed": false,
                "address": SEED_ADDRESS,
                "derivation_path": "m/44'/60'/0'/0/0",
                "address_index": 0,
                "purpose": "invoice",
                "status": "active",
                "created_at_unix": 1,
                "counterparty_id": "party_a",
            },
            {
                "id": "alloc_b",
                "wallet_family": "eth-seed",
                "wallet_profile": "seed-main",
                "chain_id": 1,
                "chain_id_assumed": false,
                "address": second_address,
                "derivation_path": "m/44'/60'/0'/0/1",
                "address_index": 1,
                "purpose": "invoice",
                "status": "active",
                "created_at_unix": 1,
                "counterparty_id": "party_b",
            },
        ]);
    });
    let mut policy = gates_on_policy_body();
    policy["block_cross_party_linkage"] = json!(true);
    update_policy(&env, policy).await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("cross_party_linkage:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_claim_step_without_reviewed_contract() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let now = now_unix();
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_claim");
        step["sequence"] = json!(1);
        step["action"] = json!("claim_reward");
        step["asset_kind"] = json!("reward");
        step["claim_adapter"] = json!("merkle-distributor-v1");
        step["protocol_address"] = json!("0x1111111111111111111111111111111111111111");
        step["claim_index_hex"] = json!("0x7");
        step["claim_proof"] = json!([format!("0x{}", "11".repeat(32))]);
        step["amount_hex"] = json!("0xf4240");
        step.as_object_mut().unwrap().remove("destination_address");
        step["simulation_status"] = json!("passed");
        step["simulation_evidence"] = json!([format!("simulated_at_unix={now}")]);
    });
    // allow_claim_execution is ON (family gate passes) but the claim contract
    // is not reviewed in the risk catalog: W5's claim gate must refuse.
    let (status, body) = enqueue_step(&env, &plan_id, "step_claim", true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("claim_execution_disabled:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_fund_gas_without_gas_topup_optin() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let now = now_unix();
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_fund_gas");
        step["sequence"] = json!(1);
        step["action"] = json!("fund_gas");
        step["amount_hex"] = json!("0x2f9b8");
        step["destination_address"] = json!(SEED_ADDRESS);
        step["simulation_status"] = json!("passed");
        step["simulation_evidence"] = json!([format!("simulated_at_unix={now}")]);
    });
    let mut policy = gates_on_policy_body();
    policy["allow_gas_topups"] = json!(false);
    update_policy(&env, policy).await;
    let (status, body) = enqueue_step(&env, &plan_id, "step_fund_gas", true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("allow_gas_topups"),
        "error: {body}"
    );
    env.shutdown();
}

// ── Idempotency ────────────────────────────────────────────────────────────

#[tokio::test]
async fn reenqueue_of_pending_step_refuses_already_enqueued() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "first enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(message.starts_with("already_enqueued:"), "error: {body}");
    assert!(message.contains(&job_id), "error: {body}");
    assert_eq!(queue_jobs(&env).await.len(), 1);
    env.shutdown();
}

#[tokio::test]
async fn reenqueue_of_succeeded_step_refuses_already_enqueued() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "first enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();
    set_queue_job_state(&env, &job_id, "sent");

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("already_enqueued:"),
        "error: {body}"
    );
    env.shutdown();
}

#[tokio::test]
async fn failed_step_requires_operator_reapproval_before_reenqueue() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "first enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();
    set_queue_job_state(&env, &job_id, "failed_terminal");

    // First re-enqueue attempt: refused with E1 semantics — the failed job
    // parks as operator_action_required and the step's approval is withdrawn.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("operator_action_required:"),
        "error: {body}"
    );
    assert_eq!(queue_job_state(&env, &job_id), "operator_action_required");
    let step = plan_step(&env, &plan_id, &step_id).await;
    assert_eq!(step["approved"], json!(false));
    assert_eq!(step["status"], json!("review_required"));

    // Still refused without re-approval.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("operator_action_required:"),
        "error: {body}"
    );

    // Operator inspects and re-approves through the normal approval pipeline.
    let approved = approve_plan(&env, &plan_id).await;
    assert_eq!(
        approved["plan"]["steps"][0]["approved"],
        json!(true),
        "approve response: {approved}"
    );

    // Re-enqueue now succeeds with a fresh job; the marker moves on and the
    // parked job stays as the inspection record.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "re-enqueue: {body}");
    let new_job_id = body["job"]["id"].as_str().unwrap().to_string();
    assert_ne!(new_job_id, job_id);
    assert_eq!(queue_job_state(&env, &job_id), "operator_action_required");
    let step = plan_step(&env, &plan_id, &step_id).await;
    assert_eq!(step["queued_job_id"], json!(new_job_id));
    env.shutdown();
}

// ── Dependency chaining (W6.4) ─────────────────────────────────────────────

#[tokio::test]
async fn enqueue_step_refuses_dependency_not_enqueued_then_chains_in_order() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let dep_id = step_id.clone();
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_dependent");
        step["sequence"] = json!(1);
        step["depends_on"] = json!([dep_id]);
    });

    // Dependent first: refusal names the missing prerequisite.
    let (status, body) = enqueue_step(&env, &plan_id, "step_dependent", true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with(&format!("dependency_not_enqueued:{step_id}")),
        "error: {body}"
    );
    assert!(queue_jobs(&env).await.is_empty());

    // Enqueue the prerequisite, then the dependent chains onto its job id.
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "prerequisite enqueue: {body}");
    let dep_job_id = body["job"]["id"].as_str().unwrap().to_string();

    let (status, body) = enqueue_step(&env, &plan_id, "step_dependent", true).await;
    assert_eq!(status, StatusCode::OK, "dependent enqueue: {body}");
    assert_eq!(body["job"]["prerequisite_job_ids"], json!([dep_job_id]));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_step_refuses_missing_dependency() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    edit_plan_step(&env, &plan_id, &step_id, |step| {
        step["depends_on"] = json!(["ghost-step"]);
    });
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("dependency_missing:ghost-step"),
        "error: {body}"
    );
    env.shutdown();
}

// ── Bulk enqueue: typed confirmation + ordering ────────────────────────────

#[tokio::test]
async fn enqueue_plan_refuses_wrong_confirmation_and_reports_expected_phrase() {
    let (env, plan_id, _step_id) = approved_plan_env().await;
    let expected = format!("EXECUTE 1 PLAN STEPS TOTAL {ONE_ETH_DECIMAL} WEI");

    let (status, body) = enqueue_plan(&env, &plan_id, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    let message = body["error"].as_str().unwrap();
    assert!(
        message.starts_with("confirmation_mismatch:"),
        "error: {body}"
    );
    assert!(message.contains(&expected), "error: {body}");
    // Machine-readable expected phrase for the UI dialog / CLI.
    assert_eq!(body["action"], json!(expected.clone()));

    let (status, body) = enqueue_plan(&env, &plan_id, "EXECUTE 9 PLAN STEPS TOTAL 1 WEI").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    assert_eq!(body["action"], json!(expected));

    // Nothing was enqueued by either mismatch.
    assert!(queue_jobs(&env).await.is_empty());
    env.shutdown();
}

#[tokio::test]
async fn enqueue_plan_with_typed_confirmation_enqueues_in_order_with_skips() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let dep_id = step_id.clone();
    // step_b: eligible, depends on the real step, moves 1 ETH too.
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_b");
        step["sequence"] = json!(1);
        step["depends_on"] = json!([dep_id]);
    });
    // step_c: not approved => skipped with a named reason.
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_c");
        step["sequence"] = json!(2);
        step["approved"] = json!(false);
        step["status"] = json!("review_required");
    });
    // step_d: depends on the skipped step_c => skip propagates by name.
    add_plan_step(&env, &plan_id, &step_id, |step| {
        step["id"] = json!("step_d");
        step["sequence"] = json!(3);
        step["depends_on"] = json!(["step_c"]);
    });

    // The phrase counts ONLY the steps that will actually enqueue (2) and
    // their total native value (2 ETH).
    let expected = "EXECUTE 2 PLAN STEPS TOTAL 2000000000000000000 WEI";
    let (status, body) = enqueue_plan(&env, &plan_id, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "probe: {body}");
    assert_eq!(body["action"], json!(expected));

    let (status, body) = enqueue_plan(&env, &plan_id, expected).await;
    assert_eq!(status, StatusCode::OK, "bulk enqueue: {body}");
    assert_eq!(body["status"], json!("queued"));
    let enqueued = body["enqueued"].as_array().unwrap();
    assert_eq!(enqueued.len(), 2, "enqueued: {body}");
    assert_eq!(enqueued[0]["step_id"], json!(step_id.clone()));
    assert_eq!(enqueued[1]["step_id"], json!("step_b"));
    let job_a = enqueued[0]["job_id"].as_str().unwrap().to_string();
    let job_b = enqueued[1]["job_id"].as_str().unwrap().to_string();
    let skipped = body["skipped"].as_array().unwrap();
    assert!(
        skipped
            .iter()
            .any(|s| s["step_id"] == json!("step_c") && s["reason"] == json!("not_approved")),
        "skipped: {body}"
    );
    assert!(
        skipped.iter().any(|s| s["step_id"] == json!("step_d")
            && s["reason"] == json!("dependency_skipped:step_c")),
        "skipped: {body}"
    );

    // Jobs land in sequence order; the dependent carries its prerequisite.
    let jobs = queue_jobs(&env).await;
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["id"], json!(job_a.clone()));
    assert_eq!(jobs[1]["id"], json!(job_b.clone()));
    assert_eq!(jobs[1]["prerequisite_job_ids"], json!([job_a.clone()]));

    // Markers persisted for both enqueued steps.
    let step_a = plan_step(&env, &plan_id, &step_id).await;
    assert_eq!(step_a["queued_job_id"], json!(job_a));
    let step_b = plan_step(&env, &plan_id, "step_b").await;
    assert_eq!(step_b["queued_job_id"], json!(job_b));
    env.shutdown();
}

#[tokio::test]
async fn enqueue_plan_refuses_when_gates_off_at_policy_check() {
    let env = setup_plan_env().await;
    // Policy on only long enough to approve; then everything off (defaults).
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, _step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let mut policy = gates_on_policy_body();
    policy["allow_plan_execution"] = json!(false);
    update_policy(&env, policy).await;

    let (status, body) = enqueue_plan(&env, &plan_id, "anything").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "response: {body}");
    let message = body["error"].as_str().unwrap();
    assert!(message.starts_with("no_steps_eligible:"), "error: {body}");
    assert!(
        message.contains("execution_gate: allow_plan_execution is disabled"),
        "error: {body}"
    );
    assert!(queue_jobs(&env).await.is_empty());
    env.shutdown();
}

// ── Drain-time hard block (CRITICAL: no execution yet) ─────────────────────

/// W7.3: the drain-time hard block ("plan-step execution is not enabled
/// yet") lifts once every W7.1 gate passes — the job signs and broadcasts
/// against the mock RPC provider instead of staying blocked (superseding
/// the pre-W7.3 behavior this test used to assert).
#[tokio::test]
async fn plan_step_jobs_execute_at_drain_once_all_gates_pass() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();

    // Every W7.1 gate is ON: the job signs and broadcasts.
    let process = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(
        process_json["processed"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(process_json["blocked"], json!(0), "process: {process_json}");
    assert_eq!(
        process_json["operator_action_required"],
        json!(0),
        "process: {process_json}"
    );

    let jobs = queue_jobs(&env).await;
    assert_eq!(jobs[0]["id"], json!(job_id));
    assert_eq!(jobs[0]["state"], json!("sent"));
    assert!(jobs[0]["last_error"].is_null(), "job: {}", jobs[0]);
    assert!(
        jobs[0]["transaction_hash_hex"].as_str().unwrap().len() == 64,
        "job: {}",
        jobs[0]
    );
    assert!(
        jobs[0]["broadcast_transaction_hash_hex"]
            .as_str()
            .unwrap()
            .starts_with("11"),
        "job: {}",
        jobs[0]
    );

    // The typed sign -> broadcast audit chain is recorded, with no key
    // material anywhere in it.
    let audit = get(&env.client, env.addr, "/api/audit", Some(&env.token)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: Value = audit.json().await.unwrap();
    let events = audit_json["events"].as_array().unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"wallet_consolidation.plan.step_sign"),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&"wallet_consolidation.plan.step_broadcast"),
        "{kinds:?}"
    );
    let audit_text = audit_json.to_string();
    assert!(
        !audit_text.to_lowercase().contains("abandon abandon"),
        "audit log must never contain mnemonic material"
    );
    env.shutdown();
}

#[tokio::test]
async fn policy_flip_between_enqueue_and_drain_blocks_with_gate_reason() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");

    // Kill switch flipped after enqueue: drain must not even reach the
    // W7.2 hard block; the job blocks with the pause reason (W7.1/E1).
    let pause = post_json(
        &env.client,
        env.addr,
        "/api/queue/pause",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(pause.status(), StatusCode::OK);
    let process = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(
        process_json["processed"],
        json!(0),
        "process: {process_json}"
    );
    assert!(
        process_json["paused_reason"]
            .as_str()
            .unwrap()
            .starts_with("execution_paused:"),
        "process: {process_json}"
    );

    // Resume but drop the family gate: drain blocks with the gate reason.
    let resume = post_json(
        &env.client,
        env.addr,
        "/api/queue/resume",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(resume.status(), StatusCode::OK);
    let mut policy = gates_on_policy_body();
    policy["allow_sweep_execution"] = json!(false);
    update_policy(&env, policy).await;
    let process = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(process_json["blocked"], json!(1), "process: {process_json}");
    let jobs = queue_jobs(&env).await;
    assert_eq!(
        jobs[0]["last_error"],
        json!("execution_gate: allow_sweep_execution is disabled")
    );
    env.shutdown();
}

// ── Gates off: the new routes refuse; nothing else changes ─────────────────

#[tokio::test]
async fn gates_off_enqueue_routes_refuse_at_policy_check_and_queue_untouched() {
    let env = setup_plan_env().await;
    // Approve with a policy, then remove every gate (the default state).
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    update_policy(
        &env,
        json!({
            "enabled": true,
            "allowed_destinations": [{ "address": DESTINATION, "label": "test-treasury" }],
        }),
    )
    .await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"],
        json!("execution_gate: allow_plan_execution is disabled")
    );
    assert!(queue_jobs(&env).await.is_empty());

    // Existing queue processing on an empty queue is untouched.
    let process = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(process_json["processed"], json!(0));
    env.shutdown();
}
