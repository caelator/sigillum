mod common;

use common::mock_evm::spawn_mock_evm_provider;
use common::{get, post_json, spawn_daemon};
use std::net::SocketAddr;

use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

async fn setup_seed_inventory_for_consolidation(
    fee_estimation_enabled: Option<bool>,
) -> (
    TempDir,
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
    reqwest::Client,
    String,
) {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
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

    let mut provider = json!({
        "name": "mainnet",
        "rpc_url": format!("http://{rpc_addr}/"),
        "auth_token_key": "alchemy",
        "chain_id": 1,
        "max_priority_fee_per_gas_hex": "0x59682f00",
        "max_fee_per_gas_hex": "0x12a05f200",
        "native_gas_limit": 21000,
        "erc20_gas_limit": 65000,
    });
    if let Some(enabled) = fee_estimation_enabled {
        provider["fee_estimation_enabled"] = json!(enabled);
    }
    let provider_response = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        provider,
        Some(&token),
    )
    .await;
    assert_eq!(provider_response.status(), StatusCode::OK);

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
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");

    (dir, addr, handle, rpc_handle, client, token)
}

async fn generate_and_simulate_consolidation_plan(
    client: &reqwest::Client,
    addr: SocketAddr,
    token: &str,
) -> serde_json::Value {
    let plan = post_json(
        client,
        addr,
        "/api/plans/consolidation/generate",
        json!({
            "destination_address": "0x9999999999999999999999999999999999999999",
        }),
        Some(token),
    )
    .await;
    assert_eq!(plan.status(), StatusCode::OK);
    let plan_json: serde_json::Value = plan.json().await.unwrap();

    let simulate = post_json(
        client,
        addr,
        "/api/plans/consolidation/simulate",
        json!({ "plan_id": plan_json["plan"]["id"].as_str().unwrap() }),
        Some(token),
    )
    .await;
    let simulate_status = simulate.status();
    let simulate_json: serde_json::Value = simulate.json().await.unwrap();
    assert_eq!(
        simulate_status,
        StatusCode::OK,
        "simulate response: {simulate_json}"
    );
    simulate_json
}

fn passed_sweep_native_step(plan_json: &serde_json::Value) -> &serde_json::Value {
    plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "sweep_native" && step["simulation_status"] == "passed")
        .unwrap_or_else(|| panic!("missing passed sweep_native step in {plan_json}"))
}

fn evidence_contains(step: &serde_json::Value, expected: &str) -> bool {
    step["simulation_evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evidence| evidence == expected)
}

fn evidence_contains_prefix(step: &serde_json::Value, prefix: &str) -> bool {
    step["simulation_evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evidence| {
            evidence
                .as_str()
                .is_some_and(|evidence| evidence.starts_with(prefix))
        })
}

#[tokio::test]
async fn deposit_registry_refresh_and_sweep_flow_roundtrip() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    // Stealth deposit sweeps gate under the Sweep execution family. The
    // auto-enqueue, manual enqueue, and drains below need the master and sweep
    // gates open with the sweep destination allow-listed. Linkage blocking is
    // explicitly disabled because untagged deposits are distinct identities.
    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "block_cross_party_linkage": false,
            "allowed_destinations": [{ "address": "0x1111111111111111111111111111111111111111" }],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;

    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 8453,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
        }),
        Some(&token),
    )
    .await;

    post_json(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111",
        }),
        Some(&token),
    )
    .await;

    let native_deposit = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/create-native",
        json!({
            "wallet_profile": "payments-mainnet",
            "expected_value_wei_hex": "0x1",
            "auto_queue_sweep": true,
            "min_sweep_value_wei_hex": "0x1",
            "note": "invoice-42"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(native_deposit.status(), StatusCode::OK);
    let native_deposit_json: serde_json::Value = native_deposit.json().await.unwrap();
    assert_eq!(native_deposit_json["deposit"]["chain_id"], 8453);
    assert_eq!(native_deposit_json["deposit"]["chain_id_assumed"], false);
    let native_id = native_deposit_json["deposit"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let underfunded_deposit = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/create-native",
        json!({
            "wallet_profile": "payments-mainnet",
            "expected_value_wei_hex": "0x1bc16d674ec80000",
            "auto_queue_sweep": true,
            "min_sweep_value_wei_hex": "0x1",
            "note": "invoice-underfunded"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(underfunded_deposit.status(), StatusCode::OK);
    let underfunded_json: serde_json::Value = underfunded_deposit.json().await.unwrap();
    let underfunded_id = underfunded_json["deposit"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let underfunded_erc20_deposit = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/create-erc20",
        json!({
            "wallet_profile": "payments-mainnet",
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "expected_amount_hex": "0x1e8480",
            "auto_queue_sweep": true,
            "min_sweep_amount_hex": "0x1"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(underfunded_erc20_deposit.status(), StatusCode::OK);
    let underfunded_erc20_json: serde_json::Value = underfunded_erc20_deposit.json().await.unwrap();
    let underfunded_erc20_id = underfunded_erc20_json["deposit"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let refresh = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "auto_enqueue": true }),
        Some(&token),
    )
    .await;
    assert_eq!(refresh.status(), StatusCode::OK);
    let refresh_json: serde_json::Value = refresh.json().await.unwrap();
    assert_eq!(refresh_json["detected"], 3);
    assert_eq!(refresh_json["queued"], 1);
    let refreshed_deposits = refresh_json["deposits"].as_array().unwrap();
    let funded = refreshed_deposits
        .iter()
        .find(|deposit| deposit["id"] == native_id)
        .expect("funded deposit should be present");
    assert_eq!(funded["chain_id"], 8453);
    assert_eq!(funded["chain_id_assumed"], false);
    let underfunded = refreshed_deposits
        .iter()
        .find(|deposit| deposit["id"] == underfunded_id)
        .expect("underfunded deposit should be present");
    assert_eq!(underfunded["status"], "underfunded");
    assert_eq!(underfunded["observed_amount_hex"], "0xde0b6b3a7640000");
    assert!(underfunded["queue_job_id"].is_null());
    let underfunded_erc20 = refreshed_deposits
        .iter()
        .find(|deposit| deposit["id"] == underfunded_erc20_id)
        .expect("underfunded ERC-20 deposit should be present");
    assert_eq!(underfunded_erc20["status"], "underfunded");
    assert_eq!(underfunded_erc20["observed_amount_hex"], "0xf4240");
    assert!(underfunded_erc20["queue_job_id"].is_null());
    let sweep_job_id = funded["queue_job_id"].as_str().unwrap().to_string();

    let process = post_json(
        &client,
        addr,
        "/api/queue/process",
        json!({ "id": sweep_job_id }),
        Some(&token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: serde_json::Value = process.json().await.unwrap();
    assert_eq!(process_json["succeeded"], 1);
    assert_eq!(process_json["jobs"][0]["state"], "sent");

    let refresh_after = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "id": native_id, "auto_enqueue": false }),
        Some(&token),
    )
    .await;
    assert_eq!(refresh_after.status(), StatusCode::OK);
    let refresh_after_json: serde_json::Value = refresh_after.json().await.unwrap();
    assert_eq!(refresh_after_json["deposits"][0]["status"], "sweep_sent");
    assert_eq!(
        refresh_after_json["deposits"][0]["broadcast_transaction_hash_hex"],
        process_json["jobs"][0]["transaction_hash_hex"]
    );

    let erc20_deposit = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/create-erc20",
        json!({
            "wallet_profile": "payments-mainnet",
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "expected_amount_hex": "0xf4240",
            "auto_queue_sweep": false,
            "min_sweep_amount_hex": "0xf4240"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(erc20_deposit.status(), StatusCode::OK);
    let erc20_json: serde_json::Value = erc20_deposit.json().await.unwrap();
    assert_eq!(erc20_json["deposit"]["chain_id"], 8453);
    assert_eq!(erc20_json["deposit"]["chain_id_assumed"], false);
    let erc20_id = erc20_json["deposit"]["id"].as_str().unwrap().to_string();

    let manual_enqueue = post_json(
        &client,
        addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": erc20_id }),
        Some(&token),
    )
    .await;
    assert_eq!(manual_enqueue.status(), StatusCode::OK);
    let manual_enqueue_json: serde_json::Value = manual_enqueue.json().await.unwrap();
    assert_eq!(
        manual_enqueue_json["job"]["kind"],
        "eth_stealth_erc20_sweep"
    );

    let maintenance = post_json(
        &client,
        addr,
        "/api/maintenance/run",
        json!({
            "deposit_refresh_limit": 10,
            "queue_process_limit": 10,
            "auto_enqueue": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(maintenance.status(), StatusCode::OK);
    let maintenance_json: serde_json::Value = maintenance.json().await.unwrap();
    assert_eq!(maintenance_json["status"], "ok");
    assert_eq!(maintenance_json["processed"], 1);
    assert_eq!(maintenance_json["succeeded"], 1);
    assert_eq!(maintenance_json["failures_by_cause"]["provider_error"], 0);
    assert_eq!(maintenance_json["failures_by_cause"]["policy_block"], 0);

    let diagnostics = get(&client, addr, "/api/diagnostics", Some(&token)).await;
    assert_eq!(diagnostics.status(), StatusCode::OK);
    let diagnostics_json: serde_json::Value = diagnostics.json().await.unwrap();
    assert_eq!(diagnostics_json["queue_job_count"], 2);
    assert_eq!(diagnostics_json["eth_stealth_deposit_count"], 4);

    let deposits = get(&client, addr, "/api/deposits/eth-stealth", Some(&token)).await;
    assert_eq!(deposits.status(), StatusCode::OK);
    let deposits_json: serde_json::Value = deposits.json().await.unwrap();
    assert_eq!(deposits_json["deposits"].as_array().unwrap().len(), 4);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn treasury_policy_update_round_trips_simulation_freshness() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let update = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "simulation_freshness_secs": 120,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let update_json: serde_json::Value = update.json().await.unwrap();
    assert_eq!(
        update_json["policy"]["simulation_freshness_secs"],
        json!(120)
    );

    let read_back = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(read_back.status(), StatusCode::OK);
    let read_back_json: serde_json::Value = read_back.json().await.unwrap();
    assert_eq!(
        read_back_json["policy"]["simulation_freshness_secs"],
        json!(120)
    );

    let defaulted = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(defaulted.status(), StatusCode::OK);
    let defaulted_json: serde_json::Value = defaulted.json().await.unwrap();
    assert_eq!(
        defaulted_json["policy"]["simulation_freshness_secs"],
        json!(900)
    );

    let invalid = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "simulation_freshness_secs": 0,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    handle.abort();
}

#[tokio::test]
async fn treasury_policy_update_round_trips_hot_floor_and_target() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let update = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "0x1",
            "hot_target_wei_hex": "0x2",
            "hot_overflow_wei_hex": "0x3",
            "allow_treasury_automation": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let update_json: serde_json::Value = update.json().await.unwrap();
    assert_eq!(update_json["policy"]["hot_floor_wei_hex"], json!("0x1"));
    assert_eq!(update_json["policy"]["hot_target_wei_hex"], json!("0x2"));
    assert_eq!(update_json["policy"]["hot_overflow_wei_hex"], json!("0x3"));
    assert_eq!(
        update_json["policy"]["allow_treasury_automation"],
        json!(true)
    );

    let read_back = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(read_back.status(), StatusCode::OK);
    let read_back_json: serde_json::Value = read_back.json().await.unwrap();
    assert_eq!(read_back_json["policy"]["hot_floor_wei_hex"], json!("0x1"));
    assert_eq!(read_back_json["policy"]["hot_target_wei_hex"], json!("0x2"));
    assert_eq!(
        read_back_json["policy"]["hot_overflow_wei_hex"],
        json!("0x3")
    );
    assert_eq!(
        read_back_json["policy"]["allow_treasury_automation"],
        json!(true)
    );

    let defaulted = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(defaulted.status(), StatusCode::OK);
    let defaulted_json: serde_json::Value = defaulted.json().await.unwrap();
    assert_eq!(
        defaulted_json["policy"]["hot_floor_wei_hex"],
        json!("0xde0b6b3a7640000")
    );
    assert_eq!(
        defaulted_json["policy"]["hot_target_wei_hex"],
        json!("0xde0b6b3a7640000")
    );
    assert_eq!(
        defaulted_json["policy"]["hot_overflow_wei_hex"],
        json!(null)
    );
    assert_eq!(
        defaulted_json["policy"]["allow_treasury_automation"],
        json!(false)
    );

    let invalid_order = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "0x3",
            "hot_target_wei_hex": "0x2",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_order.status(), StatusCode::BAD_REQUEST);

    let invalid_overflow = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "0x1",
            "hot_target_wei_hex": "0x3",
            "hot_overflow_wei_hex": "0x2",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_overflow.status(), StatusCode::BAD_REQUEST);
    let invalid_overflow_body = invalid_overflow.text().await.unwrap();
    assert!(invalid_overflow_body.contains("hot_overflow_wei_hex"));

    let invalid_floor = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "not-hex",
            "hot_target_wei_hex": "0x2",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_floor.status(), StatusCode::BAD_REQUEST);

    handle.abort();
}

#[tokio::test]
async fn plan_simulation_records_estimated_fee_basis_when_provider_opts_in() {
    let (_dir, addr, handle, rpc_handle, client, token) =
        setup_seed_inventory_for_consolidation(Some(true)).await;

    let simulate_json = generate_and_simulate_consolidation_plan(&client, addr, &token).await;
    let step = passed_sweep_native_step(&simulate_json);
    assert!(evidence_contains(step, "fee_basis=estimated"));
    assert!(evidence_contains(step, "max_fee_per_gas_hex=0xd09dc300"));
    assert!(evidence_contains_prefix(step, "simulated_at_unix="));

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn plan_simulation_keeps_static_fee_basis_when_estimation_disabled() {
    let (_dir, addr, handle, rpc_handle, client, token) =
        setup_seed_inventory_for_consolidation(None).await;

    let simulate_json = generate_and_simulate_consolidation_plan(&client, addr, &token).await;
    let step = passed_sweep_native_step(&simulate_json);
    assert!(evidence_contains(step, "fee_basis=static_profile"));
    assert!(evidence_contains(step, "max_fee_per_gas_hex=0x12a05f200"));
    assert!(evidence_contains_prefix(step, "simulated_at_unix="));

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn plan_simulation_preserves_plan_level_policy_blocked_status() {
    let (dir, addr, handle, rpc_handle, client, token) =
        setup_seed_inventory_for_consolidation(None).await;
    let destination = "0x9999999999999999999999999999999999999999";

    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [{ "address": destination }],
            "max_plan_native_wei_hex": "0x1",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    let generate = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": destination }),
        Some(&token),
    )
    .await;
    let generate_status = generate.status();
    let generate_json: serde_json::Value = generate.json().await.unwrap();
    assert_eq!(
        generate_status,
        StatusCode::OK,
        "generate response: {generate_json}"
    );
    let generated_plan = &generate_json["plan"];
    let plan_id = generated_plan["id"].as_str().unwrap().to_string();
    assert_eq!(generated_plan["status"], "blocked");
    assert!(
        generated_plan["policy_violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|violation| violation == "exceeds_policy_plan_cap")
    );
    assert_eq!(generated_plan["summary"]["blocked_steps"], json!(0));
    let generated_steps = generated_plan["steps"].as_array().unwrap();
    assert!(!generated_steps.is_empty());
    assert!(generated_steps.iter().all(|step| {
        step["status"] == "review_required"
            && step["blockers"]
                .as_array()
                .is_none_or(|blockers| blockers.is_empty())
    }));

    let simulate = post_json(
        &client,
        addr,
        "/api/plans/consolidation/simulate",
        json!({ "plan_id": plan_id }),
        Some(&token),
    )
    .await;
    let simulate_status = simulate.status();
    let simulate_json: serde_json::Value = simulate.json().await.unwrap();
    assert_eq!(
        simulate_status,
        StatusCode::OK,
        "simulate response: {simulate_json}"
    );
    assert_eq!(simulate_json["status"], "simulated");
    assert_eq!(simulate_json["plan"]["status"], "blocked");
    assert_eq!(simulate_json["plan"]["summary"]["blocked_steps"], json!(0));
    assert!(
        simulate_json["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["simulation_status"] == "passed")
    );

    let envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("wallet_inventory.json")).unwrap())
            .unwrap();
    let persisted_plan = envelope["data"]["consolidation_plans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["id"] == plan_id)
        .unwrap();
    assert_eq!(persisted_plan["status"], "blocked");
    assert!(
        persisted_plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["simulation_status"] == "passed")
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn plan_approval_downgrades_stale_simulation_to_required() {
    let (dir, addr, handle, rpc_handle, client, token) =
        setup_seed_inventory_for_consolidation(None).await;

    let fresh_simulate_json = generate_and_simulate_consolidation_plan(&client, addr, &token).await;
    let fresh_plan_id = fresh_simulate_json["plan"]["id"].as_str().unwrap();
    let fresh_approve = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": fresh_plan_id }),
        Some(&token),
    )
    .await;
    let fresh_approve_status = fresh_approve.status();
    let fresh_approve_json: serde_json::Value = fresh_approve.json().await.unwrap();
    assert_eq!(
        fresh_approve_status,
        StatusCode::OK,
        "fresh approve response: {fresh_approve_json}"
    );
    let fresh_step = passed_sweep_native_step(&fresh_approve_json);
    assert_eq!(fresh_step["simulation_status"], "passed");
    assert!(
        fresh_approve_json["plan"]["summary"]["executable_steps"]
            .as_u64()
            .unwrap()
            > 0
    );

    let stale_simulate_json = generate_and_simulate_consolidation_plan(&client, addr, &token).await;
    let stale_plan_id = stale_simulate_json["plan"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let path = dir.path().join("wallet_inventory.json");
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let plans = envelope["data"]["consolidation_plans"]
        .as_array_mut()
        .unwrap();
    let plan = plans
        .iter_mut()
        .find(|plan| plan["id"] == stale_plan_id)
        .unwrap();
    for step in plan["steps"].as_array_mut().unwrap() {
        let evidence = step["simulation_evidence"].as_array_mut().unwrap();
        let simulated_at = evidence
            .iter_mut()
            .find(|evidence| {
                evidence
                    .as_str()
                    .is_some_and(|value| value.starts_with("simulated_at_unix="))
            })
            .unwrap();
        *simulated_at = json!("simulated_at_unix=1");
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&envelope).unwrap()).unwrap();

    let stale_approve = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": stale_plan_id }),
        Some(&token),
    )
    .await;
    let stale_approve_status = stale_approve.status();
    let stale_approve_json: serde_json::Value = stale_approve.json().await.unwrap();
    assert_eq!(
        stale_approve_status,
        StatusCode::OK,
        "stale approve response: {stale_approve_json}"
    );
    let stale_step = stale_approve_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "sweep_native")
        .unwrap();
    assert_eq!(stale_step["simulation_status"], "required");
    assert_eq!(
        stale_approve_json["plan"]["summary"]["executable_steps"],
        json!(0)
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn treasury_policy_routes_enforce_consolidation_guardrails() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;

    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;

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

    // No policy until an operator configures one.
    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: serde_json::Value = policy.json().await.unwrap();
    assert!(policy_json["policy"].is_null());

    // Configure the policy; the duplicate uppercase destination collapses
    // into the first normalized entry and its label is preserved.
    let update = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [
                { "address": "0x9999999999999999999999999999999999999999", "label": "cold-treasury" },
                { "address": "0X9999999999999999999999999999999999999999", "label": "duplicate" },
            ],
        }),
        Some(&token),
    )
    .await;
    let update_status = update.status();
    let update_json: serde_json::Value = update.json().await.unwrap();
    assert_eq!(
        update_status,
        StatusCode::OK,
        "policy update response: {update_json}"
    );
    assert_eq!(update_json["status"], "updated");
    assert_eq!(update_json["policy"]["enabled"], true);
    assert_eq!(
        update_json["policy"]["allowed_destinations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        update_json["policy"]["allowed_destinations"][0]["address"],
        "0x9999999999999999999999999999999999999999"
    );
    assert_eq!(
        update_json["policy"]["allowed_destinations"][0]["label"],
        "cold-treasury"
    );
    assert_eq!(update_json["policy"]["require_simulation"], true);

    let read_back = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    let read_back_json: serde_json::Value = read_back.json().await.unwrap();
    assert_eq!(read_back_json["policy"], update_json["policy"]);

    // Seed native holdings (1 ETH per discovered address from the mock RPC).
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
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert!(!scan_json["holdings"].as_array().unwrap().is_empty());

    // Non-allowlisted destination: every native sweep is policy-blocked.
    let blocked_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": "0x8888888888888888888888888888888888888888" }),
        Some(&token),
    )
    .await;
    assert_eq!(blocked_plan.status(), StatusCode::OK);
    let blocked_plan_json: serde_json::Value = blocked_plan.json().await.unwrap();
    assert_eq!(blocked_plan_json["plan"]["status"], "blocked");
    let blocked_steps = blocked_plan_json["plan"]["steps"].as_array().unwrap();
    assert!(!blocked_steps.is_empty());
    assert!(blocked_steps.iter().all(|step| {
        step["action"] == "sweep_native"
            && step["status"] == "blocked"
            && step["risk_level"] == "blocked"
            && step["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blocker| blocker == "block_destination")
    }));

    // Allowlisted destination: no policy blockers, plan stays reviewable.
    let open_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": "0x9999999999999999999999999999999999999999" }),
        Some(&token),
    )
    .await;
    assert_eq!(open_plan.status(), StatusCode::OK);
    let open_plan_json: serde_json::Value = open_plan.json().await.unwrap();
    assert_eq!(open_plan_json["plan"]["status"], "review_required");
    assert!(open_plan_json["plan"]["policy_violations"].is_null());
    let open_steps = open_plan_json["plan"]["steps"].as_array().unwrap();
    assert!(!open_steps.is_empty());
    assert!(open_steps.iter().all(|step| {
        step["status"] == "review_required"
            && step["blockers"]
                .as_array()
                .is_none_or(|blockers| blockers.is_empty())
    }));

    // Approval re-checks the CURRENT policy: rotating the allowlist after
    // plan generation blocks the previously reviewable steps.
    let rotate = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [
                { "address": "0x7777777777777777777777777777777777777777" },
            ],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(rotate.status(), StatusCode::OK);

    let approve = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": open_plan_json["plan"]["id"].as_str().unwrap() }),
        Some(&token),
    )
    .await;
    assert_eq!(approve.status(), StatusCode::OK);
    let approve_json: serde_json::Value = approve.json().await.unwrap();
    assert_eq!(approve_json["plan"]["status"], "blocked");
    assert_eq!(approve_json["plan"]["summary"]["approved_steps"], json!(0));
    let approve_steps = approve_json["plan"]["steps"].as_array().unwrap();
    assert!(approve_steps.iter().all(|step| {
        step["approved"] == false
            && step["status"] == "blocked"
            && step["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blocker| blocker == "block_destination")
    }));

    // Plan cap: reviewable native sweeps sum above the cap, so the plan is
    // blocked as a whole while individual steps stay reviewable.
    let plan_cap = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [
                { "address": "0x9999999999999999999999999999999999999999" },
            ],
            "max_plan_native_wei_hex": "0x1",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(plan_cap.status(), StatusCode::OK);

    let capped_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": "0x9999999999999999999999999999999999999999" }),
        Some(&token),
    )
    .await;
    assert_eq!(capped_plan.status(), StatusCode::OK);
    let capped_plan_json: serde_json::Value = capped_plan.json().await.unwrap();
    assert_eq!(capped_plan_json["plan"]["status"], "blocked");
    assert!(
        capped_plan_json["plan"]["policy_violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|violation| violation == "exceeds_policy_plan_cap")
    );
    assert!(
        capped_plan_json["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "review_required")
    );

    // Step cap: each native sweep above the cap is blocked individually.
    let step_cap = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [
                { "address": "0x9999999999999999999999999999999999999999" },
            ],
            "max_step_native_wei_hex": "0x1",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(step_cap.status(), StatusCode::OK);

    let step_capped_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({ "destination_address": "0x9999999999999999999999999999999999999999" }),
        Some(&token),
    )
    .await;
    assert_eq!(step_capped_plan.status(), StatusCode::OK);
    let step_capped_json: serde_json::Value = step_capped_plan.json().await.unwrap();
    assert_eq!(step_capped_json["plan"]["status"], "blocked");
    assert!(
        step_capped_json["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| {
                step["status"] == "blocked"
                    && step["blockers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|blocker| blocker == "block_step_cap")
            })
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn treasury_receive_address_routes_allocate_and_rotate() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    // The provider profile only satisfies the seed profile's routing config.
    // No mock RPC is spawned: receive allocation is pure local xpub
    // derivation and must never dial a provider.
    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:9/",
            "chain_id": 8453,
        }),
        Some(&token),
    )
    .await;

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
    let seed_json: serde_json::Value = seed.json().await.unwrap();
    let receive_xpub = seed_json["profile"]["receive_xpub"]
        .as_str()
        .unwrap()
        .to_string();
    let first_receive_address = seed_json["profile"]["first_receive_address"]
        .as_str()
        .unwrap()
        .to_string();

    // Independent expectation: derive addresses straight from the exported
    // receive xpub, outside the allocation flow.
    let expected_address = |index: u32| {
        sigillum_core::derive_ethereum_address_from_xpub(&receive_xpub, index)
            .unwrap()
            .address
    };

    let empty_list = get(
        &client,
        addr,
        "/api/treasury/receive-addresses",
        Some(&token),
    )
    .await;
    assert_eq!(empty_list.status(), StatusCode::OK);
    let empty_list_json: serde_json::Value = empty_list.json().await.unwrap();
    assert_eq!(empty_list_json["allocations"].as_array().unwrap().len(), 0);

    // First allocation takes index 0, the profile's first receive address.
    let acme = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({
            "wallet_profile": "seed-main",
            "purpose": "counterparty-acme",
            "label": "Acme invoices",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(acme.status(), StatusCode::OK);
    let acme_json: serde_json::Value = acme.json().await.unwrap();
    assert_eq!(acme_json["status"], "allocated");
    assert_eq!(acme_json["allocation"]["wallet_family"], "eth-seed");
    assert_eq!(acme_json["allocation"]["wallet_profile"], "seed-main");
    assert_eq!(acme_json["allocation"]["chain_id"], 8453);
    assert_eq!(acme_json["allocation"]["chain_id_assumed"], false);
    assert_eq!(acme_json["allocation"]["address_index"], 0);
    assert_eq!(
        acme_json["allocation"]["derivation_path"],
        "m/44'/60'/0'/0/0"
    );
    assert_eq!(
        acme_json["allocation"]["address"],
        json!(expected_address(0))
    );
    assert_eq!(
        acme_json["allocation"]["address"],
        json!(first_receive_address)
    );
    // Known BIP-44 vector for the all-abandon test mnemonic at m/44'/60'/0'/0/0.
    assert!(
        acme_json["allocation"]["address"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("0x9858effd232b4033e47d90003d41ec34ecaeda94")
    );
    assert_eq!(acme_json["allocation"]["purpose"], "counterparty-acme");
    assert_eq!(acme_json["allocation"]["label"], "Acme invoices");
    assert_eq!(acme_json["allocation"]["status"], "active");
    assert!(acme_json["allocation"]["retired_at_unix"].is_null());
    let acme_id = acme_json["allocation"]["id"].as_str().unwrap().to_string();

    // A second purpose advances to the next fresh index.
    let beta = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({
            "wallet_profile": "seed-main",
            "purpose": "counterparty-beta",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(beta.status(), StatusCode::OK);
    let beta_json: serde_json::Value = beta.json().await.unwrap();
    assert_eq!(beta_json["allocation"]["address_index"], 1);
    assert_eq!(beta_json["allocation"]["chain_id"], 8453);
    assert_eq!(beta_json["allocation"]["chain_id_assumed"], false);
    assert_eq!(
        beta_json["allocation"]["derivation_path"],
        "m/44'/60'/0'/0/1"
    );
    assert_eq!(
        beta_json["allocation"]["address"],
        json!(expected_address(1))
    );
    assert!(beta_json["allocation"]["label"].is_null());

    // Rotation retires the acme allocation and issues the next index with
    // the same purpose and label.
    let rotate = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/rotate",
        json!({ "allocation_id": acme_id }),
        Some(&token),
    )
    .await;
    assert_eq!(rotate.status(), StatusCode::OK);
    let rotate_json: serde_json::Value = rotate.json().await.unwrap();
    assert_eq!(rotate_json["status"], "rotated");
    assert_eq!(rotate_json["allocation"]["address_index"], 2);
    assert_eq!(rotate_json["allocation"]["chain_id"], 8453);
    assert_eq!(rotate_json["allocation"]["chain_id_assumed"], false);
    assert_eq!(
        rotate_json["allocation"]["address"],
        json!(expected_address(2))
    );
    assert_eq!(rotate_json["allocation"]["purpose"], "counterparty-acme");
    assert_eq!(rotate_json["allocation"]["label"], "Acme invoices");
    assert_eq!(rotate_json["allocation"]["status"], "active");
    assert_ne!(rotate_json["allocation"]["id"], json!(acme_id.clone()));

    // The list keeps the retired allocation alongside both active ones.
    let list = get(
        &client,
        addr,
        "/api/treasury/receive-addresses",
        Some(&token),
    )
    .await;
    let list_json: serde_json::Value = list.json().await.unwrap();
    let allocations = list_json["allocations"].as_array().unwrap();
    assert_eq!(allocations.len(), 3);
    assert_eq!(allocations[0]["id"], json!(acme_id.clone()));
    assert_eq!(allocations[0]["status"], "retired");
    assert_eq!(allocations[0]["chain_id"], 8453);
    assert_eq!(allocations[0]["chain_id_assumed"], false);
    assert!(allocations[0]["retired_at_unix"].is_u64());
    assert_eq!(allocations[1]["status"], "active");
    assert_eq!(allocations[2]["status"], "active");

    // The console overview reports the receive rollup.
    let overview = get(&client, addr, "/api/treasury/overview", Some(&token)).await;
    assert_eq!(overview.status(), StatusCode::OK);
    let overview_json: serde_json::Value = overview.json().await.unwrap();
    assert_eq!(overview_json["receive"]["active_allocations"], 2);
    assert_eq!(overview_json["receive"]["retired_allocations"], 1);
    assert_eq!(overview_json["receive"]["purposes"], 2);

    // Unknown wallet profile is a 404.
    let unknown_profile = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({ "wallet_profile": "missing", "purpose": "x" }),
        Some(&token),
    )
    .await;
    assert_eq!(unknown_profile.status(), StatusCode::NOT_FOUND);

    // Unknown allocation id is a 404.
    let unknown_allocation = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/rotate",
        json!({ "allocation_id": "missing" }),
        Some(&token),
    )
    .await;
    assert_eq!(unknown_allocation.status(), StatusCode::NOT_FOUND);

    // Rotating an already-retired allocation is a 400.
    let retired_again = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/rotate",
        json!({ "allocation_id": acme_id }),
        Some(&token),
    )
    .await;
    assert_eq!(retired_again.status(), StatusCode::BAD_REQUEST);

    // Whitespace-only purpose fails request validation with a 400.
    let empty_purpose = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({ "wallet_profile": "seed-main", "purpose": "   " }),
        Some(&token),
    )
    .await;
    assert_eq!(empty_purpose.status(), StatusCode::BAD_REQUEST);

    handle.abort();
}

#[tokio::test]
async fn self_check_verifies_providers_wallets_policy_and_allocations() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
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
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    // Self-check is session-gated.
    let unauthorized = post_json(&client, addr, "/api/selfcheck/run", json!({}), None).await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;

    // One healthy provider on the mock RPC, one pointing at a dead port.
    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "deadnet",
            "rpc_url": "http://127.0.0.1:9/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;

    let test_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let seed = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-main",
            "label": "Seed main",
            "mnemonic": test_mnemonic,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed.status(), StatusCode::OK);

    let account_xpub =
        sigillum_core::derive_ethereum_account_xpub_from_mnemonic(test_mnemonic, None, 0).unwrap();
    let account_xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "account-xpub",
            "project_account": 0,
            "provider_profile": "mainnet",
            "external_account_xpub": account_xpub,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(account_xpub_profile.status(), StatusCode::OK);

    let custom_xpub_export =
        sigillum_core::derive_ethereum_xpub_control_branch_from_mnemonic(test_mnemonic, None, 0)
            .unwrap();
    let custom_xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "custom-xpub",
            "project_account": 99,
            "provider_profile": "mainnet",
            "external_receive_xpub": custom_xpub_export.receive_xpub.clone(),
            "external_receive_path": custom_xpub_export.receive_path.clone(),
        }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_xpub_profile.status(), StatusCode::OK);

    // Enabled policy with an empty allowlist must surface as a warning.
    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({ "enabled": true, "allowed_destinations": [] }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    let allocate = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({ "wallet_profile": "seed-main", "purpose": "counterparty-acme" }),
        Some(&token),
    )
    .await;
    assert_eq!(allocate.status(), StatusCode::OK);
    let allocate_json: serde_json::Value = allocate.json().await.unwrap();
    let allocation_id = allocate_json["allocation"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let account_allocate = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({ "wallet_profile": "account-xpub", "purpose": "external-watch" }),
        Some(&token),
    )
    .await;
    assert_eq!(account_allocate.status(), StatusCode::OK);
    let account_allocate_json: serde_json::Value = account_allocate.json().await.unwrap();
    let account_allocation_id = account_allocate_json["allocation"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let custom_allocate = post_json(
        &client,
        addr,
        "/api/treasury/receive-addresses/allocate",
        json!({ "wallet_profile": "custom-xpub", "purpose": "custom-path" }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_allocate.status(), StatusCode::OK);
    let custom_allocate_json: serde_json::Value = custom_allocate.json().await.unwrap();
    let custom_allocation_id = custom_allocate_json["allocation"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        custom_allocate_json["allocation"]["derivation_path"],
        format!("{}/0", custom_xpub_export.receive_path)
    );

    // Unknown domains are rejected before any checks run.
    let unknown = post_json(
        &client,
        addr,
        "/api/selfcheck/run",
        json!({ "domains": ["bogus"] }),
        Some(&token),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);

    // Empty body = all domains; aggregate fails because deadnet is down.
    let run = post_json(&client, addr, "/api/selfcheck/run", json!({}), Some(&token)).await;
    assert_eq!(run.status(), StatusCode::OK);
    let run_json: serde_json::Value = run.json().await.unwrap();
    assert_eq!(run_json["status"], "fail");
    assert!(run_json["generated_at_unix"].as_u64().unwrap() > 0);
    let checks = run_json["checks"].as_array().unwrap();

    let find = |id: &str| {
        checks
            .iter()
            .find(|check| check["id"] == id)
            .unwrap_or_else(|| panic!("missing check {id} in {checks:?}"))
    };

    let mainnet = find("provider:mainnet");
    assert_eq!(mainnet["status"], "pass", "mainnet check: {mainnet}");
    assert!(mainnet["latency_ms"].is_u64());

    let deadnet = find("provider:deadnet");
    assert_eq!(deadnet["status"], "fail");
    assert!(
        deadnet["detail"]
            .as_str()
            .unwrap()
            .starts_with("RPC unreachable:")
    );
    // Unreachable probes have no latency and the field is skipped entirely.
    assert!(deadnet.get("latency_ms").is_none());

    let seed_check = find("seed-wallet:seed-main");
    assert_eq!(seed_check["status"], "pass", "seed check: {seed_check}");
    assert_eq!(seed_check["domain"], "seed-wallet");
    assert_eq!(seed_check["subject"], "seed-main");

    let account_xpub_check = find("xpub-wallet:account-xpub");
    assert_eq!(
        account_xpub_check["status"], "pass",
        "account xpub check: {account_xpub_check}"
    );
    let custom_xpub_check = find("xpub-wallet:custom-xpub");
    assert_eq!(
        custom_xpub_check["status"], "warn",
        "custom xpub check: {custom_xpub_check}"
    );
    assert!(
        custom_xpub_check["detail"]
            .as_str()
            .unwrap()
            .contains("external xpub path is operator-asserted metadata")
    );

    let policy_check = find("policy:treasury");
    assert_eq!(policy_check["status"], "warn");
    assert_eq!(
        policy_check["detail"],
        "Enabled policy with empty allowlist blocks every routed sweep"
    );

    let allocation_check = find(&format!("receive-allocation:{allocation_id}"));
    assert_eq!(allocation_check["status"], "pass");
    let account_allocation_check = find(&format!("receive-allocation:{account_allocation_id}"));
    assert_eq!(account_allocation_check["status"], "pass");
    let custom_allocation_check = find(&format!("receive-allocation:{custom_allocation_id}"));
    assert_eq!(custom_allocation_check["status"], "pass");

    // Unconfigured domains contribute no results.
    assert!(checks.iter().all(|check| {
        !["stealth-wallet", "watch-book", "fido2"].contains(&check["domain"].as_str().unwrap())
    }));

    // Domain filtering only runs the requested checks.
    let filtered = post_json(
        &client,
        addr,
        "/api/selfcheck/run",
        json!({ "domains": ["policy"] }),
        Some(&token),
    )
    .await;
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered_json: serde_json::Value = filtered.json().await.unwrap();
    assert_eq!(filtered_json["status"], "warn");
    let filtered_checks = filtered_json["checks"].as_array().unwrap();
    assert_eq!(filtered_checks.len(), 1);
    assert_eq!(filtered_checks[0]["id"], "policy:treasury");

    // Deleting the wallet profile orphans its allocation.
    let delete = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/delete",
        json!({ "name": "seed-main" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete.status(), StatusCode::OK);

    let orphaned = post_json(
        &client,
        addr,
        "/api/selfcheck/run",
        json!({ "domains": ["receive-allocation"] }),
        Some(&token),
    )
    .await;
    assert_eq!(orphaned.status(), StatusCode::OK);
    let orphaned_json: serde_json::Value = orphaned.json().await.unwrap();
    assert_eq!(orphaned_json["status"], "fail");
    let orphaned_checks = orphaned_json["checks"].as_array().unwrap();
    assert_eq!(orphaned_checks.len(), 3);
    let orphaned_seed = orphaned_checks
        .iter()
        .find(|check| check["id"] == format!("receive-allocation:{allocation_id}"))
        .unwrap();
    assert_eq!(orphaned_seed["status"], "fail");
    assert_eq!(
        orphaned_seed["detail"],
        "Orphaned allocation — wallet profile deleted"
    );
    let remaining_account_xpub = orphaned_checks
        .iter()
        .find(|check| check["id"] == format!("receive-allocation:{account_allocation_id}"))
        .unwrap();
    assert_eq!(remaining_account_xpub["status"], "pass");
    let remaining_custom_xpub = orphaned_checks
        .iter()
        .find(|check| check["id"] == format!("receive-allocation:{custom_allocation_id}"))
        .unwrap();
    assert_eq!(remaining_custom_xpub["status"], "pass");

    handle.abort();
    rpc_handle.abort();
}
