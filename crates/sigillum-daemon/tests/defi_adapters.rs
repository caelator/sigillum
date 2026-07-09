use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

const OWNER_ADDRESS: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const GOOD_VAULT: &str = "0xdead4626000000000000000000000000000000aa";
const NON_4626_TOKEN: &str = "0xdead4626000000000000000000000000000000bb";

#[derive(Clone, Debug)]
struct RpcConfig {
    native_balance_hex: String,
    redeem_reverts: bool,
}

#[derive(Clone)]
struct RpcState {
    config: Arc<RpcConfig>,
}

struct TestDaemon {
    _dir: TempDir,
    addr: SocketAddr,
    daemon_handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    token: String,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.daemon_handle.abort();
        self.rpc_handle.abort();
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

async fn spawn_mock_evm_provider(config: RpcConfig) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(RpcState {
            config: Arc::new(config),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

async fn rpc_handler(
    State(state): State<RpcState>,
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
        Value::Array(
            requests
                .iter()
                .map(|request| rpc_response(&state.config, request))
                .collect(),
        )
    } else {
        rpc_response(&state.config, &body)
    };

    (StatusCode::OK, Json(payload))
}

fn rpc_response(config: &RpcConfig, request: &Value) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    if method == "eth_call" {
        return eth_call_response(config, request);
    }

    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!("0x20"),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_getBalance" => json!(config.native_balance_hex),
        "eth_getLogs" => json!([]),
        other => json!({ "unsupported": other }),
    };
    json_rpc_result(request, result)
}

fn eth_call_response(config: &RpcConfig, request: &Value) -> Value {
    let to = request["params"][0]["to"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let data = request["params"][0]["data"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if (to == GOOD_VAULT || to == NON_4626_TOKEN) && data.starts_with("0x70a08231") {
        return json_rpc_result(request, json!(abi_word(0xf4240)));
    }

    if to == GOOD_VAULT && data.starts_with("0xd905777e") {
        return json_rpc_result(request, json!(abi_word(0xe8480)));
    }
    if to == GOOD_VAULT && data.starts_with("0x07a2d13a") {
        return json_rpc_result(request, json!(abi_word(0xe8480)));
    }
    if to == GOOD_VAULT && data.starts_with("0xba087652") {
        if config.redeem_reverts {
            return json_rpc_error(request, 3, "execution reverted");
        }
        return json_rpc_result(request, json!(abi_word(0xf0000)));
    }

    if to == NON_4626_TOKEN && (data.starts_with("0xd905777e") || data.starts_with("0x07a2d13a")) {
        return json_rpc_error(request, 3, "execution reverted");
    }

    json_rpc_result(request, json!(abi_word(0)))
}

fn abi_word(value: u64) -> String {
    format!("0x{value:064x}")
}

fn json_rpc_result(request: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or_else(|| json!(1)),
        "result": result,
    })
}

fn json_rpc_error(request: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or_else(|| json!(1)),
        "error": {
            "code": code,
            "message": message,
        },
    })
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

async fn setup_daemon(config: RpcConfig) -> TestDaemon {
    let dir = TempDir::new().unwrap();
    let (addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider(config).await;
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
    let init_status = init.status();
    let init_json: Value = init.json().await.unwrap();
    assert_eq!(init_status, StatusCode::OK, "init response: {init_json}");
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let api_key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;
    let api_key_status = api_key.status();
    let api_key_json: Value = api_key.json().await.unwrap();
    assert_eq!(
        api_key_status,
        StatusCode::OK,
        "api key response: {api_key_json}"
    );

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
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
        }),
        Some(&token),
    )
    .await;
    let provider_status = provider.status();
    let provider_json: Value = provider.json().await.unwrap();
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider response: {provider_json}"
    );

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
    let seed_status = seed.status();
    let seed_json: Value = seed.json().await.unwrap();
    assert_eq!(seed_status, StatusCode::OK, "seed response: {seed_json}");

    TestDaemon {
        _dir: dir,
        addr,
        daemon_handle,
        rpc_handle,
        client,
        token,
    }
}

fn default_rpc_config(native_balance_hex: &str) -> RpcConfig {
    RpcConfig {
        native_balance_hex: native_balance_hex.into(),
        redeem_reverts: false,
    }
}

async fn scan_erc4626_probe(setup: &TestDaemon, token_address: &str) -> Value {
    let scan = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": "mainnet",
            "gap_limit": 1,
            "max_index": 0,
            "discover_defi_token_positions": true,
            "defi_token_probes": [{
                "protocol": "erc4626",
                "token_address": token_address,
            }],
            "defi_position_limit": 8,
        }),
        Some(&setup.token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    scan_json
}

async fn generate_consolidation_plan(setup: &TestDaemon, body: Value) -> Value {
    let plan = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/generate",
        body,
        Some(&setup.token),
    )
    .await;
    let plan_status = plan.status();
    let plan_json: Value = plan.json().await.unwrap();
    assert_eq!(plan_status, StatusCode::OK, "plan response: {plan_json}");
    plan_json
}

async fn simulate_step(setup: &TestDaemon, plan_id: &str, step_id: &str) -> Value {
    let simulate = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/simulate",
        json!({
            "plan_id": plan_id,
            "step_ids": [step_id],
        }),
        Some(&setup.token),
    )
    .await;
    let simulate_status = simulate.status();
    let simulate_json: Value = simulate.json().await.unwrap();
    assert_eq!(
        simulate_status,
        StatusCode::OK,
        "simulate response: {simulate_json}"
    );
    simulate_json
}

fn defi_holding<'a>(scan_json: &'a Value, token_address: &str) -> &'a Value {
    scan_json["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|holding| {
            holding["asset_kind"] == "defi"
                && holding["asset_address"]
                    .as_str()
                    .is_some_and(|address| address.eq_ignore_ascii_case(token_address))
        })
        .unwrap_or_else(|| panic!("missing defi holding for {token_address} in {scan_json}"))
}

fn exit_defi_step<'a>(plan_json: &'a Value, adapter: Option<&str>) -> &'a Value {
    plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| {
            step["action"] == "exit_defi_position"
                && adapter.is_none_or(|adapter| step["claim_adapter"] == adapter)
        })
        .unwrap_or_else(|| panic!("missing exit_defi_position step in {plan_json}"))
}

fn step_by_id<'a>(plan_json: &'a Value, step_id: &str) -> &'a Value {
    plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == step_id)
        .unwrap_or_else(|| panic!("missing step {step_id} in {plan_json}"))
}

fn evidence_contains(step: &Value, expected: &str) -> bool {
    step["simulation_evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evidence| evidence == expected)
}

fn evidence_contains_prefix(step: &Value, prefix: &str) -> bool {
    step["simulation_evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evidence| {
            evidence
                .as_str()
                .is_some_and(|value| value.starts_with(prefix))
        })
}

fn blockers_contain(step: &Value, expected: &str) -> bool {
    step["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker == expected)
}

#[tokio::test]
async fn defi_erc4626_detection_records_adapter_and_max_redeem_amount() {
    let setup = setup_daemon(default_rpc_config("0x0")).await;

    let scan_json = scan_erc4626_probe(&setup, GOOD_VAULT).await;
    let holding = defi_holding(&scan_json, GOOD_VAULT);

    assert_eq!(holding["asset_kind"], "defi");
    assert_eq!(holding["source"], "defi-token-probe:erc4626");
    assert_eq!(holding["claim_adapter"], "erc4626-redeem");
    assert_eq!(holding["amount_hex"], "0xe8480");
    assert_eq!(holding["protocol_address"], GOOD_VAULT);

    drop(setup);
}

#[tokio::test]
async fn defi_erc4626_detection_fails_closed_without_interface() {
    let setup = setup_daemon(default_rpc_config("0x0")).await;

    let scan_json = scan_erc4626_probe(&setup, NON_4626_TOKEN).await;
    let holding = defi_holding(&scan_json, NON_4626_TOKEN);
    assert!(holding.get("claim_adapter").is_none_or(Value::is_null));
    assert_eq!(holding["amount_hex"], "0xf4240");

    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, None);
    assert_eq!(step["status"], "blocked");
    assert!(blockers_contain(step, "requires_protocol_adapter"));

    drop(setup);
}

#[tokio::test]
async fn defi_erc4626_preflight_pass_records_expected_assets_out() {
    let setup = setup_daemon(default_rpc_config("0xde0b6b3a7640000")).await;

    scan_erc4626_probe(&setup, GOOD_VAULT).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("erc4626-redeem"));
    assert_eq!(step["status"], "review_required");
    assert_eq!(step["simulation_status"], "required");
    assert_eq!(step["address"], OWNER_ADDRESS);
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let step_id = step["id"].as_str().unwrap();

    let simulate_json = simulate_step(&setup, plan_id, step_id).await;
    let simulated_step = step_by_id(&simulate_json, step_id);

    assert_eq!(simulated_step["simulation_status"], "passed");
    assert!(evidence_contains(
        simulated_step,
        "defi_exit_adapter=erc4626-redeem"
    ));
    assert!(evidence_contains(
        simulated_step,
        "prepared_call=erc4626.redeem(shares,receiver,owner)"
    ));
    assert!(evidence_contains(
        simulated_step,
        "expected_assets_out_hex=0xf0000"
    ));

    drop(setup);
}

#[tokio::test]
async fn defi_erc4626_preflight_revert_blocks_step() {
    let mut config = default_rpc_config("0xde0b6b3a7640000");
    config.redeem_reverts = true;
    let setup = setup_daemon(config).await;

    scan_erc4626_probe(&setup, GOOD_VAULT).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("erc4626-redeem"));
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let step_id = step["id"].as_str().unwrap();

    let simulate_json = simulate_step(&setup, plan_id, step_id).await;
    let simulated_step = step_by_id(&simulate_json, step_id);

    assert_eq!(simulated_step["simulation_status"], "failed");
    assert_eq!(simulated_step["status"], "blocked");
    assert!(blockers_contain(simulated_step, "simulation_failed"));
    assert!(evidence_contains_prefix(simulated_step, "eth_call_error="));

    drop(setup);
}

#[tokio::test]
async fn defi_erc4626_gas_shortfall_blocks_step() {
    let setup = setup_daemon(default_rpc_config("0x1")).await;

    scan_erc4626_probe(&setup, GOOD_VAULT).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("erc4626-redeem"));
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let step_id = step["id"].as_str().unwrap();

    let simulate_json = simulate_step(&setup, plan_id, step_id).await;
    let simulated_step = step_by_id(&simulate_json, step_id);

    assert_eq!(simulated_step["simulation_status"], "blocked");
    assert_eq!(simulated_step["status"], "blocked");
    assert!(blockers_contain(simulated_step, "simulation_blocked"));
    assert!(evidence_contains(
        simulated_step,
        "gas_policy_blocker=insufficient_native_gas"
    ));

    drop(setup);
}
