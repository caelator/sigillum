mod common;

use common::mock_evm::{
    spawn_activity_mock_evm_provider, spawn_cursor_mock_evm_provider,
    spawn_erc1155_batch_mock_evm_provider, spawn_failing_mock_evm_provider,
    spawn_mock_evm_provider, spawn_slow_mock_evm_provider,
};
use common::{configure_mainnet_provider, get, init_default_compartment, post_json, spawn_daemon};
use std::collections::HashSet;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn wallet_inventory_scan_records_seed_profile_native_holdings() {
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

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": "mainnet",
            "gap_limit": 1,
            "max_index": 0
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");
    assert_eq!(scan_json["job"]["addresses_scanned"], 4);
    assert_eq!(scan_json["job"]["active_addresses"], 4);
    assert_eq!(scan_json["job"]["holdings_detected"], 4);
    assert_eq!(scan_json["addresses"].as_array().unwrap().len(), 4);
    assert_eq!(scan_json["holdings"].as_array().unwrap().len(), 4);
    let first_address_classes = scan_json["addresses"][0]["classifications"]
        .as_array()
        .unwrap();
    assert!(
        first_address_classes
            .iter()
            .any(|classification| classification == "signer_available")
    );
    assert!(
        first_address_classes
            .iter()
            .any(|classification| classification == "gas_available")
    );
    assert!(
        first_address_classes
            .iter()
            .any(|classification| classification == "transaction_history")
    );
    assert!(
        first_address_classes
            .iter()
            .any(|classification| classification == "value_detected")
    );
    assert_eq!(scan_json["holdings"][0]["asset_kind"], "native");
    assert_eq!(scan_json["holdings"][0]["amount_hex"], "0xde0b6b3a7640000");

    let list = get(&client, addr, "/api/inventory/wallets", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_json["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(list_json["addresses"].as_array().unwrap().len(), 4);
    assert_eq!(list_json["holdings"].as_array().unwrap().len(), 4);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_discovers_standard_seed_accounts() {
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
            "project_account": 7,
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
            "derivation_pattern": "standard",
            "account_limit": 2,
            "gap_limit": 1,
            "max_index": 0
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");
    assert_eq!(scan_json["job"]["addresses_scanned"], 2);
    assert_eq!(scan_json["job"]["active_addresses"], 2);
    assert_eq!(scan_json["job"]["holdings_detected"], 2);

    let addresses = scan_json["addresses"].as_array().unwrap();
    assert_eq!(addresses.len(), 2);
    assert!(addresses.iter().any(|address| {
        address["derivation_pattern"] == "standard"
            && address["account_index"] == 0
            && address["address_index"] == 0
            && address["derivation_path"] == "m/44'/60'/0'/0/0"
    }));
    assert!(addresses.iter().any(|address| {
        address["derivation_pattern"] == "standard"
            && address["account_index"] == 1
            && address["address_index"] == 0
            && address["derivation_path"] == "m/44'/60'/1'/0/0"
    }));

    let checkpoints = scan_json["job"]["checkpoints"].as_array().unwrap();
    assert_eq!(checkpoints.len(), 2);
    assert!(checkpoints.iter().any(|checkpoint| {
        checkpoint["derivation_pattern"] == "standard"
            && checkpoint["account_index"] == 0
            && checkpoint["next_index"] == 1
            && checkpoint["completed"] == true
    }));
    assert!(checkpoints.iter().any(|checkpoint| {
        checkpoint["derivation_pattern"] == "standard"
            && checkpoint["account_index"] == 1
            && checkpoint["next_index"] == 1
            && checkpoint["completed"] == true
    }));

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_rejects_invalid_seed_derivation_controls() {
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

    let invalid_pattern = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "derivation_pattern": "custom"
        }),
        Some(&token),
    )
    .await;
    let invalid_pattern_status = invalid_pattern.status();
    let invalid_pattern_json: serde_json::Value = invalid_pattern.json().await.unwrap();
    assert_eq!(
        invalid_pattern_status,
        StatusCode::BAD_REQUEST,
        "invalid pattern response: {invalid_pattern_json}"
    );

    let invalid_limit = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "account_limit": 11
        }),
        Some(&token),
    )
    .await;
    let invalid_limit_status = invalid_limit.status();
    let invalid_limit_json: serde_json::Value = invalid_limit.json().await.unwrap();
    assert_eq!(
        invalid_limit_status,
        StatusCode::BAD_REQUEST,
        "invalid account limit response: {invalid_limit_json}"
    );

    handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_records_ad_hoc_watch_addresses() {
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

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "provider_profile": "mainnet",
            "wallet_family": "eth-watch",
            "watch_addresses": [{
                "address": "0x7777777777777777777777777777777777777777",
                "label": "old-ledger"
            }]
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");
    assert_eq!(scan_json["job"]["addresses_scanned"], 1);
    assert_eq!(scan_json["job"]["active_addresses"], 1);
    assert_eq!(scan_json["job"]["holdings_detected"], 1);
    assert_eq!(scan_json["job"]["wallet_families"][0], "eth-watch");
    assert_eq!(scan_json["job"]["wallet_profiles"][0], "watch:old-ledger");
    assert_eq!(scan_json["addresses"][0]["wallet_family"], "eth-watch");
    assert_eq!(
        scan_json["addresses"][0]["wallet_profile"],
        "watch:old-ledger"
    );
    assert_eq!(
        scan_json["addresses"][0]["address"],
        "0x7777777777777777777777777777777777777777"
    );
    let classifications = scan_json["addresses"][0]["classifications"]
        .as_array()
        .unwrap();
    assert!(
        classifications
            .iter()
            .any(|classification| classification == "watch_only")
    );
    assert!(
        classifications
            .iter()
            .any(|classification| classification == "gas_available")
    );

    let list = get(&client, addr, "/api/inventory/wallets", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_json["addresses"].as_array().unwrap().len(), 1);
    assert_eq!(list_json["holdings"].as_array().unwrap().len(), 1);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_transfer_log_cursors_scan_disjoint_ranges() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, log_ranges) = spawn_cursor_mock_evm_provider().await;
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

    let scan_body = json!({
        "provider_profile": "mainnet",
        "wallet_family": "eth-watch",
        "watch_addresses": [{
            "address": "0x7777777777777777777777777777777777777777",
            "label": "old-ledger"
        }],
        "discover_erc20_transfers": true,
        "token_discovery_from_block": "0x0",
        "token_discovery_to_block": "0x9",
        "token_discovery_limit": 1
    });

    let first = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        scan_body.clone(),
        Some(&token),
    )
    .await;
    let first_status = first.status();
    let first_json: serde_json::Value = first.json().await.unwrap();
    assert_eq!(first_status, StatusCode::OK, "scan response: {first_json}");
    assert_eq!(first_json["job"]["status"], "completed");
    assert!(
        first_json["job"]["block_cursors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cursor| {
                cursor["topic_family"] == "erc20-transfer"
                    && cursor["chain_id"] == 1
                    && cursor["last_scanned_block"] == 5
            })
    );
    let first_ranges = log_ranges.lock().unwrap().clone();
    assert!(
        first_ranges
            .iter()
            .any(|range| range.from_block == "0x0" && range.to_block == "0x9"),
        "first scan ranges: {first_ranges:?}"
    );
    log_ranges.lock().unwrap().clear();

    let second = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        scan_body,
        Some(&token),
    )
    .await;
    let second_status = second.status();
    let second_json: serde_json::Value = second.json().await.unwrap();
    assert_eq!(
        second_status,
        StatusCode::OK,
        "scan response: {second_json}"
    );
    assert!(
        second_json["job"]["block_cursors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cursor| {
                cursor["topic_family"] == "erc20-transfer"
                    && cursor["chain_id"] == 1
                    && cursor["last_scanned_block"] == 9
            })
    );
    let second_ranges = log_ranges.lock().unwrap().clone();
    assert!(
        second_ranges
            .iter()
            .any(|range| range.from_block == "0x6" && range.to_block == "0x9"),
        "second scan ranges: {second_ranges:?}"
    );
    assert!(
        second_ranges.iter().all(|range| range.from_block != "0x0"),
        "second scan must not rescan the original lower bound: {second_ranges:?}"
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_live_cancel_is_prompt_terminal_and_resumable() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_slow_mock_evm_provider(Duration::from_millis(500)).await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;
    configure_mainnet_provider(&client, addr, &token, rpc_addr).await;

    let scan_client = client.clone();
    let scan_token = token.clone();
    let scan = tokio::spawn(async move {
        post_json(
            &scan_client,
            addr,
            "/api/inventory/scan/evm",
            json!({
                "provider_profile": "mainnet",
                "wallet_family": "eth-watch",
                "watch_addresses": [{
                    "address": "0x7777777777777777777777777777777777777777",
                    "label": "cancel-me"
                }]
            }),
            Some(&scan_token),
        )
        .await
    });

    let job_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let jobs = get(&client, addr, "/api/discovery/jobs", Some(&token)).await;
            if jobs.status() == StatusCode::OK {
                let jobs: serde_json::Value = jobs.json().await.unwrap();
                if let Some(id) = jobs["jobs"]
                    .as_array()
                    .and_then(|jobs| jobs.iter().find(|job| job["status"] == "running"))
                    .and_then(|job| job["id"].as_str())
                {
                    break id.to_string();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("running discovery job should become visible");

    let cancel_started = Instant::now();
    let cancel = post_json(
        &client,
        addr,
        "/api/discovery/jobs/cancel",
        json!({ "id": job_id }),
        Some(&token),
    )
    .await;
    assert!(
        cancel_started.elapsed() < Duration::from_millis(250),
        "cancel must not wait behind the running scan"
    );
    let cancel_status = cancel.status();
    let cancel_json: serde_json::Value = cancel.json().await.unwrap();
    assert_eq!(
        cancel_status,
        StatusCode::OK,
        "cancel response: {cancel_json}"
    );
    assert_eq!(cancel_json["status"], "cancel_requested");

    let scan = scan.await.unwrap();
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "canceled");
    assert!(scan_json["job"]["completed_at_unix"].is_number());
    assert!(scan_json["job"]["scan_request"].is_object());

    let resume = post_json(
        &client,
        addr,
        "/api/discovery/jobs/resume",
        json!({ "id": job_id }),
        Some(&token),
    )
    .await;
    let resume_status = resume.status();
    let resume_json: serde_json::Value = resume.json().await.unwrap();
    assert_eq!(
        resume_status,
        StatusCode::OK,
        "resume response: {resume_json}"
    );
    assert_eq!(resume_json["job"]["status"], "completed");
    assert_eq!(resume_json["job"]["resumed_from_job_id"], job_id);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_provider_error_terminalizes_running_job() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_failing_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;
    configure_mainnet_provider(&client, addr, &token, rpc_addr).await;

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "provider_profile": "mainnet",
            "wallet_family": "eth-watch",
            "watch_addresses": [{
                "address": "0x7777777777777777777777777777777777777777",
                "label": "provider-failure"
            }]
        }),
        Some(&token),
    )
    .await;
    assert!(!scan.status().is_success());

    let jobs = get(&client, addr, "/api/discovery/jobs", Some(&token)).await;
    let jobs_status = jobs.status();
    let jobs: serde_json::Value = jobs.json().await.unwrap();
    assert_eq!(jobs_status, StatusCode::OK, "jobs response: {jobs}");
    let job = jobs["jobs"]
        .as_array()
        .and_then(|jobs| jobs.last())
        .expect("failed job should remain visible");
    assert_eq!(job["status"], "failed");
    assert!(job["completed_at_unix"].is_number());
    assert!(
        job["last_error"]
            .as_str()
            .is_some_and(|message| message.contains("injected provider failure")),
        "job: {job}"
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_old_activity_classifies_dormant_candidate() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_activity_mock_evm_provider(1_100_000, 5, 5).await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;
    configure_mainnet_provider(&client, addr, &token, rpc_addr).await;

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "provider_profile": "mainnet",
            "wallet_family": "eth-watch",
            "watch_addresses": [{
                "address": "0x7777777777777777777777777777777777777777",
                "label": "old-ledger"
            }],
            "discover_erc20_transfers": true,
            "token_discovery_from_block": "0x0",
            "gap_limit": 1,
            "max_index": 0
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["addresses"][0]["last_activity_block"], 5);
    let classifications = scan_json["addresses"][0]["classifications"]
        .as_array()
        .unwrap();
    assert!(
        classifications
            .iter()
            .any(|classification| classification == "dormant_candidate")
    );

    let risks = get(&client, addr, "/api/risk/findings", Some(&token)).await;
    assert_eq!(risks.status(), StatusCode::OK);
    let risks_json: serde_json::Value = risks.json().await.unwrap();
    let findings = risks_json["findings"].as_array().unwrap();
    assert!(findings.iter().any(|finding| {
        finding["category"] == "dormant_wallet"
            && finding["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "Last observed on-chain activity block: 5")
    }));

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_recent_activity_is_not_dormant_candidate() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_activity_mock_evm_provider(1_100_000, 1_050_000, 0).await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;
    configure_mainnet_provider(&client, addr, &token, rpc_addr).await;

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "provider_profile": "mainnet",
            "wallet_family": "eth-watch",
            "watch_addresses": [{
                "address": "0x7777777777777777777777777777777777777777",
                "label": "recent-ledger"
            }],
            "discover_erc20_transfers": true,
            "token_discovery_from_block": "0x0",
            "gap_limit": 1,
            "max_index": 0
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["addresses"][0]["last_activity_block"], 1_050_000);
    let classifications = scan_json["addresses"][0]["classifications"]
        .as_array()
        .unwrap();
    assert!(
        classifications
            .iter()
            .any(|classification| classification == "value_detected")
    );
    assert!(
        !classifications
            .iter()
            .any(|classification| classification == "dormant_candidate")
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn chain_profile_dormancy_block_window_defaults_and_validates_for_chains_route() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;

    let builtins = get(&client, addr, "/api/chains", Some(&token)).await;
    assert_eq!(builtins.status(), StatusCode::OK);
    let builtins_json: serde_json::Value = builtins.json().await.unwrap();
    assert!(
        builtins_json["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|profile| {
                profile["name"] == "ethereum" && profile["dormancy_block_window"] == 1_000_000
            })
    );

    let upsert = post_json(
        &client,
        addr,
        "/api/chains/upsert",
        json!({
            "name": "activity-test-rollup",
            "chain_family": "evm",
            "chain_id": 4242,
            "dormancy_block_window": 123
        }),
        Some(&token),
    )
    .await;
    let upsert_status = upsert.status();
    let upsert_json: serde_json::Value = upsert.json().await.unwrap();
    assert_eq!(
        upsert_status,
        StatusCode::OK,
        "upsert response: {upsert_json}"
    );
    assert_eq!(upsert_json["profile"]["dormancy_block_window"], 123);

    let invalid = post_json(
        &client,
        addr,
        "/api/chains/upsert",
        json!({
            "name": "invalid-activity-rollup",
            "chain_family": "evm",
            "chain_id": 4243,
            "dormancy_block_window": 0
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    handle.abort();
}

#[tokio::test]
async fn watch_address_book_routes_feed_inventory_scans() {
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

    let upsert = post_json(
        &client,
        addr,
        "/api/inventory/watch-addresses/upsert",
        json!({
            "address": "0x8888888888888888888888888888888888888888",
            "label": "saved-ledger",
            "tags": ["archive", "ledger"],
            "enabled": true
        }),
        Some(&token),
    )
    .await;
    let upsert_status = upsert.status();
    let upsert_json: serde_json::Value = upsert.json().await.unwrap();
    assert_eq!(
        upsert_status,
        StatusCode::OK,
        "upsert response: {upsert_json}"
    );
    assert_eq!(upsert_json["entry"]["label"], "saved-ledger");

    let list = get(
        &client,
        addr,
        "/api/inventory/watch-addresses",
        Some(&token),
    )
    .await;
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_json["entries"].as_array().unwrap().len(), 1);
    assert_eq!(list_json["entries"][0]["tags"][0], "archive");

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "provider_profile": "mainnet",
            "wallet_family": "eth-watch",
            "include_watch_book": true
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["addresses_scanned"], 1);
    assert_eq!(scan_json["job"]["wallet_profiles"][0], "watch:saved-ledger");
    assert_eq!(
        scan_json["addresses"][0]["address"],
        "0x8888888888888888888888888888888888888888"
    );

    let delete = post_json(
        &client,
        addr,
        "/api/inventory/watch-addresses/delete",
        json!({ "address": "0x8888888888888888888888888888888888888888" }),
        Some(&token),
    )
    .await;
    let delete_status = delete.status();
    let delete_json: serde_json::Value = delete.json().await.unwrap();
    assert_eq!(
        delete_status,
        StatusCode::OK,
        "delete response: {delete_json}"
    );
    assert_eq!(delete_json["status"], "deleted");

    let list = get(
        &client,
        addr,
        "/api/inventory/watch-addresses",
        Some(&token),
    )
    .await;
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert!(list_json["entries"].as_array().unwrap().is_empty());

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_discovers_erc20_tokens_from_transfer_logs() {
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
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
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
            "discover_erc20_transfers": true,
            "token_discovery_from_block": "0x0",
            "token_discovery_limit": 4,
            "discover_erc20_allowances": true,
            "allowance_spender_addresses": ["0x2222222222222222222222222222222222222222"],
            "allowance_discovery_limit": 4,
            "discover_permit2_allowances": true,
            "permit2_spender_addresses": ["0x4444444444444444444444444444444444444444"],
            "permit2_allowance_limit": 8,
            "discover_erc721_transfers": true,
            "discover_erc1155_transfers": true,
            "discover_nft_operator_approvals": true,
            "nft_operator_addresses": ["0x3333333333333333333333333333333333333333"],
            "nft_operator_approval_limit": 8,
            "discover_defi_token_positions": true,
            "defi_token_probes": [{
                "protocol": "aave-v3",
                "token_address": "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8",
                "protocol_address": "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2"
            }],
            "defi_position_limit": 8,
            "discover_claim_candidates": true,
            "claim_candidate_probes": [{
                "kind": "airdrop",
                "protocol": "optimism",
                "claimant_address": "0x9858effd232b4033e47d90003d41ec34ecaeda94",
                "claim_contract_address": "0x1111111111111111111111111111111111111111",
                "asset_address": "0x4200000000000000000000000000000000000042",
                "amount_hex": "0xf4240",
                "source_label": "op-token-list",
                "claim_adapter": "merkle-distributor-v1",
                "claim_index_hex": "0x7",
                "claim_proof": [
                    format!("0x{}", "11".repeat(32)),
                    format!("0x{}", "22".repeat(32))
                ]
            }],
            "claim_candidate_limit": 4,
            "nft_discovery_from_block": "0x0",
            "nft_discovery_limit": 4
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");
    assert_eq!(scan_json["job"]["addresses_scanned"], 4);
    assert_eq!(scan_json["job"]["holdings_detected"], 22);
    assert!(
        scan_json["job"]["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|checkpoint| {
                checkpoint["wallet_family"] == "eth-seed"
                    && checkpoint["wallet_profile"] == "seed-main"
                    && checkpoint["provider_profile"] == "mainnet"
                    && checkpoint["next_index"] == 1
                    && checkpoint["completed"] == true
            })
    );
    let scan_addresses = scan_json["addresses"].as_array().unwrap();
    assert!(scan_addresses.iter().any(|address| {
        let classifications = address["classifications"].as_array().unwrap();
        classifications
            .iter()
            .any(|classification| classification == "token_holding")
            && classifications
                .iter()
                .any(|classification| classification == "nft_holding")
            && classifications
                .iter()
                .any(|classification| classification == "approval_exposure")
            && classifications
                .iter()
                .any(|classification| classification == "protocol_holding")
    }));

    let holdings = scan_json["holdings"].as_array().unwrap();
    assert_eq!(holdings.len(), 22);
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc20"
            && holding["asset_address"] == "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            && holding["amount_hex"] == "0xf4240"
            && holding["source"] == "erc20-transfer-log"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "approval"
            && holding["asset_address"] == "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            && holding["counterparty_address"] == "0x2222222222222222222222222222222222222222"
            && holding["amount_hex"] == "0xf4240"
            && holding["source"] == "erc20-allowance-probe"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "approval"
            && holding["asset_address"] == "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            && holding["counterparty_address"] == "0x4444444444444444444444444444444444444444"
            && holding["protocol_address"] == "0x000000000022d473030f116ddee9f6b43ac78ba3"
            && holding["amount_hex"] == "0xf4240"
            && holding["source"] == "permit2-allowance-probe"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc721"
            && holding["asset_address"] == "0x1234500000000000000000000000000000000000"
            && holding["token_id_hex"]
                == "0x000000000000000000000000000000000000000000000000000000000000007b"
            && holding["amount_hex"] == "0x1"
            && holding["source"] == "erc721-transfer-log"
            && holding["spam_label"] == "unverified_nft_metadata"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc1155"
            && holding["asset_address"] == "0x1155000000000000000000000000000000000000"
            && holding["token_id_hex"]
                == "0x000000000000000000000000000000000000000000000000000000000000007b"
            && holding["amount_hex"] == "0x2a"
            && holding["source"] == "erc1155-transfer-log"
            && holding["spam_label"] == "unverified_nft_metadata"
    }));
    let inventory = get(&client, addr, "/api/inventory/wallets", Some(&token)).await;
    assert_eq!(inventory.status(), StatusCode::OK);
    let inventory_json: serde_json::Value = inventory.json().await.unwrap();
    let nft_cache = inventory_json["nft_metadata_cache"].as_array().unwrap();
    assert!(nft_cache.iter().any(|entry| {
        entry["contract_address"] == "0x1234500000000000000000000000000000000000"
            && entry["token_id_hex"]
                == "0x000000000000000000000000000000000000000000000000000000000000007b"
            && entry["spam_label"] == "unverified_nft_metadata"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "approval"
            && holding["asset_address"] == "0x1234500000000000000000000000000000000000"
            && holding["counterparty_address"] == "0x3333333333333333333333333333333333333333"
            && holding["amount_hex"] == "0x1"
            && holding["source"] == "nft-operator-approval-probe"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "approval"
            && holding["asset_address"] == "0x1155000000000000000000000000000000000000"
            && holding["counterparty_address"] == "0x3333333333333333333333333333333333333333"
            && holding["amount_hex"] == "0x1"
            && holding["source"] == "nft-operator-approval-probe"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "defi"
            && holding["asset_address"] == "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8"
            && holding["protocol_address"] == "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2"
            && holding["amount_hex"] == "0xf4240"
            && holding["source"] == "defi-token-probe:aave-v3"
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "airdrop"
            && holding["asset_address"] == "0x4200000000000000000000000000000000000042"
            && holding["protocol_address"] == "0x1111111111111111111111111111111111111111"
            && holding["claim_adapter"] == "merkle-distributor-v1"
            && holding["claim_index_hex"] == "0x7"
            && holding["claim_proof"].as_array().unwrap().len() == 2
            && holding["amount_hex"] == "0xf4240"
            && holding["source"] == "claim-candidate:airdrop:optimism:op-token-list"
    }));

    let catalog_upsert = post_json(
        &client,
        addr,
        "/api/risk/catalog/upsert",
        json!({
            "address": "0x4444444444444444444444444444444444444444",
            "label": "Known malicious spender",
            "risk_level": "critical",
            "notes": ["test catalog override"],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(catalog_upsert.status(), StatusCode::OK);
    let claim_catalog_upsert = post_json(
        &client,
        addr,
        "/api/risk/catalog/upsert",
        json!({
            "address": "0x1111111111111111111111111111111111111111",
            "label": "Known OP claim contract",
            "risk_level": "trusted",
            "notes": ["trusted source still requires adapter simulation"],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(claim_catalog_upsert.status(), StatusCode::OK);
    let catalog = get(&client, addr, "/api/risk/catalog", Some(&token)).await;
    assert_eq!(catalog.status(), StatusCode::OK);
    let catalog_json: serde_json::Value = catalog.json().await.unwrap();
    assert_eq!(catalog_json["entries"].as_array().unwrap().len(), 2);

    let risks = get(&client, addr, "/api/risk/findings", Some(&token)).await;
    assert_eq!(risks.status(), StatusCode::OK);
    let risks_json: serde_json::Value = risks.json().await.unwrap();
    let findings = risks_json["findings"].as_array().unwrap();
    assert!(findings.iter().any(|finding| {
        finding["category"] == "risky_approval"
            && finding["subject"] == "0x2222222222222222222222222222222222222222"
            && finding["risk_level"] == "medium"
    }));
    assert!(findings.iter().any(|finding| {
        finding["category"] == "risky_approval"
            && finding["subject"] == "0x4444444444444444444444444444444444444444"
            && finding["risk_level"] == "critical"
            && finding["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "Risk catalog: Known malicious spender (critical)")
    }));
    assert!(findings.iter().any(|finding| {
        finding["category"] == "risky_approval"
            && finding["subject"] == "0x3333333333333333333333333333333333333333"
            && finding["risk_level"] == "high"
            && finding["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "Approval: setApprovalForAll(true)")
    }));
    assert!(findings.iter().any(|finding| {
        finding["category"] == "claim_candidate"
            && finding["subject_type"] == "claim_contract"
            && finding["subject"] == "0x1111111111111111111111111111111111111111"
            && finding["risk_level"] == "low"
            && finding["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "Risk catalog: Known OP claim contract (trusted)")
            && finding["recommendation"]
                .as_str()
                .unwrap()
                .contains("adapter verification and simulation")
    }));

    let plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({
            "destination_address": "0x9999999999999999999999999999999999999999",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(plan.status(), StatusCode::OK);
    let plan_json: serde_json::Value = plan.json().await.unwrap();
    let steps = plan_json["plan"]["steps"].as_array().unwrap();
    assert!(steps.iter().any(|step| {
        step["action"] == "revoke_erc20_approval"
            && step["status"] == "review_required"
            && step["counterparty_address"] == "0x2222222222222222222222222222222222222222"
            && step["simulation_status"] == "required"
            && step["blockers"]
                .as_array()
                .is_none_or(|blockers| blockers.is_empty())
    }));
    assert!(steps.iter().any(|step| {
        step["action"] == "revoke_permit2_allowance"
            && step["counterparty_address"] == "0x4444444444444444444444444444444444444444"
            && step["protocol_address"] == "0x000000000022d473030f116ddee9f6b43ac78ba3"
            && step["simulation_status"] == "required"
    }));
    assert!(steps.iter().any(|step| {
        step["action"] == "revoke_nft_operator_approval"
            && step["counterparty_address"] == "0x3333333333333333333333333333333333333333"
            && step["risk_level"] == "high"
    }));
    assert!(steps.iter().any(|step| {
        step["action"] == "exit_defi_position"
            && step["asset_kind"] == "defi"
            && step["asset_address"] == "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8"
            && step["protocol_address"] == "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2"
            && step["claim_adapter"] == "aave-v3-withdraw"
            && step["status"] == "review_required"
            && step["simulation_status"] == "required"
            && step["blockers"]
                .as_array()
                .is_none_or(|blockers| blockers.is_empty())
    }));
    assert!(steps.iter().any(|step| {
        step["action"] == "claim_reward"
            && step["asset_kind"] == "airdrop"
            && step["asset_address"] == "0x4200000000000000000000000000000000000042"
            && step["protocol_address"] == "0x1111111111111111111111111111111111111111"
            && step["claim_adapter"] == "merkle-distributor-v1"
            && step["claim_index_hex"] == "0x7"
            && step["claim_proof"].as_array().unwrap().len() == 2
            && step["status"] == "blocked"
            && step["simulation_status"] == "required"
            && step["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blocker| blocker == "claim_execution_disabled")
    }));

    let approve_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({
            "plan_id": plan_json["plan"]["id"].as_str().unwrap(),
        }),
        Some(&token),
    )
    .await;
    assert_eq!(approve_plan.status(), StatusCode::OK);
    let approve_plan_json: serde_json::Value = approve_plan.json().await.unwrap();
    assert_eq!(
        approve_plan_json["plan"]["summary"]["executable_steps"],
        json!(0)
    );

    let simulate_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/simulate",
        json!({
            "plan_id": plan_json["plan"]["id"].as_str().unwrap(),
        }),
        Some(&token),
    )
    .await;
    let simulate_status = simulate_plan.status();
    let simulate_json: serde_json::Value = simulate_plan.json().await.unwrap();
    assert_eq!(
        simulate_status,
        StatusCode::OK,
        "simulate response: {simulate_json}"
    );
    let simulated_steps = simulate_json["plan"]["steps"].as_array().unwrap();
    let passed_native_sweep = simulated_steps.iter().any(|step| {
        step["action"] == "sweep_native"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "prepared_call=native.transfer(value)")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "fee_policy=profile_max_fee")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "estimated_gas_cost_wei_hex=0x2632e314a000")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| {
                    evidence.as_str().is_some_and(|value| {
                        value.starts_with("native_sweep_spendable_amount_hex=0x")
                    })
                })
    });
    assert!(passed_native_sweep);
    let passed_erc20_sweep = simulated_steps.iter().any(|step| {
        step["action"] == "sweep_erc20"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "prepared_call=erc20.transfer(destination,amount)")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "gas_policy=profile_max_fee")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "estimated_gas_cost_wei_hex=0x763bfbd22000")
    });
    assert!(passed_erc20_sweep);
    let passed_erc721_sweep = simulated_steps.iter().any(|step| {
        step["action"] == "sweep_nft"
            && step["asset_kind"] == "erc721"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| {
                    evidence == "prepared_call=erc721.safeTransferFrom(owner,destination,tokenId)"
                })
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "transaction_gas_limit=100000")
    });
    assert!(passed_erc721_sweep);
    let passed_erc1155_sweep = simulated_steps.iter().any(|step| {
        step["action"] == "sweep_nft"
            && step["asset_kind"] == "erc1155"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| {
                    evidence
                        == "prepared_call=erc1155.safeTransferFrom(owner,destination,tokenId,amount,empty)"
                })
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "transaction_gas_limit=100000")
    });
    assert!(passed_erc1155_sweep);
    let passed_erc20_revoke = simulated_steps.iter().any(|step| {
        step["action"] == "revoke_erc20_approval"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "rpc_method=eth_call")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "gas_policy=profile_max_fee")
    });
    assert!(passed_erc20_revoke);
    let passed_nft_revoke = simulated_steps.iter().any(|step| {
        step["action"] == "revoke_nft_operator_approval" && step["simulation_status"] == "passed"
    });
    assert!(passed_nft_revoke);
    let passed_permit2 = simulated_steps.iter().any(|step| {
        step["action"] == "revoke_permit2_allowance"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "prepared_call=permit2.approve(token,spender,0,0)")
    });
    assert!(passed_permit2);
    let passed_claim = simulated_steps.iter().any(|step| {
        step["action"] == "claim_reward"
            && step["simulation_status"] == "passed"
            && step["status"] == "blocked"
            && step["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blocker| blocker == "claim_execution_disabled")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| {
                    evidence
                        == "prepared_call=claim.merkle_distributor_v1(index,account,amount,proof)"
                })
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "claim_proof_words=2")
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "transaction_gas_limit=120000")
    });
    assert!(passed_claim);
    assert!(
        simulate_json["plan"]["summary"]["executable_steps"]
            .as_u64()
            .unwrap()
            > 0
    );

    let export_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/export",
        json!({
            "plan_id": plan_json["plan"]["id"].as_str().unwrap(),
        }),
        Some(&token),
    )
    .await;
    let export_status = export_plan.status();
    let export_json: serde_json::Value = export_plan.json().await.unwrap();
    assert_eq!(
        export_status,
        StatusCode::OK,
        "export response: {export_json}"
    );
    assert_eq!(export_json["status"], "exported");
    assert_eq!(export_json["format"], "call_manifest");
    assert!(export_json["exported_steps"].as_u64().unwrap() > 0);
    let export_bundles = export_json["bundles"].as_array().unwrap();
    assert!(export_bundles.iter().any(|bundle| {
        bundle["source_address"] == "0x9858effd232b4033e47d90003d41ec34ecaeda94"
            && bundle["calls"].as_array().unwrap().iter().any(|call| {
                call["action"] == "sweep_erc20"
                    && call["data_hex"]
                        .as_str()
                        .is_some_and(|value| value.starts_with("0xa9059cbb"))
                    && call["value_wei_hex"] == "0x0"
            })
    }));
    assert!(export_bundles.iter().any(|bundle| {
        bundle["calls"].as_array().unwrap().iter().any(|call| {
            call["action"] == "sweep_native"
                && call["value_wei_hex"]
                    .as_str()
                    .is_some_and(|value| value.starts_with("0x"))
                && call["evidence"].as_array().unwrap().iter().any(|evidence| {
                    evidence.as_str().is_some_and(|value| {
                        value.starts_with("native_sweep_spendable_amount_hex=0x")
                    })
                })
        })
    }));
    assert!(
        export_json["skipped_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| {
                step["action"] == "claim_reward"
                    && step["reason"] == "blocked"
                    && step["blockers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|blocker| blocker == "claim_execution_disabled")
            })
    );

    let safe_export_without_address = post_json(
        &client,
        addr,
        "/api/plans/consolidation/export",
        json!({
            "plan_id": plan_json["plan"]["id"].as_str().unwrap(),
            "format": "safe_tx_builder",
        }),
        Some(&token),
    )
    .await;
    let safe_export_status = safe_export_without_address.status();
    let safe_export_json: serde_json::Value = safe_export_without_address.json().await.unwrap();
    assert_eq!(
        safe_export_status,
        StatusCode::BAD_REQUEST,
        "safe export response: {safe_export_json}"
    );
    assert!(
        safe_export_json["error"]
            .as_str()
            .is_some_and(|error| error.contains("safe_address"))
    );

    let catalog_delete = post_json(
        &client,
        addr,
        "/api/risk/catalog/delete",
        json!({
            "address": "0x4444444444444444444444444444444444444444",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(catalog_delete.status(), StatusCode::OK);
    let claim_catalog_delete = post_json(
        &client,
        addr,
        "/api/risk/catalog/delete",
        json!({
            "address": "0x1111111111111111111111111111111111111111",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(claim_catalog_delete.status(), StatusCode::OK);
    let catalog = get(&client, addr, "/api/risk/catalog", Some(&token)).await;
    let catalog_json: serde_json::Value = catalog.json().await.unwrap();
    assert!(catalog_json["entries"].as_array().unwrap().is_empty());

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_erc1155_transfer_batch_yields_only_positive_balance_holdings() {
    fn token_word(value: u64) -> String {
        format!("0x{value:064x}")
    }

    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_erc1155_batch_mock_evm_provider().await;
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
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
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
            "discover_erc1155_transfers": true,
            "nft_discovery_from_block": "0x0",
            "nft_discovery_limit": 8
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");

    let token_a1 = token_word(0xa1);
    let token_b2 = token_word(0xb2);
    let token_c3 = token_word(0xc3);
    let holdings = scan_json["holdings"].as_array().unwrap();
    let erc1155_log_holdings = holdings
        .iter()
        .filter(|holding| holding["source"] == "erc1155-transfer-log")
        .collect::<Vec<_>>();
    assert_eq!(erc1155_log_holdings.len(), 2);
    assert!(erc1155_log_holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc1155"
            && holding["asset_address"] == "0x1155000000000000000000000000000000000000"
            && holding["token_id_hex"] == token_a1
            && holding["amount_hex"] == "0x5"
    }));
    assert!(erc1155_log_holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc1155"
            && holding["asset_address"] == "0x1155000000000000000000000000000000000000"
            && holding["token_id_hex"] == token_c3
            && holding["amount_hex"] == "0x7"
    }));
    assert!(
        !holdings
            .iter()
            .any(|holding| holding["token_id_hex"] == token_b2)
    );
    assert!(
        scan_json["job"]["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|checkpoint| {
                checkpoint["wallet_family"] == "eth-seed"
                    && checkpoint["wallet_profile"] == "seed-main"
                    && checkpoint["provider_profile"] == "mainnet"
                    && checkpoint["completed"] == true
            })
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_inventory_scan_all_configured_chains_splits_consolidation_plans() {
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

    let set_provider_token = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;
    assert_eq!(set_provider_token.status(), StatusCode::OK);

    for (name, chain_id) in [("mainnet", 1_u64), ("base", 8453_u64)] {
        let provider = post_json(
            &client,
            addr,
            "/api/profiles/evm/upsert",
            json!({
                "name": name,
                "rpc_url": format!("http://{rpc_addr}/"),
                "auth_token_key": "alchemy",
                "chain_id": chain_id,
            }),
            Some(&token),
        )
        .await;
        assert_eq!(provider.status(), StatusCode::OK);
    }

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

    let invalid_scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "provider_profile": "mainnet",
            "all_configured_chains": true,
            "gap_limit": 1,
            "max_index": 0,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_scan.status(), StatusCode::BAD_REQUEST);

    let scan = post_json(
        &client,
        addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-seed",
            "wallet_profile": "seed-main",
            "all_configured_chains": true,
            "gap_limit": 1,
            "max_index": 0,
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");
    assert_eq!(scan_json["job"]["status"], "completed");
    let job_chain_ids = scan_json["job"]["chain_ids"].as_array().unwrap();
    assert!(job_chain_ids.iter().any(|chain_id| chain_id == 1));
    assert!(job_chain_ids.iter().any(|chain_id| chain_id == 8453));
    let provider_profiles = scan_json["job"]["provider_profiles"].as_array().unwrap();
    assert!(
        provider_profiles
            .iter()
            .any(|provider| provider == "mainnet")
    );
    assert!(provider_profiles.iter().any(|provider| provider == "base"));

    let observed_chain_ids = scan_json["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|address| address["chain_id"].as_u64())
        .collect::<HashSet<_>>();
    assert!(observed_chain_ids.contains(&1));
    assert!(observed_chain_ids.contains(&8453));

    let plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({
            "destination_address": "0x9999999999999999999999999999999999999999",
        }),
        Some(&token),
    )
    .await;
    let plan_status = plan.status();
    let plan_json: serde_json::Value = plan.json().await.unwrap();
    assert_eq!(plan_status, StatusCode::OK, "plan response: {plan_json}");
    let plans = plan_json["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 2);
    let plan_chain_ids = plans
        .iter()
        .filter_map(|plan| plan["chain_id"].as_u64())
        .collect::<HashSet<_>>();
    assert_eq!(plan_chain_ids, HashSet::from([1_u64, 8453_u64]));
    for plan in plans {
        let plan_chain_id = plan["chain_id"].as_u64().unwrap();
        assert!(
            plan["steps"]
                .as_array()
                .unwrap()
                .iter()
                .all(|step| step["chain_id"].as_u64() == Some(plan_chain_id))
        );
    }

    let base_only_plan = post_json(
        &client,
        addr,
        "/api/plans/consolidation/generate",
        json!({
            "destination_address": "0x9999999999999999999999999999999999999999",
            "chain_id": 8453,
        }),
        Some(&token),
    )
    .await;
    let base_only_status = base_only_plan.status();
    let base_only_json: serde_json::Value = base_only_plan.json().await.unwrap();
    assert_eq!(
        base_only_status,
        StatusCode::OK,
        "base-only plan response: {base_only_json}"
    );
    assert_eq!(base_only_json["plans"].as_array().unwrap().len(), 1);
    assert_eq!(base_only_json["plan"]["chain_id"], 8453);
    assert!(
        base_only_json["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["chain_id"].as_u64() == Some(8453))
    );

    handle.abort();
    rpc_handle.abort();
}
