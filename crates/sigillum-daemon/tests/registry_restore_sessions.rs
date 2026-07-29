mod common;

use common::mock_evm::spawn_mock_evm_provider;
use common::{get, init_default_compartment, post_json, spawn_daemon};
use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn chain_registry_routes_seed_builtins_and_manage_custom_profiles() {
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

    let builtins = get(&client, addr, "/api/chains", Some(&token)).await;
    assert_eq!(builtins.status(), StatusCode::OK);
    let builtins_json: serde_json::Value = builtins.json().await.unwrap();
    let profiles = builtins_json["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 5);
    assert!(profiles.iter().any(|profile| {
        profile["name"] == "polygon-pos"
            && profile["chain_id"] == 137
            && profile["native_symbol"] == "POL"
            && profile["native_decimals"] == 18
            && profile["builtin"] == true
    }));

    let upsert = post_json(
        &client,
        addr,
        "/api/chains/upsert",
        json!({
            "name": "test-rollup",
            "chain_family": "evm",
            "chain_id": 999999,
            "native_symbol": "TST",
            "native_decimals": 18,
            "finality_blocks": 64,
            "permit2_address": "0X5555555555555555555555555555555555555555",
            "enabled": true
        }),
        Some(&token),
    )
    .await;
    assert_eq!(upsert.status(), StatusCode::OK);
    let upsert_json: serde_json::Value = upsert.json().await.unwrap();
    assert_eq!(upsert_json["profile"]["name"], "test-rollup");
    assert_eq!(upsert_json["profile"]["chain_id"], 999999);
    assert_eq!(upsert_json["profile"]["finality_blocks"], 64);
    assert_eq!(
        upsert_json["profile"]["permit2_address"],
        "0x5555555555555555555555555555555555555555"
    );
    assert_eq!(upsert_json["profile"]["builtin"], false);

    let duplicate = post_json(
        &client,
        addr,
        "/api/chains/upsert",
        json!({
            "name": "duplicate-rollup",
            "chain_family": "evm",
            "chain_id": 999999
        }),
        Some(&token),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let delete_builtin = post_json(
        &client,
        addr,
        "/api/chains/delete",
        json!({ "name": "ethereum" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete_builtin.status(), StatusCode::BAD_REQUEST);

    let alias_list = get(&client, addr, "/api/inventory/chains", Some(&token)).await;
    assert_eq!(alias_list.status(), StatusCode::OK);
    let alias_json: serde_json::Value = alias_list.json().await.unwrap();
    assert!(
        alias_json["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|profile| profile["name"] == "test-rollup")
    );

    let delete_custom = post_json(
        &client,
        addr,
        "/api/chains/delete",
        json!({ "name": "test-rollup" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete_custom.status(), StatusCode::OK);

    let after_delete = get(&client, addr, "/api/chains", Some(&token)).await;
    assert_eq!(after_delete.status(), StatusCode::OK);
    let after_delete_json: serde_json::Value = after_delete.json().await.unwrap();
    assert!(
        !after_delete_json["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|profile| profile["name"] == "test-rollup")
    );

    handle.abort();
}

#[tokio::test]
async fn failed_restore_preserves_existing_session_and_data() {
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

    let set_key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "github", "value": "ghp_test" }),
        Some(&token),
    )
    .await;
    assert_eq!(set_key.status(), StatusCode::OK);

    let export = post_json(
        &client,
        addr,
        "/api/backup/export",
        json!({ "passphrase": "snapshot passphrase" }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: serde_json::Value = export.json().await.unwrap();
    let snapshot_hex = export_json["snapshot_hex"].as_str().unwrap();

    let restore = post_json(
        &client,
        addr,
        "/api/backup/restore",
        json!({
            "passphrase": "wrong snapshot passphrase",
            "snapshot_hex": snapshot_hex,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(restore.status(), StatusCode::UNAUTHORIZED);

    let status = get(&client, addr, "/api/status", Some(&token)).await;
    assert_eq!(status.status(), StatusCode::OK);
    let status_json: serde_json::Value = status.json().await.unwrap();
    assert_eq!(status_json["locked"], false);

    let api_keys = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert_eq!(api_keys.status(), StatusCode::OK);
    let api_keys_json: serde_json::Value = api_keys.json().await.unwrap();
    assert_eq!(api_keys_json["keys"], json!(["github"]));

    handle.abort();
}

#[tokio::test]
async fn successful_restore_clears_session_and_restores_data_on_disk() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();

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

    let set_key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "github", "value": "ghp_test" }),
        Some(&token),
    )
    .await;
    assert_eq!(set_key.status(), StatusCode::OK);

    let export = post_json(
        &client,
        addr,
        "/api/backup/export",
        json!({ "passphrase": "snapshot passphrase" }),
        Some(&token),
    )
    .await;
    let export_json: serde_json::Value = export.json().await.unwrap();
    let snapshot_hex = export_json["snapshot_hex"].as_str().unwrap();

    let delete_key = post_json(
        &client,
        addr,
        "/api/api-keys/delete",
        json!({ "key": "github" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete_key.status(), StatusCode::OK);

    let restore = post_json(
        &client,
        addr,
        "/api/backup/restore",
        json!({
            "passphrase": "snapshot passphrase",
            "snapshot_hex": snapshot_hex,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(restore.status(), StatusCode::OK);

    let old_token_keys = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert_eq!(old_token_keys.status(), StatusCode::UNAUTHORIZED);

    let relock_status = get(&client, addr, "/api/status", None).await;
    let relock_json: serde_json::Value = relock_status.json().await.unwrap();
    assert_eq!(relock_json["locked"], true);

    let restored_api_keys = std::fs::read_to_string(
        dir.path()
            .join("compartments")
            .join("0")
            .join("api_keys.json"),
    )
    .unwrap();
    let restored_api_keys: serde_json::Value = serde_json::from_str(&restored_api_keys).unwrap();
    assert_eq!(restored_api_keys, json!({ "github": "ghp_test" }));

    handle.abort();
}

#[tokio::test]
async fn session_revoke_invalidates_only_the_current_token() {
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

    let revoke = post_json(
        &client,
        addr,
        "/api/session/revoke",
        json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::OK);

    let api_keys = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert_eq!(api_keys.status(), StatusCode::UNAUTHORIZED);

    let status = get(&client, addr, "/api/status", None).await;
    assert_eq!(status.status(), StatusCode::OK);
    let status_json: serde_json::Value = status.json().await.unwrap();
    assert_eq!(status_json["locked"], true);

    handle.abort();
}

#[tokio::test]
async fn malformed_tier1_store_surfaces_as_server_error() {
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

    std::fs::write(
        dir.path()
            .join("compartments")
            .join("0")
            .join("api_keys.json"),
        "{not json",
    )
    .unwrap();

    let api_keys = get(&client, addr, "/api/api-keys", Some(&token)).await;
    assert_eq!(api_keys.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let status = get(&client, addr, "/api/status", Some(&token)).await;
    assert_eq!(status.status(), StatusCode::INTERNAL_SERVER_ERROR);

    handle.abort();
}

#[tokio::test]
async fn eth_stealth_routes_roundtrip_from_meta_export_to_local_signing() {
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

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: serde_json::Value = export.json().await.unwrap();
    let stealth_meta_address = export_json["stealth_meta_address"].as_str().unwrap();
    assert!(stealth_meta_address.starts_with("st:eth:0x"));

    let generate = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": stealth_meta_address,
            "ephemeral_private_key_hex": hex::encode([3u8; 32]),
        }),
        None,
    )
    .await;
    assert_eq!(generate.status(), StatusCode::OK);
    let generate_json: serde_json::Value = generate.json().await.unwrap();
    let stealth_address = generate_json["stealth_address"].as_str().unwrap();
    let ephemeral_public_key_hex = generate_json["ephemeral_public_key_hex"]
        .as_str()
        .unwrap()
        .to_string();
    let view_tag_hex = generate_json["view_tag_hex"].as_str().unwrap().to_string();
    assert!(stealth_address.starts_with("0x"));

    let check = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/check",
        json!({
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": ephemeral_public_key_hex,
            "view_tag_hex": view_tag_hex,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(check.status(), StatusCode::OK);
    let check_json: serde_json::Value = check.json().await.unwrap();
    assert_eq!(check_json["matches"], true);

    let sign = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/sign",
        json!({
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "digest_hex": hex::encode([9u8; 32]),
        }),
        Some(&token),
    )
    .await;
    assert_eq!(sign.status(), StatusCode::OK);
    let sign_json: serde_json::Value = sign.json().await.unwrap();
    assert_eq!(sign_json["stealth_address"], stealth_address);
    assert_eq!(
        hex::decode(sign_json["signature_hex"].as_str().unwrap())
            .unwrap()
            .len(),
        65
    );

    let sign_transfer = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/sign-transfer",
        json!({
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "chain_id": 1,
            "nonce": 7,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "gas_limit": 21000,
            "destination_address": "0x1111111111111111111111111111111111111111",
            "value_wei_hex": "0xde0b6b3a7640000",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(sign_transfer.status(), StatusCode::OK);
    let sign_transfer_json: serde_json::Value = sign_transfer.json().await.unwrap();
    assert_eq!(sign_transfer_json["kind"], "eth-transfer");
    assert!(
        sign_transfer_json["raw_transaction_hex"]
            .as_str()
            .unwrap()
            .starts_with("02")
    );

    let sign_erc20 = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/sign-erc20-transfer",
        json!({
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "chain_id": 1,
            "nonce": 8,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "gas_limit": 65000,
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "recipient_address": "0x2222222222222222222222222222222222222222",
            "amount_hex": "0x0f4240",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(sign_erc20.status(), StatusCode::OK);
    let sign_erc20_json: serde_json::Value = sign_erc20.json().await.unwrap();
    assert_eq!(sign_erc20_json["kind"], "erc20-transfer");
    assert!(
        sign_erc20_json["data_hex"]
            .as_str()
            .unwrap()
            .starts_with("a9059cbb")
    );

    handle.abort();
}

#[tokio::test]
async fn evm_provider_routes_and_stealth_send_flow_work_with_internal_auth_resolution() {
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

    let set_provider_token = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;
    assert_eq!(set_provider_token.status(), StatusCode::OK);

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth",
        }),
        Some(&token),
    )
    .await;
    let export_json: serde_json::Value = export.json().await.unwrap();

    let generate = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": export_json["stealth_meta_address"],
            "ephemeral_private_key_hex": hex::encode([3u8; 32]),
        }),
        None,
    )
    .await;
    let generate_json: serde_json::Value = generate.json().await.unwrap();
    let stealth_address = generate_json["stealth_address"].as_str().unwrap();
    let provider_url = format!("http://{rpc_addr}/");

    let fee_estimate = post_json(
        &client,
        addr,
        "/api/evm/fees/estimate",
        json!({
            "rpc_url": provider_url.clone(),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "gas_limit": 21000,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(fee_estimate.status(), StatusCode::OK);
    let fee_json: serde_json::Value = fee_estimate.json().await.unwrap();
    assert_eq!(
        fee_json["fees"]["max_priority_fee_per_gas_hex"],
        "0x59682f00"
    );
    assert_eq!(fee_json["fees"]["max_fee_per_gas_hex"], "0xd09dc300");
    assert_eq!(fee_json["estimated_gas_cost_wei_hex"], "0x42d90d641800");

    let nonce = post_json(
        &client,
        addr,
        "/api/evm/nonce",
        json!({
            "rpc_url": provider_url,
            "address": stealth_address,
            "auth_token_key": "alchemy",
            "block_tag": "pending",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(nonce.status(), StatusCode::OK);
    let nonce_json: serde_json::Value = nonce.json().await.unwrap();
    assert_eq!(nonce_json["nonce"], 7);

    let balance = post_json(
        &client,
        addr,
        "/api/evm/balance",
        json!({
            "rpc_url": format!("http://{rpc_addr}/"),
            "address": stealth_address,
            "auth_token_key": "alchemy",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(balance.status(), StatusCode::OK);
    let balance_json: serde_json::Value = balance.json().await.unwrap();
    assert_eq!(balance_json["balance_wei_hex"], "0xde0b6b3a7640000");

    let erc20_balance = post_json(
        &client,
        addr,
        "/api/evm/erc20-balance",
        json!({
            "rpc_url": format!("http://{rpc_addr}/"),
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "owner_address": stealth_address,
            "auth_token_key": "alchemy",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(erc20_balance.status(), StatusCode::OK);
    let erc20_balance_json: serde_json::Value = erc20_balance.json().await.unwrap();
    assert_eq!(erc20_balance_json["amount_hex"], "0xf4240");

    let send_native = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-transfer",
        json!({
            "rpc_url": format!("http://{rpc_addr}/"),
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "chain_id": 1,
            "destination_address": "0x1111111111111111111111111111111111111111",
            "value_wei_hex": "0xde0b6b3a7640000",
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "auth_token_key": "alchemy",
            "broadcast": true
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send_native.status(), StatusCode::OK);
    let send_native_json: serde_json::Value = send_native.json().await.unwrap();
    assert_eq!(send_native_json["nonce"], 7);
    assert_eq!(send_native_json["broadcast"], true);
    assert_eq!(
        send_native_json["broadcast_transaction_hash_hex"],
        send_native_json["transaction_hash_hex"]
    );

    let send_erc20 = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-erc20-transfer",
        json!({
            "rpc_url": format!("http://{rpc_addr}/"),
            "wallet": "payments",
            "stealth_address": stealth_address,
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "chain_id": 1,
            "token_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "recipient_address": "0x2222222222222222222222222222222222222222",
            "amount_hex": "0x0f4240",
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "auth_token_key": "alchemy",
            "broadcast": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send_erc20.status(), StatusCode::OK);
    let send_erc20_json: serde_json::Value = send_erc20.json().await.unwrap();
    assert_eq!(send_erc20_json["nonce"], 7);
    assert_eq!(send_erc20_json["broadcast"], false);
    assert!(
        send_erc20_json["data_hex"]
            .as_str()
            .unwrap()
            .starts_with("a9059cbb")
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_claim_execution_optin_with_all_gates_unblocks_merkle_claim() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;

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
            "claim_candidate_limit": 4
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");

    let risk = post_json(
        &client,
        addr,
        "/api/risk/catalog/upsert",
        json!({
            "address": "0x1111111111111111111111111111111111111111",
            "risk_level": "trusted"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(risk.status(), StatusCode::OK);

    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_claim_execution": true,
            "allowed_destinations": [{
                "address": "0x9999999999999999999999999999999999999999",
                "label": "cold-treasury"
            }]
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

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
    let generated_claim = plan_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "claim_reward")
        .unwrap_or_else(|| panic!("missing claim step in {plan_json}"));
    assert_eq!(generated_claim["status"], "blocked");
    assert_eq!(generated_claim["simulation_status"], "required");
    assert_eq!(
        generated_claim["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|blocker| blocker.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claim_execution_disabled"]
    );

    let simulate = post_json(
        &client,
        addr,
        "/api/plans/consolidation/simulate",
        json!({ "plan_id": plan_json["plan"]["id"].as_str().unwrap() }),
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
    let simulated_claim = simulate_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "claim_reward")
        .unwrap_or_else(|| panic!("missing simulated claim step in {simulate_json}"));
    assert_eq!(simulated_claim["simulation_status"], "passed");
    assert_eq!(simulated_claim["status"], "blocked");
    assert_eq!(
        simulated_claim["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|blocker| blocker.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claim_execution_disabled"]
    );

    let approve = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": plan_json["plan"]["id"].as_str().unwrap() }),
        Some(&token),
    )
    .await;
    let approve_status = approve.status();
    let approve_json: serde_json::Value = approve.json().await.unwrap();
    assert_eq!(
        approve_status,
        StatusCode::OK,
        "approve response: {approve_json}"
    );
    let approved_claim = approve_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "claim_reward")
        .unwrap_or_else(|| panic!("missing approved claim step in {approve_json}"));
    assert_eq!(approved_claim["approved"], true);
    assert_eq!(approved_claim["status"], "approved");
    assert!(
        approved_claim["blockers"]
            .as_array()
            .is_none_or(|blockers| blockers.is_empty())
    );
    assert_eq!(approved_claim["simulation_status"], "passed");
    assert!(
        approve_json["plan"]["summary"]["executable_steps"]
            .as_u64()
            .unwrap()
            >= 1
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn wallet_claim_execution_optin_without_reviewed_claim_contract_keeps_blocker() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let token = init_default_compartment(&client, addr).await;

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
            "claim_candidate_limit": 4
        }),
        Some(&token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");

    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_claim_execution": true,
            "allowed_destinations": [{
                "address": "0x9999999999999999999999999999999999999999",
                "label": "cold-treasury"
            }]
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

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

    let simulate = post_json(
        &client,
        addr,
        "/api/plans/consolidation/simulate",
        json!({ "plan_id": plan_json["plan"]["id"].as_str().unwrap() }),
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

    let approve = post_json(
        &client,
        addr,
        "/api/plans/consolidation/approve",
        json!({ "plan_id": plan_json["plan"]["id"].as_str().unwrap() }),
        Some(&token),
    )
    .await;
    let approve_status = approve.status();
    let approve_json: serde_json::Value = approve.json().await.unwrap();
    assert_eq!(
        approve_status,
        StatusCode::OK,
        "approve response: {approve_json}"
    );
    let approved_claim = approve_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["action"] == "claim_reward")
        .unwrap_or_else(|| panic!("missing claim step in {approve_json}"));
    assert_eq!(approved_claim["status"], "blocked");
    assert!(!approved_claim["approved"].as_bool().unwrap_or(false));
    assert_eq!(
        approved_claim["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|blocker| blocker.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claim_execution_disabled"]
    );
    let executable_claims = approve_json["plan"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| {
            step["action"] == "claim_reward"
                && step["status"] == "approved"
                && step["simulation_status"] == "passed"
                && step["blockers"]
                    .as_array()
                    .is_none_or(|blockers| blockers.is_empty())
        })
        .count();
    assert_eq!(executable_claims, 0);

    handle.abort();
    rpc_handle.abort();
}
