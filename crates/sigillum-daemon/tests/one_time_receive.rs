//! One-time receive-address lifecycle tests (plan task 3.3).
//!
//! Own binary because it overrides `SIGILLUM_SCHEDULER_QUEUE_TICK_SECS` /
//! `SIGILLUM_SCHEDULER_REFRESH_SECS` / `SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS`
//! process-wide before the daemon's startup config is read (the
//! `tests/scheduler.rs` precedent).
//!
//! Covered scenarios (plan acceptance + companions):
//! (a) fund a fresh one-time allocation → the scheduler observes the funds →
//!     a sweep is enqueued and executed to the allocation's destination → the
//!     allocation retires when the sweep settles → with `purge_after_sweep`
//!     the record is purged (store assertions + audit events);
//! (b) below the sweep threshold nothing enqueues (funds sit);
//! (c) execution gates off ⇒ nothing enqueues and the lifecycle reads
//!     `watching` / `execution_gates`; opening the gates later sweeps;
//! (d) dedupe: exactly one sweep job is ever enqueued across ticks;
//! (e) a shared destination across two parties is hard-blocked under the
//!     default-on `block_cross_party_linkage` posture.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
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

const SEED_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const RPC_TOKEN: &str = "rpc-test-token";
const RPC_AUTH: &str = "Bearer rpc-test-token";
const PASSPHRASE: &str = "correct horse battery staple";
const PROFILE: &str = "seed-main";
const PROVIDER: &str = "mainnet";
const DESTINATION: &str = "0x2222222222222222222222222222222222222222";
/// 0.05 ETH: the funded balance in the happy-path tests.
const FUNDED_WEI_HEX: &str = "0xb1a2bc2ec50000";
/// 0.04 ETH: the sweep threshold (below the balance, so the spendable amount
/// after gas still clears it at execution).
const THRESHOLD_WEI_HEX: &str = "0x8f0d180e480000";

/// Must land before `spawn_scheduler` reads the startup config. Same values
/// in every test of this binary, so the parallel test threads never disagree.
fn set_scheduler_env() {
    unsafe {
        std::env::set_var("SIGILLUM_SCHEDULER_QUEUE_TICK_SECS", "1");
        std::env::set_var("SIGILLUM_SCHEDULER_REFRESH_SECS", "1");
        std::env::set_var("SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS", "1");
    }
}

#[derive(Clone, Default)]
struct RpcState {
    balances: Arc<Mutex<HashMap<String, String>>>,
    broadcast_count: Arc<Mutex<usize>>,
}

impl RpcState {
    fn set_balance(&self, address: &str, balance_hex: &str) {
        self.balances
            .lock()
            .unwrap()
            .insert(address.to_ascii_lowercase(), balance_hex.to_string());
    }

    fn broadcasts(&self) -> usize {
        *self.broadcast_count.lock().unwrap()
    }
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
                .unwrap_or_else(|| "0x0".to_string());
            json!(balance)
        }
        "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
        "eth_feeHistory" => json!({
            "oldestBlock": "0x1",
            "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
            "gasUsedRatio": [0.5]
        }),
        "eth_call" => json!("0x0f4240"),
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
        "eth_sendRawTransaction" => {
            let raw = request["params"][0]
                .as_str()
                .expect("eth_sendRawTransaction carries raw transaction hex");
            *state.broadcast_count.lock().unwrap() += 1;
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
        if auth != RPC_AUTH {
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

struct OneTimeEnv {
    _dir: TempDir,
    addr: SocketAddr,
    daemon: tokio::task::JoinHandle<()>,
    rpc: tokio::task::JoinHandle<()>,
    rpc_state: RpcState,
    client: reqwest::Client,
    token: String,
}

impl OneTimeEnv {
    fn shutdown(self) {
        self.daemon.abort();
        self.rpc.abort();
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let response = post_json(&self.client, self.addr, path, body, Some(&self.token)).await;
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "POST {path} failed: {body}");
        body
    }

    async fn get(&self, path: &str) -> Value {
        get_json(&self.client, self.addr, path, &self.token).await
    }

    async fn allocations(&self) -> Vec<Value> {
        self.get("/api/treasury/receive-addresses").await["allocations"]
            .as_array()
            .unwrap()
            .clone()
    }

    async fn queue_jobs(&self) -> Vec<Value> {
        self.get("/api/queue/jobs").await["jobs"]
            .as_array()
            .unwrap()
            .clone()
    }

    async fn audit_kinds(&self) -> Vec<(String, Value)> {
        self.get("/api/audit?limit=400").await["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap().to_string(),
                    event["details"].clone(),
                )
            })
            .collect()
    }
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

/// Spawn the daemon (scheduler on), the mock provider, and provision the
/// compartment, API key, provider profile, and seed wallet profile. Policy is
/// left to each test.
async fn setup_env() -> OneTimeEnv {
    set_scheduler_env();
    let dir = TempDir::new().unwrap();
    let base_dir: PathBuf = dir.path().to_path_buf();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    // The scheduler spawns with the server entry point in production, so
    // tests opt in explicitly — exactly the call the production path makes.
    sigillum_daemon::spawn_scheduler(state);
    let daemon = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

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
            "passphrase": PASSPHRASE,
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
            "max_fee_per_gas_hex": "0x77359400",
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

    OneTimeEnv {
        _dir: dir,
        addr,
        daemon,
        rpc,
        rpc_state,
        client,
        token,
    }
}

/// Enable the treasury policy with the Sweep execution-family gates open and
/// the sweep destination allow-listed. `block_cross_party_linkage` is
/// omitted on purpose: the default-on posture (plan task 3.5) applies.
async fn open_policy(env: &OneTimeEnv) {
    env.post(
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "allowed_destinations": [{ "address": DESTINATION, "label": "cold" }],
        }),
    )
    .await;
}

async fn create_party(env: &OneTimeEnv, name: &str) -> String {
    env.post("/api/treasury/parties", json!({ "name": name }))
        .await["party"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn allocate_one_time(
    env: &OneTimeEnv,
    counterparty_id: Option<&str>,
    threshold: Option<&str>,
    purge_after_sweep: bool,
) -> Value {
    let mut body = json!({
        "wallet_profile": PROFILE,
        "purpose": "one-time-invoice",
        "one_time": true,
        "sweep_destination_address": DESTINATION,
        "purge_after_sweep": purge_after_sweep,
    });
    if let Some(id) = counterparty_id {
        body["counterparty_id"] = json!(id);
    }
    if let Some(threshold) = threshold {
        body["min_sweep_amount_hex"] = json!(threshold);
    }
    let allocation = env
        .post("/api/treasury/receive-addresses/allocate", body)
        .await["allocation"]
        .clone();
    assert_eq!(allocation["one_time"], json!(true), "{allocation}");
    assert_eq!(
        allocation["sweep_destination_address"],
        json!(DESTINATION),
        "{allocation}"
    );
    allocation
}

// ── (a) plan acceptance: fund → observe → sweep → retire → purge ───────────

#[tokio::test]
async fn one_time_allocation_sweeps_retires_and_purges_end_to_end() {
    let env = setup_env().await;
    open_policy(&env).await;
    let party_id = create_party(&env, "Client One").await;
    let allocation = allocate_one_time(&env, Some(&party_id), Some(THRESHOLD_WEI_HEX), true).await;
    let address = allocation["address"].as_str().unwrap().to_string();
    let allocation_id = allocation["id"].as_str().unwrap().to_string();

    // Fund the address: the scheduler's auto-watch must pick the balance up.
    env.rpc_state.set_balance(&address, FUNDED_WEI_HEX);

    // The sweep is enqueued and drained without any client driving it.
    wait_for(
        "the one-time sweep to broadcast",
        Duration::from_secs(30),
        || async {
            env.queue_jobs()
                .await
                .iter()
                .any(|job| job["state"] == json!("sent"))
        },
    )
    .await;

    let jobs = env.queue_jobs().await;
    assert_eq!(jobs.len(), 1, "exactly one sweep job: {jobs:?}");
    let job = &jobs[0];
    assert_eq!(job["kind"], json!("eth_seed_native_sweep"), "{job}");
    assert_eq!(
        job["destination_address"],
        json!(DESTINATION),
        "sweep pays the allocation's destination: {job}"
    );
    assert_eq!(job["address"], json!(address), "{job}");
    assert_eq!(env.rpc_state.broadcasts(), 1, "exactly one broadcast");

    // The settle pass retires, then purges the record (purge_after_sweep).
    wait_for(
        "the allocation record to be purged",
        Duration::from_secs(30),
        || async { env.allocations().await.is_empty() },
    )
    .await;

    let audit = env.audit_kinds().await;
    let allocate = audit
        .iter()
        .find(|(kind, _)| kind == "treasury.receive.allocate")
        .expect("allocate audit event");
    assert_eq!(allocate.1["one_time"], json!(true), "{audit:?}");
    assert!(
        audit.iter().any(|(kind, _)| kind == "queue.enqueue"),
        "sweep enqueue audited: {audit:?}"
    );
    let retire = audit
        .iter()
        .find(|(kind, _)| kind == "treasury.receive.retire")
        .expect("retire audit event");
    assert_eq!(retire.1["id"], json!(allocation_id));
    assert_eq!(retire.1["reason"], json!("one_time_sweep_settled"));
    let purge = audit
        .iter()
        .find(|(kind, _)| kind == "treasury.receive.purge")
        .expect("purge audit event");
    assert_eq!(purge.1["id"], json!(allocation_id));
    assert_eq!(purge.1["counterparty_binding_removed"], json!(true));

    env.shutdown();
}

// ── (b) below the threshold funds just sit ──────────────────────────────────

#[tokio::test]
async fn below_threshold_allocation_accrues_without_sweeping() {
    let env = setup_env().await;
    open_policy(&env).await;
    let allocation = allocate_one_time(&env, None, Some(THRESHOLD_WEI_HEX), false).await;
    let address = allocation["address"].as_str().unwrap().to_string();

    // Half the threshold: observed, but never enough to sweep.
    env.rpc_state.set_balance(&address, "0x47868c6ca40000");

    wait_for(
        "the balance observation",
        Duration::from_secs(20),
        || async { env.allocations().await[0]["sweep_blocker"] == json!("below_threshold") },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;

    assert!(
        env.queue_jobs().await.is_empty(),
        "no sweep below threshold"
    );
    assert_eq!(env.rpc_state.broadcasts(), 0);
    let allocation = &env.allocations().await[0];
    assert_eq!(allocation["lifecycle_state"], json!("watching"));
    assert_eq!(allocation["sweep_blocker"], json!("below_threshold"));
    assert_eq!(allocation["status"], json!("active"));

    env.shutdown();
}

// ── (c) gates off ⇒ waiting on gates; opening the gates sweeps ─────────────

#[tokio::test]
async fn gates_off_allocation_waits_then_sweeps_when_gates_open() {
    let env = setup_env().await;
    // NOTE: no policy at all yet — the Sweep family gates cannot hold.
    let allocation = allocate_one_time(&env, None, Some(THRESHOLD_WEI_HEX), false).await;
    let address = allocation["address"].as_str().unwrap().to_string();
    env.rpc_state.set_balance(&address, FUNDED_WEI_HEX);

    // The allocation accrues; nothing enqueues while the gates are closed.
    wait_for(
        "the gates blocker to surface",
        Duration::from_secs(20),
        || async { env.allocations().await[0]["sweep_blocker"] == json!("execution_gates") },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(
        env.queue_jobs().await.is_empty(),
        "gated: no sweep enqueued"
    );
    assert_eq!(env.rpc_state.broadcasts(), 0);
    let allocation = &env.allocations().await[0];
    assert_eq!(allocation["lifecycle_state"], json!("watching"));
    assert_eq!(allocation["sweep_blocker"], json!("execution_gates"));

    // Opening the gates lets the next cycle sweep (and the settle pass
    // retires the allocation afterwards).
    open_policy(&env).await;
    wait_for(
        "the sweep to broadcast once gates open",
        Duration::from_secs(30),
        || async {
            env.queue_jobs()
                .await
                .iter()
                .any(|job| job["state"] == json!("sent"))
        },
    )
    .await;
    wait_for(
        "the allocation to retire",
        Duration::from_secs(30),
        || async { env.allocations().await[0]["status"] == json!("retired") },
    )
    .await;
    let allocation = &env.allocations().await[0];
    assert_eq!(allocation["lifecycle_state"], json!("retired"));
    assert!(allocation["retired_at_unix"].is_u64(), "{allocation}");

    env.shutdown();
}

// ── (d) dedupe: exactly one sweep job across ticks ─────────────────────────

#[tokio::test]
async fn one_time_sweep_enqueues_exactly_once_across_ticks() {
    let env = setup_env().await;
    open_policy(&env).await;
    let allocation = allocate_one_time(&env, None, Some(THRESHOLD_WEI_HEX), false).await;
    let address = allocation["address"].as_str().unwrap().to_string();
    env.rpc_state.set_balance(&address, FUNDED_WEI_HEX);

    wait_for(
        "the sweep to broadcast",
        Duration::from_secs(30),
        || async {
            env.queue_jobs()
                .await
                .iter()
                .any(|job| job["state"] == json!("sent"))
        },
    )
    .await;
    let job_id = env.queue_jobs().await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Many ticks pass (the balance never changes on the mock): no second job,
    // no second broadcast, and the tracked job id never changes.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let jobs = env.queue_jobs().await;
    assert_eq!(jobs.len(), 1, "no double-enqueue across ticks: {jobs:?}");
    assert_eq!(jobs[0]["id"], json!(job_id));
    assert_eq!(
        env.rpc_state.broadcasts(),
        1,
        "never re-signed/re-broadcast"
    );
    let enqueues = env
        .audit_kinds()
        .await
        .into_iter()
        .filter(|(kind, _)| kind == "queue.enqueue")
        .count();
    assert_eq!(enqueues, 1, "exactly one enqueue audit event");

    // The settle pass still retires the allocation (no purge configured).
    wait_for(
        "the allocation to retire",
        Duration::from_secs(30),
        || async { env.allocations().await[0]["status"] == json!("retired") },
    )
    .await;

    env.shutdown();
}

// ── (e) shared destination across parties hard-blocks (default-on) ─────────

#[tokio::test]
async fn shared_destination_across_parties_hard_blocks_both_sweeps() {
    let env = setup_env().await;
    open_policy(&env).await;
    let party_one = create_party(&env, "Client One").await;
    let party_two = create_party(&env, "Client Two").await;
    let first = allocate_one_time(&env, Some(&party_one), Some(THRESHOLD_WEI_HEX), false).await;
    let second = allocate_one_time(&env, Some(&party_two), Some(THRESHOLD_WEI_HEX), false).await;
    for allocation in [&first, &second] {
        env.rpc_state
            .set_balance(allocation["address"].as_str().unwrap(), FUNDED_WEI_HEX);
    }

    // Both allocations are funded and sweep-eligible in isolation, but they
    // would converge on ONE destination for TWO parties: with the default-on
    // block_cross_party_linkage posture neither may sweep.
    wait_for(
        "the linkage blocker on both allocations",
        Duration::from_secs(20),
        || async {
            let allocations = env.allocations().await;
            allocations.len() == 2
                && allocations
                    .iter()
                    .all(|allocation| allocation["sweep_blocker"] == json!("cross_party_linkage"))
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;

    assert!(
        env.queue_jobs().await.is_empty(),
        "shared-destination sweeps are hard-blocked"
    );
    assert_eq!(env.rpc_state.broadcasts(), 0);
    for allocation in env.allocations().await {
        assert_eq!(allocation["lifecycle_state"], json!("watching"));
        assert_eq!(allocation["sweep_blocker"], json!("cross_party_linkage"));
    }

    env.shutdown();
}
