mod common;

use common::{post_json, spawn_daemon};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const PROVIDER_PROFILE: &str = "mainnet";
const DESTINATION_ADDRESS: &str = "0x9999999999999999999999999999999999999999";
const PARTY_A_DESTINATION: &str = "0x22222222222222222222222222222222222200aA";
const PARTY_B_DESTINATION: &str = "0x33333333333333333333333333333333333300bB";
const SPONSOR_BALANCE_HEX: &str = "0xde0b6b3a7640000";
const SOURCE_DUST_HEX: &str = "0x1";
const MAX_FEE_PER_GAS: u128 = 0x7735_9400;
const NATIVE_GAS_LIMIT: u128 = 21_000;

type BalanceMap = Arc<RwLock<BTreeMap<String, String>>>;

#[derive(Clone, Debug)]
struct RpcConfig {
    balances: BalanceMap,
    chain_id: u64,
}

#[derive(Clone)]
struct RpcState {
    config: RpcConfig,
}

struct TestDaemon {
    _dir: TempDir,
    addr: SocketAddr,
    daemon_handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    token: String,
    balances: BalanceMap,
    provider_profile: String,
    first_receive_address: String,
    sponsor_address: String,
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
        .with_state(RpcState { config });
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
    let result = match method {
        "eth_chainId" => json!(format!("0x{:x}", config.chain_id)),
        "eth_blockNumber" => json!("0x20"),
        "eth_getTransactionCount" => json!("0x0"),
        "eth_getBalance" => {
            let address = request["params"][0]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let balance = config
                .balances
                .read()
                .unwrap()
                .get(&address)
                .cloned()
                .unwrap_or_else(|| "0x0".to_string());
            json!(balance)
        }
        "eth_call" => json!("0x"),
        "eth_getLogs" => json!([]),
        "eth_feeHistory" => json!({
            "oldestBlock": "0x20",
            "baseFeePerGas": ["0x1", "0x1"],
            "gasUsedRatio": [0.0],
            "reward": [["0x1"]]
        }),
        "eth_maxPriorityFeePerGas" => json!("0x1"),
        other => json!({ "unsupported": other }),
    };
    json_rpc_result(request, result)
}

fn json_rpc_result(request: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or_else(|| json!(1)),
        "result": result,
    })
}

async fn setup_daemon() -> TestDaemon {
    let dir = TempDir::new().unwrap();
    let (addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let balances = Arc::new(RwLock::new(BTreeMap::new()));
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider(RpcConfig {
        balances: Arc::clone(&balances),
        chain_id: 1,
    })
    .await;
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
            "name": PROVIDER_PROFILE,
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
            "fee_estimation_enabled": false,
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
            "mnemonic": MNEMONIC,
            "project_account": 0,
            "provider_profile": PROVIDER_PROFILE,
        }),
        Some(&token),
    )
    .await;
    let seed_status = seed.status();
    let seed_json: Value = seed.json().await.unwrap();
    assert_eq!(seed_status, StatusCode::OK, "seed response: {seed_json}");
    let first_receive_address = seed_json["profile"]["first_receive_address"]
        .as_str()
        .unwrap()
        .to_string();
    let sponsor_address = seed_json["profile"]["sponsor_address"]
        .as_str()
        .unwrap()
        .to_string();

    TestDaemon {
        _dir: dir,
        addr,
        daemon_handle,
        rpc_handle,
        client,
        token,
        balances,
        provider_profile: PROVIDER_PROFILE.into(),
        first_receive_address,
        sponsor_address,
    }
}

fn set_balance(setup: &TestDaemon, address: &str, balance_hex: &str) {
    setup
        .balances
        .write()
        .unwrap()
        .insert(address.to_ascii_lowercase(), balance_hex.to_string());
}

fn expected_topup_hex() -> String {
    let gas_cost = MAX_FEE_PER_GAS * NATIVE_GAS_LIMIT;
    format!("0x{:x}", gas_cost + gas_cost / 2)
}

fn below_expected_topup_hex() -> String {
    let gas_cost = MAX_FEE_PER_GAS * NATIVE_GAS_LIMIT;
    format!("0x{:x}", gas_cost + gas_cost / 2 - 1)
}

async fn update_treasury_policy(
    setup: &TestDaemon,
    allow_gas_topups: bool,
    block_cross_party_linkage: bool,
    max_gas_topup_wei_hex: Option<String>,
    allowed_destinations: &[&str],
) -> Value {
    let policy = post_json(
        &setup.client,
        setup.addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": allowed_destinations
                .iter()
                .map(|address| json!({ "address": address }))
                .collect::<Vec<_>>(),
            "require_simulation": true,
            "allow_raw_digest_signing": false,
            "block_cross_party_linkage": block_cross_party_linkage,
            "allow_claim_execution": false,
            "allow_gas_topups": allow_gas_topups,
            "max_gas_topup_wei_hex": max_gas_topup_wei_hex,
        }),
        Some(&setup.token),
    )
    .await;
    let policy_status = policy.status();
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(
        policy_status,
        StatusCode::OK,
        "policy response: {policy_json}"
    );
    policy_json
}

async fn scan_evm(setup: &TestDaemon, gap_limit: u32, max_index: u32) -> Value {
    let scan = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": setup.provider_profile,
            "gap_limit": gap_limit,
            "max_index": max_index,
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

async fn approve_plan(setup: &TestDaemon, plan_id: &str) -> Value {
    let approve = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/approve",
        json!({
            "plan_id": plan_id,
            "step_ids": [],
        }),
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

async fn simulate_plan(setup: &TestDaemon, plan_id: &str) -> Value {
    let simulate = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/simulate",
        json!({
            "plan_id": plan_id,
            "step_ids": [],
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

async fn export_plan(setup: &TestDaemon, plan_id: &str) -> Value {
    let export = post_json(
        &setup.client,
        setup.addr,
        "/api/plans/consolidation/export",
        json!({
            "plan_id": plan_id,
            "step_ids": [],
            "format": "call_manifest",
        }),
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

async fn create_party(setup: &TestDaemon, name: &str) -> Value {
    let party = post_json(
        &setup.client,
        setup.addr,
        "/api/treasury/parties",
        json!({ "name": name }),
        Some(&setup.token),
    )
    .await;
    let party_status = party.status();
    let party_json: Value = party.json().await.unwrap();
    assert_eq!(party_status, StatusCode::OK, "party response: {party_json}");
    party_json["party"].clone()
}

async fn allocate_receive(setup: &TestDaemon, counterparty_id: &str, purpose: &str) -> Value {
    let allocation = post_json(
        &setup.client,
        setup.addr,
        "/api/treasury/receive-addresses/allocate",
        json!({
            "wallet_profile": "seed-main",
            "purpose": purpose,
            "counterparty_id": counterparty_id,
        }),
        Some(&setup.token),
    )
    .await;
    let allocation_status = allocation.status();
    let allocation_json: Value = allocation.json().await.unwrap();
    assert_eq!(
        allocation_status,
        StatusCode::OK,
        "allocation response: {allocation_json}"
    );
    allocation_json["allocation"].clone()
}

fn plan_steps(plan_json: &Value) -> &[Value] {
    plan_json["plan"]["steps"].as_array().unwrap()
}

fn array_field<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn steps_by_action<'a>(plan_json: &'a Value, action: &str) -> Vec<&'a Value> {
    plan_steps(plan_json)
        .iter()
        .filter(|step| step["action"] == action)
        .collect()
}

fn single_step_by_action<'a>(plan_json: &'a Value, action: &str) -> &'a Value {
    let steps = steps_by_action(plan_json, action);
    assert_eq!(steps.len(), 1, "expected one {action} step in {plan_json}");
    steps[0]
}

fn blockers_contain(step: &Value, expected: &str) -> bool {
    array_field(step, "blockers")
        .iter()
        .any(|blocker| blocker == expected)
}

fn blockers_start_with(step: &Value, expected: &str) -> bool {
    array_field(step, "blockers").iter().any(|blocker| {
        blocker
            .as_str()
            .is_some_and(|value| value.starts_with(expected))
    })
}

fn evidence_contains(step: &Value, expected: &str) -> bool {
    array_field(step, "simulation_evidence")
        .iter()
        .any(|item| item == expected)
}

fn evidence_contains_prefix(step: &Value, prefix: &str) -> bool {
    array_field(step, "simulation_evidence")
        .iter()
        .any(|item| item.as_str().is_some_and(|value| value.starts_with(prefix)))
}

fn warnings_mention(step: &Value, expected: &str) -> bool {
    array_field(step, "linkage_warnings")
        .iter()
        .any(|item| item.as_str().is_some_and(|value| value.contains(expected)))
}

async fn prepare_single_source(
    allow_gas_topups: bool,
    sponsor_balance_hex: &str,
    max_gas_topup_wei_hex: Option<String>,
) -> (TestDaemon, Value) {
    let setup = setup_daemon().await;
    set_balance(&setup, &setup.first_receive_address, SOURCE_DUST_HEX);
    set_balance(&setup, &setup.sponsor_address, sponsor_balance_hex);
    scan_evm(&setup, 1, 0).await;
    update_treasury_policy(
        &setup,
        allow_gas_topups,
        false,
        max_gas_topup_wei_hex,
        &[DESTINATION_ADDRESS],
    )
    .await;
    let plan_json = generate_consolidation_plan(
        &setup,
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": setup.provider_profile,
            "destination_address": DESTINATION_ADDRESS,
        }),
    )
    .await;
    (setup, plan_json)
}

async fn assert_existing_gas_blocker_after_simulation(setup: &TestDaemon, plan_json: &Value) {
    assert!(steps_by_action(plan_json, "fund_gas").is_empty());
    let sweep = single_step_by_action(plan_json, "sweep_native");
    assert!(array_field(sweep, "depends_on").is_empty());
    assert!(!blockers_start_with(sweep, "gas_topup"));

    let plan_id = plan_json["plan"]["id"].as_str().unwrap();
    approve_plan(setup, plan_id).await;
    let simulated = simulate_plan(setup, plan_id).await;
    let sweep = single_step_by_action(&simulated, "sweep_native");
    assert_eq!(sweep["simulation_status"], "blocked");
    assert!(blockers_contain(sweep, "simulation_blocked"));
    assert!(evidence_contains(
        sweep,
        "fee_policy_blocker=insufficient_native_balance_after_gas"
    ));
}

#[tokio::test]
async fn fund_gas_emitted_for_shortfall_with_sponsor() {
    let (setup, plan_json) = prepare_single_source(true, SPONSOR_BALANCE_HEX, None).await;
    let expected_topup = expected_topup_hex();

    let fund = single_step_by_action(&plan_json, "fund_gas");
    let sweep = single_step_by_action(&plan_json, "sweep_native");

    assert_eq!(fund["status"], "review_required");
    assert!(
        fund["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&setup.sponsor_address)
    );
    assert!(
        fund["destination_address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case(&setup.first_receive_address)
    );
    assert_eq!(fund["amount_hex"], expected_topup);
    assert!(fund["sequence"].as_u64().unwrap() < sweep["sequence"].as_u64().unwrap());

    let fund_index = plan_steps(&plan_json)
        .iter()
        .position(|step| step["action"] == "fund_gas")
        .unwrap();
    let sweep_index = plan_steps(&plan_json)
        .iter()
        .position(|step| step["action"] == "sweep_native")
        .unwrap();
    assert!(fund_index < sweep_index);
    assert_eq!(
        array_field(sweep, "depends_on"),
        &[json!(fund["id"].as_str().unwrap())]
    );
}

#[tokio::test]
async fn fund_gas_cap_blocks_dependent_with_named_reason() {
    let (_setup, plan_json) =
        prepare_single_source(true, SPONSOR_BALANCE_HEX, Some(below_expected_topup_hex())).await;

    assert!(steps_by_action(&plan_json, "fund_gas").is_empty());
    let sweep = single_step_by_action(&plan_json, "sweep_native");
    assert_eq!(sweep["status"], "blocked");
    assert!(blockers_contain(
        sweep,
        "gas_topup_exceeds_cap:max_gas_topup_wei_hex"
    ));
}

#[tokio::test]
async fn no_sponsor_balance_keeps_existing_gas_blocker() {
    let (setup, plan_json) = prepare_single_source(true, "0x0", None).await;
    assert_existing_gas_blocker_after_simulation(&setup, &plan_json).await;
}

#[tokio::test]
async fn policy_off_plan_has_no_fund_gas_and_keeps_old_blocker() {
    let setup = setup_daemon().await;
    set_balance(&setup, &setup.first_receive_address, SOURCE_DUST_HEX);
    set_balance(&setup, &setup.sponsor_address, SPONSOR_BALANCE_HEX);
    scan_evm(&setup, 1, 0).await;
    let plan_json = generate_consolidation_plan(
        &setup,
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": setup.provider_profile,
            "destination_address": DESTINATION_ADDRESS,
        }),
    )
    .await;

    assert_existing_gas_blocker_after_simulation(&setup, &plan_json).await;
}

#[tokio::test]
async fn cross_party_sponsor_funding_warns_and_blocks_per_policy() {
    let warn_plan = cross_party_plan(false).await;
    let funds = steps_by_action(&warn_plan, "fund_gas");
    assert_eq!(funds.len(), 2, "warn plan: {warn_plan}");
    assert!(funds.iter().any(|step| warnings_mention(step, "Bob")));
    assert!(funds.iter().any(|step| warnings_mention(step, "Acme")));
    assert!(funds.iter().all(|step| {
        step["status"] != "blocked" && !blockers_contain(step, "cross_party_linkage")
    }));
    assert!(
        warn_plan["plan"]["linkage_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding.as_str().is_some_and(|value| {
                value.contains("Sponsor") && value.contains("funds 2 parties")
            })),
        "warn plan findings: {warn_plan}"
    );

    let block_plan = cross_party_plan(true).await;
    let funds = steps_by_action(&block_plan, "fund_gas");
    assert_eq!(funds.len(), 2, "block plan: {block_plan}");
    assert!(
        funds
            .iter()
            .all(|step| blockers_contain(step, "cross_party_linkage"))
    );
    assert!(funds.iter().all(|step| step["status"] == "blocked"));
    assert_eq!(block_plan["plan"]["status"], "blocked");
}

async fn cross_party_plan(block_cross_party_linkage: bool) -> Value {
    let setup = setup_daemon().await;
    let acme = create_party(&setup, "Acme").await;
    let bob = create_party(&setup, "Bob").await;
    let acme_allocation =
        allocate_receive(&setup, acme["id"].as_str().unwrap(), "counterparty-acme").await;
    let bob_allocation =
        allocate_receive(&setup, bob["id"].as_str().unwrap(), "counterparty-bob").await;

    set_balance(
        &setup,
        acme_allocation["address"].as_str().unwrap(),
        SOURCE_DUST_HEX,
    );
    set_balance(
        &setup,
        bob_allocation["address"].as_str().unwrap(),
        SOURCE_DUST_HEX,
    );
    set_balance(&setup, &setup.sponsor_address, SPONSOR_BALANCE_HEX);
    scan_evm(&setup, 2, 1).await;
    update_treasury_policy(
        &setup,
        true,
        block_cross_party_linkage,
        None,
        &[PARTY_A_DESTINATION, PARTY_B_DESTINATION],
    )
    .await;

    generate_consolidation_plan(
        &setup,
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": setup.provider_profile,
            "routing_strategy": "per_party",
            "party_destinations": [
                {
                    "counterparty_id": acme["id"].as_str().unwrap(),
                    "destination_address": PARTY_A_DESTINATION
                },
                {
                    "counterparty_id": bob["id"].as_str().unwrap(),
                    "destination_address": PARTY_B_DESTINATION
                }
            ],
        }),
    )
    .await
}

#[tokio::test]
async fn fund_gas_full_flow_simulates_and_exports_in_order() {
    let (setup, plan_json) = prepare_single_source(true, SPONSOR_BALANCE_HEX, None).await;
    let expected_topup = expected_topup_hex();
    let plan_id = plan_json["plan"]["id"].as_str().unwrap();

    approve_plan(&setup, plan_id).await;
    let simulated = simulate_plan(&setup, plan_id).await;
    let fund = single_step_by_action(&simulated, "fund_gas");
    let sweep = single_step_by_action(&simulated, "sweep_native");
    assert_eq!(fund["simulation_status"], "passed");
    assert_eq!(sweep["simulation_status"], "passed");
    assert!(evidence_contains(fund, "fee_basis=static_profile"));
    assert!(evidence_contains(
        fund,
        &format!("gas_topup_amount_wei_hex={expected_topup}")
    ));
    assert!(evidence_contains(
        sweep,
        &format!("pending_gas_topup_wei_hex={expected_topup}")
    ));

    let export = export_plan(&setup, plan_id).await;
    assert_eq!(export["format"], "call_manifest");
    assert_eq!(export["exported_steps"], 2);
    assert!(export["skipped_steps"].as_array().unwrap().is_empty());
    let calls = export["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|bundle| bundle["calls"].as_array().unwrap().iter())
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "export response: {export}");
    assert_eq!(calls[0]["action"], "fund_gas");
    assert_eq!(calls[1]["action"], "sweep_native");
    assert_eq!(calls[0]["data_hex"], "0x");
    assert_eq!(calls[0]["value_wei_hex"], expected_topup);
    assert!(evidence_contains_prefix(
        fund,
        "fee_basis_resolved_at_unix="
    ));
}
