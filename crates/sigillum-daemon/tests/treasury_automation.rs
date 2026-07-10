//! W8 stage 3 integration tests: treasury automation maintenance cycles.
//! Mock-RPC only; helpers are intentionally duplicated in this file.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const SEED_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const RPC_TOKEN: &str = "rpc-test-token";
const PROFILE: &str = "seed-main";
const PROVIDER: &str = "mainnet";

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

#[derive(Clone, Default)]
struct RpcState {
    balances: Arc<Mutex<HashMap<String, String>>>,
}

impl RpcState {
    fn set_balance(&self, address: &str, balance_hex: String) {
        self.balances
            .lock()
            .unwrap()
            .insert(address.to_ascii_lowercase(), balance_hex);
    }
}

struct TreasuryEnv {
    _dir: TempDir,
    addr: SocketAddr,
    daemon: tokio::task::JoinHandle<()>,
    rpc: tokio::task::JoinHandle<()>,
    rpc_state: RpcState,
    client: reqwest::Client,
    token: String,
    hot_address: String,
    treasury_address: String,
}

impl TreasuryEnv {
    fn shutdown(self) {
        self.daemon.abort();
        self.rpc.abort();
    }
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

fn eth(milli: u64) -> String {
    let wei = (milli as u128) * 1_000_000_000_000_000u128;
    format!("0x{wei:x}")
}

fn rpc_response(state: &RpcState, request: &Value) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!("0x20"),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_getBalance" => {
            let address = request["params"][0]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let balance = state
                .balances
                .lock()
                .unwrap()
                .get(&address)
                .cloned()
                .unwrap_or_else(|| eth(1000));
            json!(balance)
        }
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
        "eth_getTransactionReceipt" => {
            let hash = request["params"][0].as_str().unwrap_or_default();
            json!({
                "transactionHash": hash,
                "blockNumber": "0x20",
                "status": "0x1",
                "gasUsed": "0x5208",
            })
        }
        "eth_sendRawTransaction" => submitted_raw_transaction_hash(request),
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

async fn setup_treasury_env(hot_balance: String, treasury_balance: String) -> TreasuryEnv {
    let dir = TempDir::new().unwrap();
    let base_dir = dir.path().to_path_buf();
    let (addr, daemon) = spawn_daemon(base_dir).await;
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
            "name": PROVIDER,
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
            "name": PROFILE,
            "label": "Seed main",
            "mnemonic": SEED_MNEMONIC,
            "project_account": 0,
            "provider_profile": PROVIDER,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed.status(), StatusCode::OK);

    let profiles = get(&client, addr, "/api/profiles/eth-seed", Some(&token)).await;
    assert_eq!(profiles.status(), StatusCode::OK);
    let profiles_json: Value = profiles.json().await.unwrap();
    let profile = profiles_json["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["name"] == json!(PROFILE))
        .expect("seed profile exists");
    let hot_address = profile["hot_address"].as_str().unwrap().to_string();
    let treasury_address = profile["treasury_address"].as_str().unwrap().to_string();

    rpc_state.set_balance(&hot_address, hot_balance);
    rpc_state.set_balance(&treasury_address, treasury_balance);

    let env = TreasuryEnv {
        _dir: dir,
        addr,
        daemon,
        rpc,
        rpc_state,
        client,
        token,
        hot_address,
        treasury_address,
    };
    rescan(&env).await;
    env
}

async fn rescan(env: &TreasuryEnv) {
    let scan = post_json(
        &env.client,
        env.addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": PROFILE,
            "provider_profile": PROVIDER,
            "gap_limit": 1,
            "max_index": 0,
        }),
        Some(&env.token),
    )
    .await;
    let status = scan.status();
    let body: Value = scan.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "scan response: {body}");
}

async fn update_policy(env: &TreasuryEnv, body: Value) {
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

fn automation_policy_body(
    env: &TreasuryEnv,
    floor: String,
    target: String,
    overflow: String,
) -> Value {
    json!({
        "enabled": true,
        "allowed_destinations": [
            { "address": env.hot_address, "label": "hot" },
            { "address": env.treasury_address, "label": "treasury" }
        ],
        "allow_plan_execution": true,
        "allow_sweep_execution": true,
        "allow_treasury_automation": true,
        "hot_floor_wei_hex": floor,
        "hot_target_wei_hex": target,
        "hot_overflow_wei_hex": overflow,
    })
}

async fn run_maintenance(env: &TreasuryEnv) -> Value {
    let response = post_json(
        &env.client,
        env.addr,
        "/api/maintenance/run",
        json!({}),
        Some(&env.token),
    )
    .await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "maintenance response: {body}");
    body
}

async fn plans(env: &TreasuryEnv) -> Vec<Value> {
    let response = get(
        &env.client,
        env.addr,
        "/api/plans/consolidation",
        Some(&env.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["plans"].as_array().unwrap().clone()
}

async fn queue_jobs(env: &TreasuryEnv) -> Vec<Value> {
    let response = get(&env.client, env.addr, "/api/queue/jobs", Some(&env.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["jobs"].as_array().unwrap().clone()
}

async fn audit_events(env: &TreasuryEnv) -> Vec<Value> {
    let response = get(
        &env.client,
        env.addr,
        "/api/audit?limit=200",
        Some(&env.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["events"].as_array().unwrap().clone()
}

async fn treasury_overview(env: &TreasuryEnv) -> Value {
    let response = get(
        &env.client,
        env.addr,
        "/api/treasury/overview",
        Some(&env.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

fn automation_plans(plans: &[Value]) -> Vec<Value> {
    plans
        .iter()
        .filter(|plan| plan["origin"] == json!("treasury_automation"))
        .cloned()
        .collect()
}

fn automation_steps(plans: &[Value]) -> Vec<Value> {
    automation_plans(plans)
        .into_iter()
        .flat_map(|plan| plan["steps"].as_array().unwrap().clone())
        .collect()
}

fn assert_no_both_directions(steps: &[Value], hot: &str, treasury: &str) {
    let has_overflow = steps.iter().any(|step| {
        step["address"].as_str().unwrap().eq_ignore_ascii_case(hot)
            && step["destination_address"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case(treasury)
    });
    let has_refill = steps.iter().any(|step| {
        step["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(treasury)
            && step["destination_address"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case(hot)
    });
    assert!(
        !(has_overflow && has_refill),
        "cycle generated both directions: {steps:?}"
    );
}

#[tokio::test]
async fn automation_off_keeps_maintenance_byte_identical() {
    for configure_policy in [false, true] {
        let env = setup_treasury_env(eth(3000), eth(5000)).await;
        if configure_policy {
            let mut body = automation_policy_body(&env, eth(500), eth(1000), eth(2000));
            body.as_object_mut()
                .unwrap()
                .remove("allow_treasury_automation");
            update_policy(&env, body).await;
        }

        let body = run_maintenance(&env).await;
        assert!(
            body.get("treasury_automation").is_none(),
            "off-path response changed: {body}"
        );
        let keys: BTreeSet<_> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "status",
                "refreshed",
                "detected",
                "queued",
                "processed",
                "succeeded",
                "blocked",
                "retrying",
                "operator_action_required",
                "failed",
                "confirmed",
                "failures_by_cause",
                "deposits",
                "jobs",
            ])
        );
        assert!(plans(&env).await.is_empty());
        assert!(queue_jobs(&env).await.is_empty());
        assert!(
            !audit_events(&env)
                .await
                .iter()
                .any(|event| event["kind"] == json!("treasury.automation_run"))
        );
        env.shutdown();
    }
}

#[tokio::test]
async fn overflow_generates_and_auto_enqueues_through_w7_2() {
    let env = setup_treasury_env(eth(3000), eth(5000)).await;
    update_policy(
        &env,
        automation_policy_body(&env, eth(500), eth(1000), eth(2000)),
    )
    .await;

    let body = run_maintenance(&env).await;
    assert_eq!(body["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(body["treasury_automation"]["enqueued_steps"], json!(1));
    assert_eq!(body["treasury_automation"]["skipped_steps"], json!(0));

    let plans = plans(&env).await;
    let automation = automation_plans(&plans);
    assert_eq!(automation.len(), 1, "plans: {plans:?}");
    let plan = &automation[0];
    let steps = plan["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 1, "plan: {plan}");
    let step = &steps[0];
    assert_eq!(step["action"], json!("sweep_native"));
    assert!(
        step["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.hot_address)
    );
    assert!(
        step["destination_address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.treasury_address)
    );
    assert_eq!(step["amount_hex"], json!(eth(2000)));
    assert_eq!(step["simulation_status"], json!("passed"));
    assert_eq!(step["approved"], json!(true));
    assert_eq!(step["auto_eligible"], json!(true));
    let queued_job_id = step["queued_job_id"].as_str().unwrap();

    let jobs = queue_jobs(&env).await;
    assert_eq!(jobs.len(), 1, "jobs: {jobs:?}");
    let job = &jobs[0];
    assert_eq!(job["id"], json!(queued_job_id));
    assert_eq!(job["kind"], json!("plan_step_execution"));
    assert_eq!(job["plan_id"], plan["id"]);
    assert_eq!(job["step_id"], step["id"]);
    assert!(
        !job["simulation_evidence_hash_hex"]
            .as_str()
            .unwrap()
            .is_empty(),
        "job: {job}"
    );

    let events = audit_events(&env).await;
    assert!(events.iter().any(|event| {
        event["kind"] == json!("wallet_consolidation.plan.enqueue_step")
            && event["details"]["plan_id"] == plan["id"]
            && event["details"]["step_id"] == step["id"]
    }));
    assert!(events.iter().any(|event| {
        event["kind"] == json!("treasury.automation_run")
            && event["details"]["generated"] == json!(1)
            && event["details"]["enqueued"] == json!(1)
            && event["details"]["skipped"] == json!(0)
    }));

    let overview = treasury_overview(&env).await;
    assert_eq!(overview["automation"]["enabled"], json!(true));
    assert_eq!(overview["automation"]["generated_steps"], json!(1));
    assert_eq!(overview["automation"]["enqueued_steps"], json!(1));
    env.shutdown();
}

#[tokio::test]
async fn refill_generates_treasury_to_hot() {
    let env = setup_treasury_env(eth(200), eth(5000)).await;
    update_policy(
        &env,
        automation_policy_body(&env, eth(500), eth(1000), eth(2000)),
    )
    .await;

    let body = run_maintenance(&env).await;
    assert_eq!(body["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(body["treasury_automation"]["enqueued_steps"], json!(1));

    let plans = plans(&env).await;
    let steps = automation_steps(&plans);
    assert_eq!(steps.len(), 1, "steps: {steps:?}");
    let step = &steps[0];
    assert!(
        step["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.treasury_address)
    );
    assert!(
        step["destination_address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.hot_address)
    );
    assert_eq!(step["amount_hex"], json!(eth(800)));
    assert_no_both_directions(&steps, &env.hot_address, &env.treasury_address);
    env.shutdown();
}

#[tokio::test]
async fn gates_off_generated_but_not_enqueued() {
    let env = setup_treasury_env(eth(3000), eth(5000)).await;
    let mut body = automation_policy_body(&env, eth(500), eth(1000), eth(2000));
    body["allow_plan_execution"] = json!(false);
    update_policy(&env, body).await;

    let body = run_maintenance(&env).await;
    assert_eq!(body["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(body["treasury_automation"]["enqueued_steps"], json!(0));
    assert!(
        body["treasury_automation"]["skipped_steps"]
            .as_u64()
            .unwrap()
            >= 1,
        "maintenance: {body}"
    );
    assert!(
        !body["treasury_automation"]["skipped_reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        body["failures_by_cause"]["policy_block"].as_u64().unwrap() >= 1,
        "maintenance: {body}"
    );

    let steps = automation_steps(&plans(&env).await);
    assert_eq!(steps.len(), 1, "steps: {steps:?}");
    let step = &steps[0];
    assert_eq!(step["status"], json!("review_required"));
    assert_eq!(step["approved"], json!(false));
    assert_eq!(step["auto_eligible"], json!(false));
    assert!(step["queued_job_id"].is_null());
    assert!(queue_jobs(&env).await.is_empty());
    env.shutdown();
}

#[tokio::test]
async fn destination_not_allowlisted_blocks_automation_step() {
    let env = setup_treasury_env(eth(3000), eth(5000)).await;
    let mut body = automation_policy_body(&env, eth(500), eth(1000), eth(2000));
    body["allowed_destinations"] =
        json!([{ "address": "0x9999999999999999999999999999999999999999" }]);
    update_policy(&env, body).await;

    let body = run_maintenance(&env).await;
    assert_eq!(body["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(body["treasury_automation"]["enqueued_steps"], json!(0));
    assert!(
        body["failures_by_cause"]["policy_block"].as_u64().unwrap() >= 1,
        "maintenance: {body}"
    );

    let steps = automation_steps(&plans(&env).await);
    assert_eq!(steps.len(), 1, "steps: {steps:?}");
    let step = &steps[0];
    assert_eq!(step["status"], json!("blocked"));
    assert_eq!(step["blockers"], json!(["block_destination"]));
    assert!(queue_jobs(&env).await.is_empty());
    env.shutdown();
}

#[tokio::test]
async fn oscillation_no_ping_pong_across_cycles() {
    let env = setup_treasury_env(eth(2500), eth(5000)).await;
    update_policy(
        &env,
        automation_policy_body(&env, eth(1000), eth(1500), eth(2000)),
    )
    .await;

    let cycle_1 = run_maintenance(&env).await;
    assert_eq!(cycle_1["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(cycle_1["treasury_automation"]["enqueued_steps"], json!(1));
    let steps_1 = automation_steps(&plans(&env).await);
    assert_eq!(steps_1.len(), 1, "steps: {steps_1:?}");
    assert!(
        steps_1[0]["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.hot_address)
    );
    assert!(
        steps_1[0]["destination_address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.treasury_address)
    );
    assert_eq!(steps_1[0]["amount_hex"], json!(eth(1000)));
    assert_no_both_directions(&steps_1, &env.hot_address, &env.treasury_address);

    let cycle_2 = run_maintenance(&env).await;
    assert_eq!(cycle_2["treasury_automation"]["generated_steps"], json!(0));
    assert!(
        cycle_2["treasury_automation"]["skipped_steps"]
            .as_u64()
            .unwrap()
            >= 1,
        "cycle 2: {cycle_2}"
    );
    let steps_2 = automation_steps(&plans(&env).await);
    assert_eq!(steps_2.len(), 1, "steps: {steps_2:?}");
    assert_no_both_directions(&[], &env.hot_address, &env.treasury_address);
    let jobs = queue_jobs(&env).await;
    assert_eq!(jobs.len(), 1, "jobs: {jobs:?}");
    assert_eq!(jobs[0]["state"], json!("confirmed"), "jobs: {jobs:?}");
    env.rpc_state.set_balance(&env.hot_address, eth(1500));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    rescan(&env).await;

    let before_cycle_3 = automation_steps(&plans(&env).await).len();
    let cycle_3 = run_maintenance(&env).await;
    assert_eq!(cycle_3["treasury_automation"]["generated_steps"], json!(0));
    let steps_3 = automation_steps(&plans(&env).await);
    assert_eq!(steps_3.len(), before_cycle_3, "steps: {steps_3:?}");
    assert_no_both_directions(&[], &env.hot_address, &env.treasury_address);

    env.rpc_state.set_balance(&env.hot_address, eth(950));
    let cycle_4 = run_maintenance(&env).await;
    assert_eq!(cycle_4["treasury_automation"]["generated_steps"], json!(0));
    let steps_4 = automation_steps(&plans(&env).await);
    assert_eq!(steps_4.len(), before_cycle_3, "steps: {steps_4:?}");
    assert_no_both_directions(&[], &env.hot_address, &env.treasury_address);

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    rescan(&env).await;
    let cycle_5 = run_maintenance(&env).await;
    assert_eq!(cycle_5["treasury_automation"]["generated_steps"], json!(1));
    assert_eq!(cycle_5["treasury_automation"]["enqueued_steps"], json!(1));
    let steps_5 = automation_steps(&plans(&env).await);
    assert_eq!(steps_5.len(), 2, "steps: {steps_5:?}");
    let new_step = &steps_5[1];
    assert!(
        new_step["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.treasury_address)
    );
    assert!(
        new_step["destination_address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&env.hot_address)
    );
    assert_eq!(new_step["amount_hex"], json!(eth(550)));
    assert_no_both_directions(
        std::slice::from_ref(new_step),
        &env.hot_address,
        &env.treasury_address,
    );
    env.shutdown();
}
