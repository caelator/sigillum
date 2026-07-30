mod common;

use common::{post_json, spawn_daemon};
use std::net::SocketAddr;
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
const WSTETH_TOKEN: &str = "0xdead4d57e7570000000000000000000000000000";
const STETH_TOKEN: &str = "0xdead57e7480000000000000000000000000000aa";
const UNIV2_PAIR: &str = "0xdeadfa1200000000000000000000000000000aaa";
const UNIV2_TOKEN0: &str = "0xdead70c0000000000000000000000000000000aa";
const UNIV2_TOKEN1: &str = "0xdead70c1000000000000000000000000000000bb";
const UNIV2_ROUTER: &str = "0xdead100e2000000000000000000000000000cccc";
const UNIV2_CHAIN_ID: u64 = 7777;

#[derive(Clone, Debug)]
struct RpcConfig {
    native_balance_hex: String,
    redeem_reverts: bool,
    unwrap_reverts: bool,
    univ2_approve_reverts: bool,
    univ2_remove_reverts: bool,
    chain_id: u64,
    provider_profile: String,
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
    provider_profile: String,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.daemon_handle.abort();
        self.rpc_handle.abort();
    }
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
        "eth_chainId" => json!(format!("0x{:x}", config.chain_id)),
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

    if (to == GOOD_VAULT || to == NON_4626_TOKEN || to == WSTETH_TOKEN || to == STETH_TOKEN)
        && data.starts_with("0x70a08231")
    {
        return json_rpc_result(request, json!(abi_word(0xf4240)));
    }
    if to == UNIV2_PAIR && data.starts_with("0x70a08231") {
        return json_rpc_result(request, json!(abi_word(0xf4240)));
    }
    if to == UNIV2_PAIR && data.starts_with("0x0dfe1681") {
        return json_rpc_result(request, json!(abi_address_word(UNIV2_TOKEN0)));
    }
    if to == UNIV2_PAIR && data.starts_with("0xd21220a7") {
        return json_rpc_result(request, json!(abi_address_word(UNIV2_TOKEN1)));
    }
    if to == UNIV2_PAIR && data.starts_with("0x0902f1ac") {
        return json_rpc_result(
            request,
            json!(format!(
                "{}{}{}",
                abi_word_without_prefix(0x2faf080),
                abi_word_without_prefix(0x5f5e100),
                abi_word_without_prefix(0x1)
            )),
        );
    }
    if to == UNIV2_PAIR && data.starts_with("0x18160ddd") {
        return json_rpc_result(request, json!(abi_word(0x1e84800)));
    }
    if to == UNIV2_PAIR && data.starts_with("0x095ea7b3") {
        if config.univ2_approve_reverts {
            return json_rpc_error(request, 3, "execution reverted");
        }
        return json_rpc_result(request, json!(abi_word(1)));
    }
    if to == UNIV2_ROUTER && data.starts_with("0xbaa2abde") {
        if config.univ2_remove_reverts {
            return json_rpc_error(request, 3, "execution reverted");
        }
        return json_rpc_result(
            request,
            json!(format!(
                "{}{}{}",
                "0x",
                abi_word_without_prefix(0x16e360),
                abi_word_without_prefix(0x2dc6c0)
            )),
        );
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

    if to == WSTETH_TOKEN && data.starts_with("0xde0e9a3e") {
        if config.unwrap_reverts {
            return json_rpc_error(request, 3, "execution reverted");
        }
        return json_rpc_result(request, json!(abi_word(0xf5000)));
    }

    if to == NON_4626_TOKEN && (data.starts_with("0xd905777e") || data.starts_with("0x07a2d13a")) {
        return json_rpc_error(request, 3, "execution reverted");
    }

    json_rpc_result(request, json!(abi_word(0)))
}

fn abi_word(value: u64) -> String {
    format!("0x{value:064x}")
}

fn abi_word_without_prefix(value: u64) -> String {
    format!("{value:064x}")
}

fn abi_address_word(address: &str) -> String {
    let raw = address.strip_prefix("0x").unwrap_or(address);
    format!("0x{}{}", "0".repeat(24), raw.to_ascii_lowercase())
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

async fn setup_daemon(config: RpcConfig) -> TestDaemon {
    let dir = TempDir::new().unwrap();
    let (addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let provider_profile = config.provider_profile.clone();
    let chain_id = config.chain_id;
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
            "name": provider_profile.clone(),
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": chain_id,
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
            "provider_profile": provider_profile.clone(),
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
        provider_profile,
    }
}

fn default_rpc_config(native_balance_hex: &str) -> RpcConfig {
    RpcConfig {
        native_balance_hex: native_balance_hex.into(),
        redeem_reverts: false,
        unwrap_reverts: false,
        univ2_approve_reverts: false,
        univ2_remove_reverts: false,
        chain_id: 1,
        provider_profile: "mainnet".into(),
    }
}

fn univ2_rpc_config(native_balance_hex: &str) -> RpcConfig {
    RpcConfig {
        chain_id: UNIV2_CHAIN_ID,
        provider_profile: "l2".into(),
        ..default_rpc_config(native_balance_hex)
    }
}

async fn scan_defi_token_probe(setup: &TestDaemon, protocol: &str, token_address: &str) -> Value {
    let scan = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": setup.provider_profile,
            "gap_limit": 1,
            "max_index": 0,
            "discover_defi_token_positions": true,
            "defi_token_probes": [{
                "protocol": protocol,
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

async fn scan_erc4626_probe(setup: &TestDaemon, token_address: &str) -> Value {
    scan_defi_token_probe(setup, "erc4626", token_address).await
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
    simulate_steps(setup, plan_id, &[step_id]).await
}

async fn simulate_steps(setup: &TestDaemon, plan_id: &str, step_ids: &[&str]) -> Value {
    let simulate = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/simulate",
        json!({
            "plan_id": plan_id,
            "step_ids": step_ids,
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

async fn upsert_univ2_chain_profile(setup: &TestDaemon) -> Value {
    let upsert = post_json(
        &setup.client,
        setup.addr,
        "/api/chains/upsert",
        json!({
            "name": "custom-l2",
            "chain_family": "evm",
            "chain_id": UNIV2_CHAIN_ID,
            "provider_profile": setup.provider_profile,
            "native_symbol": "ETH",
            "native_decimals": 18,
            "finality_blocks": 0,
            "uniswap_v2_router_address": UNIV2_ROUTER,
            "enabled": true
        }),
        Some(&setup.token),
    )
    .await;
    let upsert_status = upsert.status();
    let upsert_json: Value = upsert.json().await.unwrap();
    assert_eq!(
        upsert_status,
        StatusCode::OK,
        "chain upsert response: {upsert_json}"
    );
    upsert_json
}

async fn approve_plan(setup: &TestDaemon, plan_id: &str) -> Value {
    let approve = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": plan_id }),
        Some(&setup.token),
    )
    .await;
    let approve_status = approve.status();
    let approve_json: Value = approve.json().await.unwrap();
    assert_eq!(
        approve_status,
        StatusCode::OK,
        "approve response: {approve_json}"
    );
    approve_json
}

async fn export_plan(setup: &TestDaemon, plan_id: &str) -> Value {
    let export = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/export",
        json!({ "plan_id": plan_id }),
        Some(&setup.token),
    )
    .await;
    let export_status = export.status();
    let export_json: Value = export.json().await.unwrap();
    assert_eq!(
        export_status,
        StatusCode::OK,
        "export response: {export_json}"
    );
    export_json
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

fn plan_step_by_action<'a>(plan_json: &'a Value, action: &str, adapter: Option<&str>) -> &'a Value {
    plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| {
            step["action"] == action
                && adapter.is_none_or(|adapter| step["claim_adapter"] == adapter)
        })
        .unwrap_or_else(|| panic!("missing {action} step in {plan_json}"))
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

fn skipped_step<'a>(export_json: &'a Value, step_id: &str) -> &'a Value {
    export_json["skipped_steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["step_id"] == step_id)
        .unwrap_or_else(|| panic!("missing skipped step {step_id} in {export_json}"))
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

#[tokio::test]
async fn defi_lido_wsteth_detection_records_adapter() {
    let setup = setup_daemon(default_rpc_config("0x0")).await;

    let scan_json = scan_defi_token_probe(&setup, "lido-wsteth", WSTETH_TOKEN).await;
    let holding = defi_holding(&scan_json, WSTETH_TOKEN);

    assert_eq!(holding["asset_kind"], "defi");
    assert_eq!(holding["source"], "defi-token-probe:lido-wsteth");
    assert_eq!(holding["claim_adapter"], "lido-wsteth-unwrap");
    assert_eq!(holding["amount_hex"], "0xf4240");
    assert_eq!(holding["protocol_address"], WSTETH_TOKEN);

    drop(setup);
}

#[tokio::test]
async fn defi_lido_wsteth_preflight_pass_records_expected_steth_out() {
    let setup = setup_daemon(default_rpc_config("0xde0b6b3a7640000")).await;

    scan_defi_token_probe(&setup, "lido-wsteth", WSTETH_TOKEN).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("lido-wsteth-unwrap"));
    assert_eq!(step["status"], "review_required");
    assert_eq!(step["simulation_status"], "required");
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let step_id = step["id"].as_str().unwrap();

    let simulate_json = simulate_step(&setup, plan_id, step_id).await;
    let simulated_step = step_by_id(&simulate_json, step_id);

    assert_eq!(simulated_step["simulation_status"], "passed");
    assert!(evidence_contains(
        simulated_step,
        "defi_exit_adapter=lido-wsteth-unwrap"
    ));
    assert!(evidence_contains(
        simulated_step,
        "prepared_call=lido_wsteth.unwrap(amount)"
    ));
    assert!(evidence_contains(
        simulated_step,
        "expected_assets_out_hex=0xf5000"
    ));
    assert!(evidence_contains(
        simulated_step,
        "steth_withdrawal_queue=out_of_scope_review_asset"
    ));

    drop(setup);
}

#[tokio::test]
async fn defi_lido_wsteth_preflight_revert_blocks_step() {
    let mut config = default_rpc_config("0xde0b6b3a7640000");
    config.unwrap_reverts = true;
    let setup = setup_daemon(config).await;

    scan_defi_token_probe(&setup, "lido-wsteth", WSTETH_TOKEN).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("lido-wsteth-unwrap"));
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
async fn defi_lido_wsteth_gas_shortfall_blocks_step() {
    let setup = setup_daemon(default_rpc_config("0x1")).await;

    scan_defi_token_probe(&setup, "lido-wsteth", WSTETH_TOKEN).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("lido-wsteth-unwrap"));
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

#[tokio::test]
async fn defi_lido_steth_position_stays_review_fallback() {
    let setup = setup_daemon(default_rpc_config("0xde0b6b3a7640000")).await;

    let scan_json = scan_defi_token_probe(&setup, "lido-steth", STETH_TOKEN).await;
    let holding = defi_holding(&scan_json, STETH_TOKEN);
    assert_eq!(holding["asset_kind"], "defi");
    assert_eq!(holding["source"], "defi-token-probe:lido-steth");
    assert!(holding.get("claim_adapter").is_none_or(Value::is_null));

    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, None);

    assert_eq!(step["status"], "blocked");
    assert!(blockers_contain(step, "requires_protocol_adapter"));

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_detection_records_adapter() {
    let setup = setup_daemon(univ2_rpc_config("0x0")).await;

    let scan_json = scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let holding = defi_holding(&scan_json, UNIV2_PAIR);

    assert_eq!(holding["asset_kind"], "defi");
    assert_eq!(holding["source"], "defi-token-probe:uniswap-v2");
    assert_eq!(holding["claim_adapter"], "uniswap-v2-remove-liquidity");
    assert_eq!(holding["amount_hex"], "0xf4240");
    assert_eq!(holding["protocol_address"], UNIV2_PAIR);

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_missing_router_blocks_position() {
    let setup = setup_daemon(univ2_rpc_config("0xde0b6b3a7640000")).await;

    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let step = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"));
    let univ2_steps = plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["claim_adapter"] == "uniswap-v2-remove-liquidity")
        .filter(|step| step["address"] == OWNER_ADDRESS)
        .count();

    assert_eq!(univ2_steps, 1);
    assert_eq!(step["status"], "blocked");
    assert!(blockers_contain(step, "missing_uniswap_v2_router"));

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_plan_expands_two_dependency_ordered_steps() {
    let setup = setup_daemon(univ2_rpc_config("0xde0b6b3a7640000")).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let approve = plan_step_by_action(
        &plan_json,
        "approve_erc20",
        Some("uniswap-v2-remove-liquidity"),
    );
    let remove = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"));
    let approve_id = approve["id"].as_str().unwrap();
    let univ2_steps = plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["claim_adapter"] == "uniswap-v2-remove-liquidity")
        .filter(|step| step["address"] == OWNER_ADDRESS)
        .count();

    assert_eq!(univ2_steps, 2);
    assert_eq!(approve["counterparty_address"], UNIV2_ROUTER);
    assert_eq!(approve["claim_adapter"], "uniswap-v2-remove-liquidity");
    assert!(
        approve
            .get("depends_on")
            .and_then(Value::as_array)
            .is_none_or(|depends_on| depends_on.is_empty())
    );
    assert_eq!(remove["protocol_address"], UNIV2_ROUTER);
    assert_eq!(remove["exit_token0_address"], UNIV2_TOKEN0);
    assert_eq!(remove["exit_token1_address"], UNIV2_TOKEN1);
    assert_eq!(remove["exit_amount0_min_hex"], "0x17b8ff");
    assert_eq!(remove["exit_amount1_min_hex"], "0x2f71ff");
    assert!(remove["exit_deadline_unix"].as_u64().unwrap() > 0);
    assert_eq!(remove["depends_on"], json!([approve_id]));
    assert!(remove["sequence"].as_u64().unwrap() > approve["sequence"].as_u64().unwrap());

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_preflight_pass_records_expected_amounts() {
    let setup = setup_daemon(univ2_rpc_config("0xde0b6b3a7640000")).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let approve_id = plan_step_by_action(
        &plan_json,
        "approve_erc20",
        Some("uniswap-v2-remove-liquidity"),
    )["id"]
        .as_str()
        .unwrap();
    let remove_id = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"))["id"]
        .as_str()
        .unwrap();

    let simulate_json = simulate_steps(&setup, plan_id, &[approve_id, remove_id]).await;
    let simulated_approve = step_by_id(&simulate_json, approve_id);
    let simulated_remove = step_by_id(&simulate_json, remove_id);

    assert_eq!(simulated_approve["simulation_status"], "passed");
    assert_eq!(simulated_remove["simulation_status"], "passed");
    assert!(evidence_contains(
        simulated_remove,
        "defi_exit_adapter=uniswap-v2-remove-liquidity"
    ));
    assert!(evidence_contains(
        simulated_remove,
        "prepared_call=uniswap_v2.remove_liquidity(token0,token1,liquidity,amount0Min,amount1Min,to,deadline)"
    ));
    assert!(evidence_contains(
        simulated_remove,
        "expected_amount0_out_hex=0x16e360"
    ));
    assert!(evidence_contains(
        simulated_remove,
        "expected_amount1_out_hex=0x2dc6c0"
    ));

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_preflight_revert_blocks_step() {
    let mut config = univ2_rpc_config("0xde0b6b3a7640000");
    config.univ2_remove_reverts = true;
    let setup = setup_daemon(config).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let remove_id = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"))["id"]
        .as_str()
        .unwrap();

    let simulate_json = simulate_step(&setup, plan_id, remove_id).await;
    let simulated_remove = step_by_id(&simulate_json, remove_id);

    assert_eq!(simulated_remove["simulation_status"], "failed");
    assert_eq!(simulated_remove["status"], "blocked");
    assert!(blockers_contain(simulated_remove, "simulation_failed"));

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_gas_shortfall_blocks_step() {
    let setup = setup_daemon(univ2_rpc_config("0x1")).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let approve_id = plan_step_by_action(
        &plan_json,
        "approve_erc20",
        Some("uniswap-v2-remove-liquidity"),
    )["id"]
        .as_str()
        .unwrap();

    let simulate_json = simulate_step(&setup, plan_id, approve_id).await;
    let simulated_approve = step_by_id(&simulate_json, approve_id);

    assert_eq!(simulated_approve["simulation_status"], "blocked");
    assert_eq!(simulated_approve["status"], "blocked");
    assert!(evidence_contains(
        simulated_approve,
        "gas_policy_blocker=insufficient_native_gas"
    ));

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_export_orders_approve_before_remove() {
    let setup = setup_daemon(univ2_rpc_config("0xde0b6b3a7640000")).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let approve_id = plan_step_by_action(
        &plan_json,
        "approve_erc20",
        Some("uniswap-v2-remove-liquidity"),
    )["id"]
        .as_str()
        .unwrap();
    let remove_id = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"))["id"]
        .as_str()
        .unwrap();

    simulate_steps(&setup, plan_id, &[approve_id, remove_id]).await;
    approve_plan(&setup, plan_id).await;
    let export_json = export_plan(&setup, plan_id).await;
    let calls = export_json["bundles"][0]["calls"].as_array().unwrap();

    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["step_id"], approve_id);
    assert_eq!(calls[1]["step_id"], remove_id);
    assert!(
        calls[0]["data_hex"]
            .as_str()
            .unwrap()
            .starts_with("0x095ea7b3")
    );
    assert!(
        calls[1]["data_hex"]
            .as_str()
            .unwrap()
            .starts_with("0xbaa2abde")
    );

    drop(setup);
}

#[tokio::test]
async fn defi_univ2_export_skips_dependent_of_blocked_approve() {
    let mut config = univ2_rpc_config("0xde0b6b3a7640000");
    config.univ2_approve_reverts = true;
    let setup = setup_daemon(config).await;

    upsert_univ2_chain_profile(&setup).await;
    scan_defi_token_probe(&setup, "uniswap-v2", UNIV2_PAIR).await;
    let plan_json = generate_consolidation_plan(&setup, json!({})).await;
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    let approve_id = plan_step_by_action(
        &plan_json,
        "approve_erc20",
        Some("uniswap-v2-remove-liquidity"),
    )["id"]
        .as_str()
        .unwrap();
    let remove_id = exit_defi_step(&plan_json, Some("uniswap-v2-remove-liquidity"))["id"]
        .as_str()
        .unwrap();

    let simulate_json = simulate_steps(&setup, plan_id, &[approve_id, remove_id]).await;
    let simulated_approve = step_by_id(&simulate_json, approve_id);
    let simulated_remove = step_by_id(&simulate_json, remove_id);
    assert_eq!(simulated_approve["status"], "blocked");
    assert_eq!(simulated_remove["simulation_status"], "passed");

    approve_plan(&setup, plan_id).await;
    let export_json = export_plan(&setup, plan_id).await;
    let skipped_approve = skipped_step(&export_json, approve_id);
    let skipped_remove = skipped_step(&export_json, remove_id);

    assert_eq!(skipped_approve["reason"], "blocked");
    assert_eq!(
        skipped_remove["reason"].as_str().unwrap(),
        format!("dependency_blocked:{approve_id}")
    );
    assert!(export_json["bundles"].as_array().unwrap().is_empty());

    drop(setup);
}
