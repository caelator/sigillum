//! W7.3 integration tests: seed-wallet signing execution. Mock-RPC only;
//! every daemon runs in a fresh TempDir on an ephemeral port. Helpers mirror
//! (duplicated, not shared — house style) the patterns in `plan_enqueue.rs`:
//! its `setup_plan_env`/`generate_and_simulate_plan`/`approve_plan`/direct
//! store-surgery helpers build enqueued jobs end-to-end through the real
//! routes; this file adds execution (drain) on top.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
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

#[derive(Clone, Default)]
struct RpcState {
    fail_broadcast: Arc<AtomicBool>,
    broadcast_count: Arc<AtomicUsize>,
    nonce_count: Arc<AtomicUsize>,
}

fn rpc_response(state: &RpcState, request: &Value) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!("0x20"),
        "eth_getTransactionCount" => {
            state.nonce_count.fetch_add(1, Ordering::SeqCst);
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
            if state.fail_broadcast.load(Ordering::SeqCst) {
                return json!({
                    "jsonrpc": "2.0",
                    "id": request.get("id").cloned().unwrap_or(json!(1)),
                    "error": { "code": -32000, "message": "execution reverted" },
                });
            }
            state.broadcast_count.fetch_add(1, Ordering::SeqCst);
            submitted_raw_transaction_hash(request)
        }
        "eth_getTransactionReceipt" => {
            let transaction_hash = request["params"][0]
                .as_str()
                .expect("receipt lookup carries the transaction hash");
            json!({
                "status": "0x1",
                "blockNumber": "0x10",
                "gasUsed": "0x5208",
                "transactionHash": transaction_hash,
            })
        }
        other => json!({ "unsupported": other }),
    };
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(json!(1)),
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
        "max_gas_topup_wei_hex": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
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

async fn audit_events(env: &PlanEnv) -> Vec<Value> {
    let audit = get(
        &env.client,
        env.addr,
        "/api/audit?limit=200",
        Some(&env.token),
    )
    .await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: Value = audit.json().await.unwrap();
    audit_json["events"].as_array().unwrap().clone()
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

fn raw_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn edit_inventory(env: &PlanEnv, edit: impl FnOnce(&mut Value)) {
    let path = inventory_path(env);
    let mut store = read_store(&path);
    edit(&mut store["data"]);
    write_store(&path, &store);
}

/// Mark a claim contract as reviewed in the risk catalog (W5's claim gate).
fn mark_claim_contract_reviewed(env: &PlanEnv, address: &str) {
    edit_inventory(env, |data| {
        let entries = data["risk_catalog"].as_array_mut().unwrap();
        entries.push(json!({
            "address": address,
            "label": "test-reviewed-claim-contract",
            "risk_level": "trusted",
            "source": "operator",
            "notes": [],
            "created_at_unix": 1,
            "updated_at_unix": 1,
        }));
    });
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

fn queue_job_by_step(env: &PlanEnv, step_id: &str) -> Value {
    let store = read_store(&queue_path(env));
    store["data"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .cloned()
        .unwrap_or_else(|| panic!("no queue job for step {step_id}"))
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

/// Insert a raw `EthSeed*`-family job directly (no enqueue route exists for
/// this legacy family — W7.2/W7.3 only expose `PlanStepExecution` enqueue).
fn insert_eth_seed_job(env: &PlanEnv, payload: Value) -> String {
    let path = queue_path(env);
    // The queue store is created lazily on first save; a fresh env may not
    // have touched any queue route yet.
    let mut store = if path.exists() {
        read_store(&path)
    } else {
        json!({ "schema": "sigillum.queue", "schema_version": 3, "data": { "jobs": [] } })
    };
    let job_id = "eth-seed-job-1".to_string();
    let job = json!({
        "id": job_id,
        "state": "queued",
        "attempts": 0,
        "created_at_unix": 1,
        "updated_at_unix": 1,
        "last_error": null,
        "transaction_hash_hex": null,
        "broadcast_transaction_hash_hex": null,
    });
    let mut job = job;
    for (key, value) in payload.as_object().unwrap() {
        job[key] = value.clone();
    }
    store["data"]["jobs"].as_array_mut().unwrap().push(job);
    write_store(&path, &store);
    job_id
}

// ── Full-flow: dependency-ordered multi-step execution with audit trail ────

#[tokio::test]
async fn dependency_chain_executes_in_order_with_full_audit_trail() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, sweep_step_id) = generate_and_simulate_plan(&env).await;

    // Build the production-shaped dependency chain: fund_gas -> sweep -> revoke.
    let fund_gas_step_id = add_plan_step(&env, &plan_id, &sweep_step_id, |step| {
        step["action"] = json!("fund_gas");
        step["asset_kind"] = json!("native");
        step["asset_address"] = json!(null);
        step["destination_address"] = json!(SEED_ADDRESS);
        // Zero keeps the batch's typed-confirmation total equal to the sweep
        // amount without hand-computing a u256 sum.
        step["amount_hex"] = json!("0x0");
        step["depends_on"] = json!([]);
    });
    edit_step_depends_on(
        &env,
        &plan_id,
        &sweep_step_id,
        vec![fund_gas_step_id.clone()],
    );

    let revoke_step_id = add_plan_step(&env, &plan_id, &sweep_step_id, |step| {
        step["action"] = json!("revoke_erc20_approval");
        step["asset_kind"] = json!("approval");
        step["asset_address"] = json!("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        step["counterparty_address"] = json!("0x3333333333333333333333333333333333333333");
        step["destination_address"] = json!(null);
    });
    // `add_plan_step` clones the template (sweep) step, so its `depends_on`
    // must be set as a follow-up edit once the new step's real id is known.
    edit_step_depends_on(&env, &plan_id, &revoke_step_id, vec![sweep_step_id.clone()]);

    let approved = approve_plan(&env, &plan_id).await;
    let steps = approved["plan"]["steps"].as_array().unwrap();
    assert!(
        steps.iter().all(|step| step["approved"] == json!(true)),
        "all three steps must approve: {steps:?}"
    );

    // Bulk enqueue in dependency order (W6.4/W7.2 already orders this). Fetch
    // the exact expected typed-confirmation phrase from a deliberate
    // mismatch rather than hand-computing the batch's u256 total.
    let (mismatch_status, mismatch_body) = enqueue_plan(&env, &plan_id, "").await;
    assert_eq!(mismatch_status, StatusCode::BAD_REQUEST, "{mismatch_body}");
    let confirmation = mismatch_body["action"].as_str().unwrap().to_string();
    assert!(
        confirmation.starts_with("EXECUTE 3 PLAN STEPS TOTAL "),
        "{confirmation}"
    );

    let (status, body) = enqueue_plan(&env, &plan_id, &confirmation).await;
    assert_eq!(status, StatusCode::OK, "enqueue-plan: {body}");
    let enqueued = body["enqueued"].as_array().unwrap();
    assert_eq!(enqueued.len(), 3, "enqueue-plan: {body}");

    let fund_gas_job = queue_job_by_step(&env, &fund_gas_step_id);
    let sweep_job = queue_job_by_step(&env, &sweep_step_id);
    let revoke_job = queue_job_by_step(&env, &revoke_step_id);
    assert_eq!(
        sweep_job["prerequisite_job_ids"],
        json!([fund_gas_job["id"]])
    );
    assert_eq!(revoke_job["prerequisite_job_ids"], json!([sweep_job["id"]]));

    // A fresh prerequisite only reaches `sent` in its broadcast cycle. Each
    // dependent must remain unsigned until a later cycle observes the
    // prerequisite's successful receipt and moves it to `confirmed`.
    let first_process = process_queue(&env).await;
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        1,
        "first process: {first_process}"
    );
    assert_eq!(
        queue_job_by_step(&env, &fund_gas_step_id)["state"],
        json!("sent")
    );
    let sweep_job = queue_job_by_step(&env, &sweep_step_id);
    assert_eq!(sweep_job["state"], json!("blocked"), "{sweep_job}");
    assert_eq!(
        sweep_job["transaction_hash_hex"],
        json!(null),
        "dependent must remain unsigned: {sweep_job}"
    );

    let second_process = process_queue(&env).await;
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        2,
        "second process: {second_process}"
    );
    assert_eq!(
        queue_job_by_step(&env, &fund_gas_step_id)["state"],
        json!("confirmed")
    );
    assert_eq!(
        queue_job_by_step(&env, &sweep_step_id)["state"],
        json!("sent")
    );
    let revoke_job = queue_job_by_step(&env, &revoke_step_id);
    assert_eq!(revoke_job["state"], json!("blocked"), "{revoke_job}");
    assert_eq!(revoke_job["transaction_hash_hex"], json!(null));

    let third_process = process_queue(&env).await;
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        3,
        "third process: {third_process}"
    );
    assert_eq!(
        queue_job_by_step(&env, &sweep_step_id)["state"],
        json!("confirmed")
    );
    assert_eq!(
        queue_job_by_step(&env, &revoke_step_id)["state"],
        json!("sent")
    );

    let fourth_process = process_queue(&env).await;
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        3,
        "confirmation must not rebroadcast: {fourth_process}"
    );
    for step_id in [&fund_gas_step_id, &sweep_step_id, &revoke_step_id] {
        let job = queue_job_by_step(&env, step_id);
        assert_eq!(job["state"], json!("confirmed"), "job for {step_id}: {job}");
        assert_eq!(job["receipt_status"], json!("success"), "{job}");
        assert!(job["transaction_hash_hex"].is_string(), "job: {job}");
    }

    // Audit trail: sign -> broadcast for every one of the three steps, no
    // key material anywhere.
    let events = audit_events(&env).await;
    let sign_events: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == json!("wallet_consolidation.plan.step_sign"))
        .collect();
    let broadcast_events: Vec<&Value> = events
        .iter()
        .filter(|event| event["kind"] == json!("wallet_consolidation.plan.step_broadcast"))
        .collect();
    assert_eq!(sign_events.len(), 3, "{events:#?}");
    assert_eq!(broadcast_events.len(), 3, "{events:#?}");
    let signed_step_ids: Vec<&str> = sign_events
        .iter()
        .map(|event| event["details"]["step_id"].as_str().unwrap())
        .collect();
    assert!(signed_step_ids.contains(&sweep_step_id.as_str()));
    assert!(signed_step_ids.contains(&revoke_step_id.as_str()));
    assert!(signed_step_ids.contains(&fund_gas_step_id.as_str()));

    env.shutdown();
}

#[tokio::test]
async fn failed_gas_topup_permanently_blocks_dependent_sweep_before_signing() {
    let env = setup_plan_env().await;
    env.rpc_state.fail_broadcast.store(true, Ordering::SeqCst);
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, sweep_step_id) = generate_and_simulate_plan(&env).await;

    let fund_gas_step_id = add_plan_step(&env, &plan_id, &sweep_step_id, |step| {
        step["action"] = json!("fund_gas");
        step["asset_kind"] = json!("native");
        step["asset_address"] = json!(null);
        step["destination_address"] = json!(SEED_ADDRESS);
        step["amount_hex"] = json!("0x0");
        step["depends_on"] = json!([]);
    });
    edit_step_depends_on(
        &env,
        &plan_id,
        &sweep_step_id,
        vec![fund_gas_step_id.clone()],
    );

    approve_plan(&env, &plan_id).await;
    let (mismatch_status, mismatch_body) = enqueue_plan(&env, &plan_id, "").await;
    assert_eq!(mismatch_status, StatusCode::BAD_REQUEST, "{mismatch_body}");
    let confirmation = mismatch_body["action"].as_str().unwrap();
    let (status, body) = enqueue_plan(&env, &plan_id, confirmation).await;
    assert_eq!(status, StatusCode::OK, "enqueue-plan: {body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        0,
        "provider rejected the top-up and the sweep must never broadcast"
    );

    let fund_job = queue_job_by_step(&env, &fund_gas_step_id);
    assert_eq!(
        fund_job["state"],
        json!("operator_action_required"),
        "{fund_job}"
    );
    let sweep_job = queue_job_by_step(&env, &sweep_step_id);
    assert_eq!(sweep_job["state"], json!("blocked"), "{sweep_job}");
    assert!(
        sweep_job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("dependency_failed:"),
        "{sweep_job}"
    );
    assert_eq!(sweep_job["transaction_hash_hex"], json!(null));

    let second_process = process_queue(&env).await;
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        0,
        "blocked dependent must remain unsigned: {second_process}"
    );
    assert_eq!(
        queue_job_by_step(&env, &sweep_step_id)["transaction_hash_hex"],
        json!(null)
    );

    env.shutdown();
}

fn edit_step_depends_on(env: &PlanEnv, plan_id: &str, step_id: &str, depends_on: Vec<String>) {
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
    step["depends_on"] = json!(depends_on);
    write_store(&path, &store);
}

// ── F5 adversarial execution-path regressions ───────────────────────────

#[tokio::test]
async fn hostile_inventory_tampered_destination_refused_at_enqueue() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;

    edit_inventory(&env, |data| {
        let step = data["consolidation_plans"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .flat_map(|plan| plan["steps"].as_array_mut().unwrap().iter_mut())
            .find(|step| step["id"] == json!(step_id))
            .expect("approved step exists");
        step["destination_address"] = json!("0x8888888888888888888888888888888888888888");
    });

    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], json!("policy_violation"));
    assert_eq!(body["action"], json!("block_destination"));
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

#[tokio::test]
async fn hostile_inventory_garbage_evidence_refused_at_enqueue() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;

    edit_inventory(&env, |data| {
        let step = data["consolidation_plans"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .flat_map(|plan| plan["steps"].as_array_mut().unwrap().iter_mut())
            .find(|step| step["id"] == json!(step_id))
            .expect("approved step exists");
        step["simulation_evidence"] = json!([]);
        step["simulation_status"] = json!("passed");
    });

    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert!(
        !status.is_success(),
        "missing evidence must be refused: {body}"
    );
    let refusal_text = format!("{} {}", body["error"], body["message"]);
    assert!(
        refusal_text.contains("simulation_stale"),
        "refusal should mention simulation_stale: {body}"
    );
    assert!(queue_jobs(&env).await.is_empty());

    env.shutdown();
}

#[tokio::test]
async fn corrupt_inventory_is_quarantined_and_good_state_restored() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");

    let policy = get(
        &env.client,
        env.addr,
        "/api/treasury/policy",
        Some(&env.token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["enabled"], json!(true));

    std::fs::write(inventory_path(&env), b"{ not valid json ]").unwrap();

    let policy = get(
        &env.client,
        env.addr,
        "/api/treasury/policy",
        Some(&env.token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["enabled"], json!(true));

    let has_quarantine = std::fs::read_dir(&env.base_dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".corrupt-")
    });
    assert!(has_quarantine, "corrupt inventory should be quarantined");

    let live_bytes = std::fs::read(inventory_path(&env)).unwrap();
    serde_json::from_slice::<Value>(&live_bytes).unwrap();

    env.shutdown();
}

#[tokio::test]
async fn policy_update_and_drain_serialize_no_torn_state_or_double_broadcast() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");

    let drain = post_json(
        &env.client,
        env.addr,
        "/api/queue/process",
        json!({}),
        Some(&env.token),
    );
    let pause = post_json(
        &env.client,
        env.addr,
        "/api/queue/pause",
        json!({}),
        Some(&env.token),
    );
    let (drain_res, pause_res) = tokio::join!(drain, pause);
    assert_eq!(drain_res.status(), StatusCode::OK);
    assert_eq!(pause_res.status(), StatusCode::OK);

    let broadcasts = env.rpc_state.broadcast_count.load(Ordering::SeqCst);
    assert!(broadcasts <= 1, "no double broadcast: {broadcasts}");

    let jobs = queue_jobs(&env).await;
    let job = jobs
        .iter()
        .find(|job| job["step_id"] == json!(step_id))
        .expect("queued job exists");
    assert!(
        job.get("signed_raw_transaction_hex").is_none(),
        "queue APIs redact replayable signed bytes: {job}"
    );
    if broadcasts == 1 {
        assert_eq!(job["state"], json!("sent"), "{job}");
        assert!(job["transaction_hash_hex"].is_string(), "{job}");
    } else {
        assert_ne!(job["state"], json!("sent"), "{job}");
        if job["state"] == json!("prepared") {
            assert!(job["transaction_hash_hex"].is_string(), "{job}");
            let persisted_job = queue_job_by_step(&env, &step_id);
            assert!(
                persisted_job["signed_raw_transaction_hex"].is_string(),
                "durable queue storage retains exact replay bytes: {persisted_job}"
            );
        } else {
            assert_eq!(job["state"], json!("queued"), "{job}");
            assert!(job["transaction_hash_hex"].is_null(), "{job}");
        }
    }

    let queue_bytes = std::fs::read(queue_path(&env)).unwrap();
    serde_json::from_slice::<Value>(&queue_bytes).unwrap();

    env.shutdown();
}

#[tokio::test]
async fn enqueue_sign_broadcast_events_carry_matching_session_fingerprint() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );

    let events = audit_events(&env).await;
    let mut fingerprints = Vec::new();
    for kind in [
        "wallet_consolidation.plan.enqueue_step",
        "wallet_consolidation.plan.step_sign",
        "wallet_consolidation.plan.step_broadcast",
    ] {
        let event = events
            .iter()
            .find(|event| event["kind"] == json!(kind))
            .unwrap_or_else(|| panic!("missing audit event {kind}: {events:#?}"));
        let fp = event["details"]["session_fingerprint_hex"]
            .as_str()
            .unwrap_or_else(|| panic!("missing session fingerprint: {event:#?}"));
        // 8-byte (16 hex char) truncated SHA-256 of the session token, used for
        // audit attribution without storing the token.
        assert_eq!(fp.len(), 16, "fingerprint length for {kind}: {event}");
        assert!(
            fp.chars().all(|ch| ch.is_ascii_hexdigit()),
            "fingerprint must be hex for {kind}: {event}"
        );
        fingerprints.push(fp.to_string());
    }
    assert!(
        fingerprints.windows(2).all(|pair| pair[0] == pair[1]),
        "same session identity across enqueue/sign/broadcast: {fingerprints:?}"
    );

    env.shutdown();
}

// ── Evidence-hash tamper detection ──────────────────────────────────────

#[tokio::test]
async fn evidence_hash_tamper_blocks_execution_as_operator_action_required() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();

    // Simulate tampering with the persisted job's prepared call parameters
    // between enqueue and drain (e.g. a corrupted or maliciously edited
    // queue store).
    edit_queue_job(&env, &job_id, |job| {
        job["call_target_address"] = json!("0x8888888888888888888888888888888888888888");
    });

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(
        process_json["succeeded"],
        json!(0),
        "process: {process_json}"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(job["state"], json!("operator_action_required"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("evidence_hash_tamper:"),
        "{job}"
    );
    assert!(job["transaction_hash_hex"].is_null(), "never signed: {job}");

    env.shutdown();
}

// ── Watch-only re-check (defense in depth) ──────────────────────────────

#[tokio::test]
async fn watch_only_wallet_family_is_reblocked_at_execution_time() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();

    // The `wallet_family` field is not part of the evidence-hash commitment
    // (it is sourced from the live step during re-verification), so
    // corrupting only the persisted job's copy exercises the DEDICATED
    // watch-only re-check defense-in-depth path rather than the hash check.
    edit_queue_job(&env, &job_id, |job| {
        job["wallet_family"] = json!("eth-xpub");
    });

    let process_json = process_queue(&env).await;
    assert_eq!(process_json["blocked"], json!(1), "process: {process_json}");
    assert_eq!(
        process_json["succeeded"],
        json!(0),
        "process: {process_json}"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(job["state"], json!("blocked"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("block_watch_only_signer:"),
        "{job}"
    );

    env.shutdown();
}

// ── Claims never auto-retry on failure ──────────────────────────────────

#[tokio::test]
async fn claim_reward_failure_never_retries_and_parks_for_operator_review() {
    let env = setup_plan_env().await;
    env.rpc_state.fail_broadcast.store(true, Ordering::SeqCst);
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, sweep_step_id) = generate_and_simulate_plan(&env).await;

    let claim_step_id = add_plan_step(&env, &plan_id, &sweep_step_id, |step| {
        step["action"] = json!("claim_reward");
        step["asset_kind"] = json!("reward");
        step["asset_address"] = json!(null);
        step["protocol_address"] = json!("0x1111111111111111111111111111111111111111");
        step["claim_adapter"] = json!("merkle-distributor-v1");
        step["claim_index_hex"] = json!("0x7");
        step["claim_proof"] = json!([format!("0x{}", "11".repeat(32))]);
        step["amount_hex"] = json!("0xf4240");
        step["destination_address"] = json!(null);
        step["depends_on"] = json!([]);
    });
    // W5's claim gate additionally requires the claim contract to be
    // reviewed in the risk catalog before allow_claim_execution takes effect.
    mark_claim_contract_reviewed(&env, "0x1111111111111111111111111111111111111111");

    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &claim_step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue claim: {body}");
    let job_id = body["job"]["id"].as_str().unwrap().to_string();

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["operator_action_required"],
        json!(1),
        "process: {process_json}"
    );
    assert_eq!(process_json["failed"], json!(0), "process: {process_json}");
    assert_eq!(
        process_json["retrying"],
        json!(0),
        "process: {process_json}"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(job["state"], json!("operator_action_required"), "{job}");
    assert!(
        job["last_error"]
            .as_str()
            .unwrap()
            .starts_with("claim_execution_failed:"),
        "{job}"
    );

    // Draining again must NOT retry the claim: `operator_action_required`
    // is not a runnable state.
    let second_process = process_queue(&env).await;
    assert_eq!(
        second_process["processed"],
        json!(0),
        "must not reprocess: {second_process}"
    );

    env.shutdown();
}

// ── Gates off: EthSeed* legacy jobs gate the same way plan steps do ────────

#[tokio::test]
async fn eth_seed_jobs_are_gate_driven_and_execute_once_gates_pass() {
    let env = setup_plan_env().await;
    let job_id = insert_eth_seed_job(
        &env,
        json!({
            "kind": "eth_seed_native_sweep",
            "wallet_profile": "seed-main",
            "address": SEED_ADDRESS,
            "derivation_path": "m/44'/60'/0'/0/0",
            "destination_address": DESTINATION,
        }),
    );

    // Gates off (no treasury policy at all): gated the same way a
    // PlanStepExecution sweep step would be, not the pre-W7.3 unconditional
    // "seed-wallet queue execution is not enabled yet" message.
    let process_json = process_queue(&env).await;
    assert_eq!(process_json["blocked"], json!(1), "process: {process_json}");
    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(
        job["last_error"],
        json!("execution_gate: plan execution requires an enabled treasury policy"),
        "{job}"
    );

    // Gates on: the job actually signs and broadcasts.
    update_policy(&env, gates_on_policy_body()).await;
    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );
    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(job["state"], json!("sent"), "{job}");
    assert!(job["transaction_hash_hex"].is_string(), "{job}");

    env.shutdown();
}

#[tokio::test]
async fn eth_seed_native_sweep_reauthorizes_fresh_spendable_before_signing() {
    let env = setup_plan_env().await;
    let mut policy = gates_on_policy_body();
    // The raw queue payload's minimum is allowed at enqueue/drain-gate time,
    // while the provider's fresh one-ETH balance is deliberately over cap.
    policy["max_step_native_wei_hex"] = json!("0x1");
    update_policy(&env, policy).await;
    let job_id = insert_eth_seed_job(
        &env,
        json!({
            "kind": "eth_seed_native_sweep",
            "wallet_profile": "seed-main",
            "address": SEED_ADDRESS,
            "derivation_path": "m/44'/60'/0'/0/0",
            "destination_address": DESTINATION,
            "min_value_wei_hex": "0x1",
        }),
    );
    let nonce_count_before = env.rpc_state.nonce_count.load(Ordering::SeqCst);
    let broadcast_count_before = env.rpc_state.broadcast_count.load(Ordering::SeqCst);

    let process_json = process_queue(&env).await;
    assert_eq!(process_json["blocked"], json!(1), "process: {process_json}");
    assert_eq!(
        process_json["succeeded"],
        json!(0),
        "process: {process_json}"
    );

    let jobs = queue_jobs(&env).await;
    let job = jobs.iter().find(|job| job["id"] == json!(job_id)).unwrap();
    assert_eq!(job["state"], json!("blocked"), "{job}");
    assert_eq!(
        job["last_error"],
        json!("policy_violation: block_step_cap"),
        "{job}"
    );
    assert!(job["transaction_hash_hex"].is_null(), "never signed: {job}");
    assert_eq!(
        env.rpc_state.nonce_count.load(Ordering::SeqCst),
        nonce_count_before,
        "policy must block before nonce resolution"
    );
    assert_eq!(
        env.rpc_state.broadcast_count.load(Ordering::SeqCst),
        broadcast_count_before,
        "policy must block before broadcast"
    );
    let persisted = read_store(&queue_path(&env));
    let persisted_job = persisted["data"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == json!(job_id))
        .expect("persisted queue job exists");
    assert!(
        persisted_job.get("signed_raw_transaction_hex").is_none(),
        "policy block must not produce replayable signed bytes: {persisted_job}"
    );

    env.shutdown();
}

// ── Key hygiene: no key material anywhere ───────────────────────────────

#[tokio::test]
async fn no_key_material_appears_in_audit_queue_store_or_inventory_bytes() {
    let env = setup_plan_env().await;
    update_policy(&env, gates_on_policy_body()).await;
    let (plan_id, step_id) = generate_and_simulate_plan(&env).await;
    approve_plan(&env, &plan_id).await;
    let (status, body) = enqueue_step(&env, &plan_id, &step_id).await;
    assert_eq!(status, StatusCode::OK, "enqueue: {body}");

    let process_json = process_queue(&env).await;
    assert_eq!(
        process_json["succeeded"],
        json!(1),
        "process: {process_json}"
    );

    // The exact signing key this job derived and used, computed
    // independently the same way `derive_eth_seed_signing_key` does.
    let signing_key = sigillum_core::derive_ethereum_private_key_from_mnemonic(
        SEED_MNEMONIC,
        None,
        "m/44'/60'/0'/0/0",
    )
    .expect("derive test signing key");
    let key_hex = hex::encode(signing_key.to_bytes());
    assert_eq!(
        sigillum_core::ethereum_address_from_signing_key(&signing_key),
        SEED_ADDRESS,
        "sanity: the derived key must match the address this job actually signed with"
    );

    let events = audit_events(&env).await;
    let audit_text = serde_json::to_string(&events).unwrap();
    let queue_bytes = raw_bytes(&queue_path(&env));
    let inventory_bytes = raw_bytes(&inventory_path(&env));
    let queue_text = String::from_utf8_lossy(&queue_bytes);
    let inventory_text = String::from_utf8_lossy(&inventory_bytes);

    for (label, haystack) in [
        ("audit events", audit_text.as_str()),
        ("queue store", queue_text.as_ref()),
        ("wallet inventory store", inventory_text.as_ref()),
    ] {
        assert!(
            !haystack.to_lowercase().contains(&key_hex),
            "{label} must never contain the signing key hex"
        );
        assert!(
            !haystack.contains("abandon abandon abandon"),
            "{label} must never contain the seed mnemonic"
        );
    }

    env.shutdown();
}
