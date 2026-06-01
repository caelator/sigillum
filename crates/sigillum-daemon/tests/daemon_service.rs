use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

async fn spawn_daemon(base_dir: PathBuf) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _state) = sigillum_daemon::build_router(base_dir, addr.port());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

#[derive(Clone)]
struct RpcState;

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn rpc_response(request: &serde_json::Value) -> serde_json::Value {
        fn abi_word(value: u64) -> String {
            format!("{value:064x}")
        }

        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_call" => {
                let to = request["params"][0]["to"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let data = request["params"][0]["data"].as_str().unwrap_or_default();
                if to == "0x000000000022d473030f116ddee9f6b43ac78ba3" {
                    if data.contains("9858effd232b4033e47d90003d41ec34ecaeda94")
                        && data.contains("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
                        && data.contains("4444444444444444444444444444444444444444")
                    {
                        json!(format!(
                            "0x{}{}{}",
                            abi_word(0x0f4240),
                            abi_word(0x6fffffff),
                            abi_word(1)
                        ))
                    } else {
                        json!(format!("0x{}{}{}", abi_word(0), abi_word(0), abi_word(0)))
                    }
                } else if data.starts_with("0xe985e9c5") {
                    if data.contains("9858effd232b4033e47d90003d41ec34ecaeda94")
                        && data.contains("3333333333333333333333333333333333333333")
                    {
                        json!(format!("0x{}1", "0".repeat(63)))
                    } else {
                        json!(format!("0x{}", "0".repeat(64)))
                    }
                } else if data.starts_with("0x6352211e") {
                    json!("0x0000000000000000000000009858effd232b4033e47d90003d41ec34ecaeda94")
                } else if data.starts_with("0x00fdd58e") {
                    if data.contains("9858effd232b4033e47d90003d41ec34ecaeda94")
                        && data.ends_with(
                            "000000000000000000000000000000000000000000000000000000000000007b",
                        )
                    {
                        json!("0x2a")
                    } else {
                        json!("0x0")
                    }
                } else {
                    json!("0x0f4240")
                }
            }
            "eth_getLogs" => {
                const ERC20_OR_ERC721_TRANSFER_TOPIC: &str =
                    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
                let filter = &request["params"][0];
                let fallback_topic = format!("0x{}", "00".repeat(32));
                let topics = filter["topics"].as_array().cloned().unwrap_or_default();
                let event_topic = topics
                    .first()
                    .and_then(|topic| topic.as_str())
                    .unwrap_or("");
                let mut log_topics = topics
                    .iter()
                    .map(|topic| {
                        topic
                            .as_str()
                            .map(|value| json!(value))
                            .unwrap_or_else(|| json!(fallback_topic))
                    })
                    .collect::<Vec<_>>();
                let is_nft_filter = topics.len() >= 4;
                let is_erc1155_filter =
                    is_nft_filter && event_topic != ERC20_OR_ERC721_TRANSFER_TOPIC;
                if is_nft_filter && !is_erc1155_filter {
                    log_topics[3] = json!(format!("0x{}7b", "0".repeat(62)));
                }
                json!([{
                    "address": if is_erc1155_filter {
                        "0x1155000000000000000000000000000000000000"
                    } else if is_nft_filter {
                        "0x1234500000000000000000000000000000000000"
                    } else {
                        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
                    },
                    "topics": log_topics,
                    "data": if is_erc1155_filter {
                        format!(
                            "0x{}40{}80{}01{}7b{}01{}2a",
                            "0".repeat(62),
                            "0".repeat(62),
                            "0".repeat(62),
                            "0".repeat(62),
                            "0".repeat(62),
                            "0".repeat(62)
                        )
                    } else {
                        format!("0x{}0f4240", "0".repeat(58))
                    },
                    "blockNumber": "0x10",
                    "transactionHash": format!("0x{}", "44".repeat(32)),
                    "logIndex": "0x0"
                }])
            }
            "eth_sendRawTransaction" => json!(format!("0x{}", "11".repeat(32))),
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
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
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
            serde_json::Value::Array(requests.iter().map(rpc_response).collect())
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

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: serde_json::Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

async fn get(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.get(format!("http://{addr}{path}"));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
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
        json!("11".repeat(32))
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
async fn xpub_profiles_export_and_derive_receive_addresses() {
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

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:8545/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    let xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "treasury-receive",
            "project_account": 7,
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(xpub_profile.status(), StatusCode::OK);
    let xpub_profile_json: serde_json::Value = xpub_profile.json().await.unwrap();
    assert_eq!(xpub_profile_json["profile"]["project_account"], 7);

    let list = get(&client, addr, "/api/profiles/eth-xpub", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_json["profiles"][0]["name"], "treasury-receive");

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/export",
        json!({ "wallet_profile": "treasury-receive" }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: serde_json::Value = export.json().await.unwrap();
    let receive_xpub = export_json["receive_xpub"].as_str().unwrap().to_string();
    assert_eq!(export_json["wallet_profile"], "treasury-receive");
    assert_eq!(export_json["project_account"], 7);
    assert_eq!(export_json["account_path"], "m/44'/60'/7'");
    assert_eq!(export_json["receive_path"], "m/44'/60'/7'/0");
    assert!(receive_xpub.starts_with("xpub"));

    let derive_zero = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/derive",
        json!({
            "xpub": receive_xpub,
            "index": 0,
        }),
        None,
    )
    .await;
    assert_eq!(derive_zero.status(), StatusCode::OK);
    let derive_zero_json: serde_json::Value = derive_zero.json().await.unwrap();
    let address_zero = derive_zero_json["address"].as_str().unwrap().to_string();
    assert_eq!(derive_zero_json["index"], 0);
    assert!(address_zero.starts_with("0x"));
    assert_eq!(address_zero.len(), 42);

    let derive_one = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/derive",
        json!({
            "xpub": export_json["receive_xpub"],
            "index": 1,
        }),
        None,
    )
    .await;
    assert_eq!(derive_one.status(), StatusCode::OK);
    let derive_one_json: serde_json::Value = derive_one.json().await.unwrap();
    assert_ne!(derive_one_json["address"], address_zero);

    let audit = get(&client, addr, "/api/audit?limit=20", Some(&token)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: serde_json::Value = audit.json().await.unwrap();
    assert!(
        audit_json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["kind"] == "wallet.eth_xpub.export")
    );

    handle.abort();
}

#[tokio::test]
async fn seed_wallet_profiles_import_12_and_24_word_phrases() {
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

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:8545/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let seed_12 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-12",
            "label": "Twelve word",
            "mnemonic": twelve_word,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed_12.status(), StatusCode::OK);
    let seed_12_json: serde_json::Value = seed_12.json().await.unwrap();
    assert_eq!(seed_12_json["profile"]["word_count"], 12);
    assert_eq!(seed_12_json["profile"]["account_path"], "m/44'/60'/0'");
    assert!(
        seed_12_json["profile"]["receive_xpub"]
            .as_str()
            .unwrap()
            .starts_with("xpub")
    );
    assert_eq!(
        seed_12_json["profile"]["first_receive_address"]
            .as_str()
            .unwrap()
            .len(),
        42
    );

    let twenty_four_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    let seed_24 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-24",
            "label": "Twenty four word",
            "mnemonic": twenty_four_word,
            "mnemonic_passphrase": "optional",
            "project_account": 1,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed_24.status(), StatusCode::OK);
    let seed_24_json: serde_json::Value = seed_24.json().await.unwrap();
    assert_eq!(seed_24_json["profile"]["word_count"], 24);
    assert_eq!(seed_24_json["profile"]["account_path"], "m/44'/60'/1'");

    let list = get(&client, addr, "/api/profiles/eth-seed", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    let profiles = list_json["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["name"], "seed-12");
    assert_eq!(profiles[1]["name"], "seed-24");

    let invalid = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-invalid",
            "mnemonic": "abandon abandon",
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let delete = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/delete",
        json!({ "name": "seed-12" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete.status(), StatusCode::OK);

    handle.abort();
}

#[tokio::test]
async fn profile_backed_send_and_queue_flow_persist_internal_configuration() {
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

    let provider_profile = post_json(
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
    assert_eq!(provider_profile.status(), StatusCode::OK);

    let wallet_profile = post_json(
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
    assert_eq!(wallet_profile.status(), StatusCode::OK);

    let providers = get(&client, addr, "/api/profiles/evm", Some(&token)).await;
    let providers_json: serde_json::Value = providers.json().await.unwrap();
    assert_eq!(providers_json["profiles"][0]["name"], "mainnet");

    let wallets = get(&client, addr, "/api/profiles/eth-stealth", Some(&token)).await;
    let wallets_json: serde_json::Value = wallets.json().await.unwrap();
    assert_eq!(wallets_json["profiles"][0]["name"], "payments-mainnet");

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
            "ephemeral_private_key_hex": hex::encode([7u8; 32]),
        }),
        None,
    )
    .await;
    let generate_json: serde_json::Value = generate.json().await.unwrap();

    let send = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-with-profile",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000",
            "broadcast": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send.status(), StatusCode::OK);
    let send_json: serde_json::Value = send.json().await.unwrap();
    assert_eq!(send_json["kind"], "eth-transfer");
    assert_eq!(
        send_json["to_address"],
        "0x1111111111111111111111111111111111111111"
    );

    let enqueue = post_json(
        &client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(enqueue.status(), StatusCode::OK);
    let enqueue_json: serde_json::Value = enqueue.json().await.unwrap();
    let job_id = enqueue_json["job"]["id"].as_str().unwrap().to_string();

    let list_before = get(&client, addr, "/api/queue/jobs", Some(&token)).await;
    let list_before_json: serde_json::Value = list_before.json().await.unwrap();
    assert_eq!(list_before_json["jobs"][0]["state"], "queued");

    let process = post_json(
        &client,
        addr,
        "/api/queue/process",
        json!({ "id": job_id }),
        Some(&token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: serde_json::Value = process.json().await.unwrap();
    assert_eq!(process_json["succeeded"], 1);
    assert_eq!(process_json["jobs"][0]["state"], "sent");

    let list_after = get(&client, addr, "/api/queue/jobs", Some(&token)).await;
    let list_after_json: serde_json::Value = list_after.json().await.unwrap();
    assert_eq!(list_after_json["jobs"][0]["state"], "sent");
    assert_eq!(
        list_after_json["jobs"][0]["broadcast_transaction_hash_hex"],
        json!("11".repeat(32))
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn profile_bound_wallet_and_provider_work_after_session_switches_compartments() {
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
            "erc20_gas_limit": 65000
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
            "default_destination_address": "0x1111111111111111111111111111111111111111"
        }),
        Some(&token),
    )
    .await;

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth"
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
            "ephemeral_private_key_hex": hex::encode([5u8; 32]),
        }),
        None,
    )
    .await;
    let generate_json: serde_json::Value = generate.json().await.unwrap();

    let add = post_json(
        &client,
        addr,
        "/api/compartment/add",
        json!({
            "label": "secure",
            "threshold": 2,
            "passphrase_mode": "wrapped"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(add.status(), StatusCode::OK);
    let add_json: serde_json::Value = add.json().await.unwrap();
    assert_eq!(add_json["id"], 1);

    let switch = post_json(
        &client,
        addr,
        "/api/compartment/switch",
        json!({ "id": 1 }),
        Some(&token),
    )
    .await;
    assert_eq!(switch.status(), StatusCode::OK);

    let send = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-with-profile",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000",
            "broadcast": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send.status(), StatusCode::OK);
    let send_json: serde_json::Value = send.json().await.unwrap();
    assert_eq!(send_json["kind"], "eth-transfer");
    assert_eq!(
        send_json["to_address"],
        "0x1111111111111111111111111111111111111111"
    );

    handle.abort();
    rpc_handle.abort();
}

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
    assert_eq!(scan_json["job"]["holdings_detected"], 17);

    let holdings = scan_json["holdings"].as_array().unwrap();
    assert_eq!(holdings.len(), 17);
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
    }));
    assert!(holdings.iter().any(|holding| {
        holding["asset_kind"] == "erc1155"
            && holding["asset_address"] == "0x1155000000000000000000000000000000000000"
            && holding["token_id_hex"]
                == "0x000000000000000000000000000000000000000000000000000000000000007b"
            && holding["amount_hex"] == "0x2a"
            && holding["source"] == "erc1155-transfer-log"
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
    let catalog = get(&client, addr, "/api/risk/catalog", Some(&token)).await;
    assert_eq!(catalog.status(), StatusCode::OK);
    let catalog_json: serde_json::Value = catalog.json().await.unwrap();
    assert_eq!(catalog_json["entries"].as_array().unwrap().len(), 1);

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
    let passed_erc20_revoke = simulated_steps.iter().any(|step| {
        step["action"] == "revoke_erc20_approval"
            && step["simulation_status"] == "passed"
            && step["simulation_evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| evidence == "rpc_method=eth_call")
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
    assert!(
        simulate_json["plan"]["summary"]["executable_steps"]
            .as_u64()
            .unwrap()
            > 0
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
    let catalog = get(&client, addr, "/api/risk/catalog", Some(&token)).await;
    let catalog_json: serde_json::Value = catalog.json().await.unwrap();
    assert!(catalog_json["entries"].as_array().unwrap().is_empty());

    handle.abort();
    rpc_handle.abort();
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
    let native_id = native_deposit_json["deposit"]["id"]
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
    assert_eq!(refresh_json["detected"], 1);
    assert_eq!(refresh_json["queued"], 1);
    let sweep_job_id = refresh_json["deposits"][0]["queue_job_id"]
        .as_str()
        .unwrap()
        .to_string();

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
        json!("11".repeat(32))
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

    let diagnostics = get(&client, addr, "/api/diagnostics", Some(&token)).await;
    assert_eq!(diagnostics.status(), StatusCode::OK);
    let diagnostics_json: serde_json::Value = diagnostics.json().await.unwrap();
    assert_eq!(diagnostics_json["queue_job_count"], 2);
    assert_eq!(diagnostics_json["eth_stealth_deposit_count"], 2);

    let deposits = get(&client, addr, "/api/deposits/eth-stealth", Some(&token)).await;
    assert_eq!(deposits.status(), StatusCode::OK);
    let deposits_json: serde_json::Value = deposits.json().await.unwrap();
    assert_eq!(deposits_json["deposits"].as_array().unwrap().len(), 2);

    handle.abort();
    rpc_handle.abort();
}
