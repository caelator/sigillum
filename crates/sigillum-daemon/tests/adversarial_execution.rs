//! F1 adversarial suite for the execution path: transport abuse, stale/foreign
//! ids, enqueue replay, and approve-vs-enqueue policy TOCTOU.

mod common;

use common::{get, post_json, spawn_daemon, submitted_raw_transaction_hash};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

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
            "eth_sendRawTransaction" => submitted_raw_transaction_hash(request),
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

fn read_store(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn write_store(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn inventory_path(env: &PlanEnv) -> PathBuf {
    env.base_dir.join("wallet_inventory.json")
}

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

fn assert_rejected(status: StatusCode, body: &Value) {
    assert!(
        status.is_client_error(),
        "expected client error, got {status}: {body}"
    );
    assert_ne!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "adversarial input must not produce 500: {body}"
    );
}

async fn raw_post(
    env: &PlanEnv,
    path: &str,
    body: impl Into<reqwest::Body>,
    content_type: &str,
) -> (StatusCode, Value) {
    let response = env
        .client
        .post(format!("http://{}{}", env.addr, path))
        .bearer_auth(&env.token)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    let body = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
    (status, body)
}

// ── Authentication fail-closed ────────────────────────────────────────────

#[tokio::test]
async fn auth_fail_closed() {
    let (env, plan_id, step_id) = approved_plan_env().await;

    for token in [None, Some("%%%")] {
        let step = post_json(
            &env.client,
            env.addr,
            "/api/plans/enqueue-step",
            json!({ "plan_id": plan_id, "step_id": step_id, "confirm": true }),
            token,
        )
        .await;
        let status = step.status();
        let body: Value = step.json().await.unwrap();
        assert_eq!(status, StatusCode::UNAUTHORIZED, "response: {body}");
        assert!(queue_jobs(&env).await.is_empty());

        let plan = post_json(
            &env.client,
            env.addr,
            "/api/plans/enqueue-plan",
            json!({ "plan_id": plan_id, "confirmation": "anything" }),
            token,
        )
        .await;
        let status = plan.status();
        let body: Value = plan.json().await.unwrap();
        assert_eq!(status, StatusCode::UNAUTHORIZED, "response: {body}");
        assert!(queue_jobs(&env).await.is_empty());
    }

    env.shutdown();
}

// ── Transport abuse ───────────────────────────────────────────────────────

#[tokio::test]
async fn transport_abuse() {
    let (env, plan_id, step_id) = approved_plan_env().await;

    let (status, body) = raw_post(
        &env,
        "/api/plans/enqueue-step",
        r#"{"unterminated": "#,
        "application/json",
    )
    .await;
    assert_rejected(status, &body);
    assert!(queue_jobs(&env).await.is_empty());

    let valid_body =
        serde_json::to_vec(&json!({ "plan_id": plan_id, "step_id": step_id, "confirm": true }))
            .unwrap();
    let (status, body) = raw_post(&env, "/api/plans/enqueue-step", valid_body, "text/plain").await;
    assert_rejected(status, &body);
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

// ── Stale and foreign ids ─────────────────────────────────────────────────

#[tokio::test]
async fn stale_ids() {
    let (env, plan_id, step_id) = approved_plan_env().await;

    let (status, body) = enqueue_step(&env, "plan-does-not-exist", &step_id, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "response: {body}");
    assert_eq!(body["error"], json!("Consolidation plan not found."));
    assert!(queue_jobs(&env).await.is_empty());

    let (status, body) = enqueue_step(&env, &plan_id, "step-does-not-exist", true).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "response: {body}");
    assert_eq!(body["error"], json!("Consolidation plan step not found."));
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

#[tokio::test]
async fn foreign_compartment_ids() {
    let (env_a, plan_a, _step_a) = approved_plan_env().await;
    let (env_b, plan_b, step_b) = approved_plan_env().await;

    let (status, body) = enqueue_step(&env_a, &plan_b, &step_b, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "response: {body}");
    assert!(queue_jobs(&env_a).await.is_empty());

    let (status, body) = enqueue_step(&env_a, &plan_a, &step_b, true).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "response: {body}");
    assert!(queue_jobs(&env_a).await.is_empty());

    env_a.shutdown();
    env_b.shutdown();
}

// ── Enqueue replay ────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_of_successful_enqueue_step() {
    let (env, plan_id, step_id) = approved_plan_env().await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::OK, "first enqueue: {body}");
    assert_eq!(body["status"], json!("queued"));
    assert_eq!(queue_jobs(&env).await.len(), 1);

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_rejected(status, &body);
    assert_eq!(status, StatusCode::CONFLICT, "replay response: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("already_enqueued:"),
        "error: {body}"
    );
    assert_eq!(queue_jobs(&env).await.len(), 1);

    env.shutdown();
}

#[tokio::test]
async fn replay_of_successful_enqueue_plan() {
    let (env, plan_id, _step_id) = approved_plan_env().await;

    let (status, body) = enqueue_plan(&env, &plan_id, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "probe response: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("confirmation_mismatch:"),
        "error: {body}"
    );
    let phrase = body["action"].as_str().unwrap().to_string();
    assert!(queue_jobs(&env).await.is_empty());

    let (status, body) = enqueue_plan(&env, &plan_id, &phrase).await;
    assert_eq!(status, StatusCode::OK, "bulk enqueue: {body}");
    assert_eq!(body["status"], json!("queued"));
    let queued_len = queue_jobs(&env).await.len();
    assert!(queued_len >= 1, "queue length after enqueue: {queued_len}");

    let (status, body) = enqueue_plan(&env, &plan_id, &phrase).await;
    assert_rejected(status, &body);
    let message = body["error"].as_str().unwrap();
    assert!(
        message.starts_with("no_steps_eligible:") || message.starts_with("confirmation_mismatch:"),
        "error: {body}"
    );
    assert_eq!(queue_jobs(&env).await.len(), queued_len);

    env.shutdown();
}

// ── Approve-vs-enqueue TOCTOU ─────────────────────────────────────────────

#[tokio::test]
async fn toctou_gate_flip_after_approval() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["allow_plan_execution"] = json!(false);
    update_policy(&env, policy).await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "response: {body}");
    assert_eq!(
        body["error"],
        json!("execution_gate: allow_plan_execution is disabled")
    );
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

#[tokio::test]
async fn toctou_destination_flip_after_approval() {
    let (env, plan_id, step_id) = approved_plan_env().await;
    let mut policy = gates_on_policy_body();
    policy["allowed_destinations"] = json!([
        { "address": "0x7777777777777777777777777777777777777777", "label": "different" }
    ]);
    update_policy(&env, policy).await;

    let (status, body) = enqueue_step(&env, &plan_id, &step_id, true).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "response: {body}");
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_destination"));
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

#[tokio::test]
async fn toctou_linkage_flip_after_approval() {
    let (env, plan_id, step_id) = approved_plan_env().await;
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
    assert_eq!(status, StatusCode::FORBIDDEN, "response: {body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("cross_party_linkage:"),
        "error: {body}"
    );
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}
