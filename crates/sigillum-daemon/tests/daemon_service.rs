use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

fn submitted_raw_transaction_hash(request: &serde_json::Value) -> serde_json::Value {
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

#[derive(Clone)]
struct RpcState;

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn rpc_response(request: &serde_json::Value) -> serde_json::Value {
        fn abi_word(value: u64) -> String {
            format!("{value:064x}")
        }

        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_feeHistory" => json!({
                "oldestBlock": "0x1",
                "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
                "gasUsedRatio": [0.5]
            }),
            "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
            "eth_call" => {
                let to = request["params"][0]["to"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let data = request["params"][0]["data"].as_str().unwrap_or_default();
                if to == "0x1111111111111111111111111111111111111111"
                    && data.starts_with("0x2e7ba6ef")
                {
                    if data.contains("9858effd232b4033e47d90003d41ec34ecaeda94")
                        && data.contains(
                            "00000000000000000000000000000000000000000000000000000000000f4240",
                        )
                        && data.contains(&"11".repeat(32))
                        && data.contains(&"22".repeat(32))
                    {
                        json!("0x")
                    } else {
                        json!({ "claim": "missing evidence" })
                    }
                } else if to == "0x000000000022d473030f116ddee9f6b43ac78ba3" {
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
                } else if data == "0x" {
                    if request["params"][0]["value"].as_str().is_some() {
                        json!("0x")
                    } else {
                        json!({ "missing": "value" })
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
            "eth_sendRawTransaction" => submitted_raw_transaction_hash(request),
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

async fn spawn_erc1155_batch_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn abi_word(value: u64) -> String {
        format!("{value:064x}")
    }

    fn batch_data() -> String {
        let mut data = String::from("0x");
        for value in [0x40_u64, 0xc0, 0x3, 0xa1, 0xb2, 0xc3, 0x3, 0x5, 0x9, 0x7] {
            data.push_str(&abi_word(value));
        }
        data
    }

    fn rpc_response(request: &serde_json::Value) -> serde_json::Value {
        const TRANSFER_BATCH_TOPIC: &str =
            "0x4a39dc06d4c0dbc64b70af90fd698a233a518aa5d07e595d983b8c0526c8f7fb";
        const OWNER_ADDRESS: &str = "9858effd232b4033e47d90003d41ec34ecaeda94";

        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getTransactionCount" => json!("0x0"),
            "eth_getBalance" => json!("0x0"),
            "eth_getLogs" => {
                let filter = &request["params"][0];
                let fallback_topic = format!("0x{}", "00".repeat(32));
                let topics = filter["topics"].as_array().cloned().unwrap_or_default();
                let event_topic = topics
                    .first()
                    .and_then(|topic| topic.as_str())
                    .unwrap_or("");
                if event_topic == TRANSFER_BATCH_TOPIC {
                    let mut log_topics = topics
                        .iter()
                        .map(|topic| {
                            topic
                                .as_str()
                                .map(|value| json!(value))
                                .unwrap_or_else(|| json!(fallback_topic.clone()))
                        })
                        .collect::<Vec<_>>();
                    while log_topics.len() < 4 {
                        log_topics.push(json!(fallback_topic.clone()));
                    }

                    json!([{
                        "address": "0x1155000000000000000000000000000000000000",
                        "topics": log_topics,
                        "data": batch_data(),
                        "blockNumber": "0x10",
                        "transactionHash": format!("0x{}", "44".repeat(32)),
                        "logIndex": "0x0"
                    }])
                } else {
                    json!([])
                }
            }
            "eth_call" => {
                let data = request["params"][0]["data"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if data.starts_with("0x00fdd58e") {
                    if data.contains(OWNER_ADDRESS) && data.ends_with(&abi_word(0xa1)) {
                        json!("0x5")
                    } else if data.contains(OWNER_ADDRESS) && data.ends_with(&abi_word(0xc3)) {
                        json!("0x7")
                    } else {
                        json!("0x0")
                    }
                } else {
                    json!("0x0")
                }
            }
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct LogRangeRequest {
    from_block: String,
    to_block: String,
}

#[derive(Clone)]
struct CursorRpcState {
    log_ranges: Arc<Mutex<Vec<LogRangeRequest>>>,
}

async fn spawn_cursor_mock_evm_provider() -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    Arc<Mutex<Vec<LogRangeRequest>>>,
) {
    fn parse_quantity(value: &str) -> Option<u64> {
        u64::from_str_radix(
            value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .unwrap_or(value),
            16,
        )
        .ok()
    }

    fn transfer_log(
        block: u64,
        token_address: &str,
        topics: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        json!({
            "address": token_address,
            "topics": topics,
            "data": format!("0x{}01", "0".repeat(62)),
            "blockNumber": format!("0x{block:x}"),
            "transactionHash": format!("0x{}", "55".repeat(32)),
            "logIndex": "0x0"
        })
    }

    fn rpc_response(state: &CursorRpcState, request: &serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x9"),
            "eth_getTransactionCount" => json!("0x0"),
            "eth_getBalance" => json!("0x0"),
            "eth_call" => {
                json!("0x0000000000000000000000000000000000000000000000000000000000000001")
            }
            "eth_getLogs" => {
                let filter = &request["params"][0];
                let from_block = filter["fromBlock"].as_str().unwrap_or("0x0").to_string();
                let to_block = filter["toBlock"].as_str().unwrap_or("0x0").to_string();
                state.log_ranges.lock().unwrap().push(LogRangeRequest {
                    from_block: from_block.clone(),
                    to_block: to_block.clone(),
                });
                let from = parse_quantity(&from_block).unwrap_or_default();
                let to = parse_quantity(&to_block).unwrap_or_default();
                let topics = filter["topics"].as_array().cloned().unwrap_or_default();
                if from <= 5 && to >= 5 {
                    json!([transfer_log(
                        5,
                        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                        topics
                    )])
                } else if from <= 9 && to >= 9 {
                    json!([transfer_log(
                        9,
                        "0x6b175474e89094c44da98b954eedeac495271d0f",
                        topics
                    )])
                } else {
                    json!([])
                }
            }
            other => json!({ "unsupported": other }),
        };

        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(json!(1)),
            "result": result,
        })
    }

    async fn rpc_handler(
        State(state): State<CursorRpcState>,
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
            serde_json::Value::Array(
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

    let log_ranges = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(CursorRpcState {
            log_ranges: Arc::clone(&log_ranges),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, log_ranges)
}

#[derive(Clone)]
struct ActivityRpcState {
    tip_block: u64,
    log_block: u64,
    transaction_count: u64,
}

async fn spawn_activity_mock_evm_provider(
    tip_block: u64,
    log_block: u64,
    transaction_count: u64,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn parse_quantity(value: &str) -> Option<u64> {
        u64::from_str_radix(
            value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
                .unwrap_or(value),
            16,
        )
        .ok()
    }

    fn rpc_response(state: &ActivityRpcState, request: &serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!(format!("0x{:x}", state.tip_block)),
            "eth_getTransactionCount" => json!(format!("0x{:x}", state.transaction_count)),
            "eth_getBalance" => json!("0x0"),
            "eth_call" => {
                json!("0x0000000000000000000000000000000000000000000000000000000000000001")
            }
            "eth_getLogs" => {
                let filter = &request["params"][0];
                let from_block = filter["fromBlock"].as_str().unwrap_or("0x0");
                let to_block = filter["toBlock"].as_str().unwrap_or("0x0");
                let from = parse_quantity(from_block).unwrap_or_default();
                let to = parse_quantity(to_block).unwrap_or_default();
                if from <= state.log_block && to >= state.log_block {
                    let fallback_topic = format!("0x{}", "00".repeat(32));
                    let topics = filter["topics"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .map(|topic| {
                            topic
                                .as_str()
                                .map(|value| json!(value))
                                .unwrap_or_else(|| json!(fallback_topic))
                        })
                        .collect::<Vec<_>>();
                    json!([{
                        "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                        "topics": topics,
                        "data": format!("0x{}01", "0".repeat(62)),
                        "blockNumber": format!("0x{:x}", state.log_block),
                        "transactionHash": format!("0x{}", "66".repeat(32)),
                        "logIndex": "0x0"
                    }])
                } else {
                    json!([])
                }
            }
            other => json!({ "unsupported": other }),
        };

        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(json!(1)),
            "result": result,
        })
    }

    async fn rpc_handler(
        State(state): State<ActivityRpcState>,
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
            serde_json::Value::Array(
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
        .with_state(ActivityRpcState {
            tip_block,
            log_block,
            transaction_count,
        });
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

async fn init_default_compartment(client: &reqwest::Client, addr: SocketAddr) -> String {
    let init = post_json(
        client,
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
    init_json["session_token"].as_str().unwrap().to_string()
}

async fn configure_mainnet_provider(
    client: &reqwest::Client,
    addr: SocketAddr,
    token: &str,
    rpc_addr: SocketAddr,
) {
    let api_key = post_json(
        client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(token),
    )
    .await;
    assert_eq!(api_key.status(), StatusCode::OK);

    let provider = post_json(
        client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
        }),
        Some(token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);
}

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

    let mixed_external_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "mixed-external",
            "project_account": 7,
            "provider_profile": "mainnet",
            "external_receive_xpub": "xpub-receive",
            "external_account_xpub": "xpub-account",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(mixed_external_profile.status(), StatusCode::BAD_REQUEST);

    let path_without_xpub = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "path-without-xpub",
            "project_account": 7,
            "provider_profile": "mainnet",
            "external_receive_path": "m/44'/60'/7'/1",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(path_without_xpub.status(), StatusCode::BAD_REQUEST);

    let custom_export =
        sigillum_core::derive_ethereum_xpub_control_branch_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            None,
            7,
        )
        .unwrap();
    let custom_xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "custom-control",
            "project_account": 99,
            "provider_profile": "mainnet",
            "external_receive_xpub": custom_export.receive_xpub.clone(),
            "external_receive_path": custom_export.receive_path.clone(),
            "execution_enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_xpub_profile.status(), StatusCode::OK);
    let custom_xpub_profile_json: serde_json::Value = custom_xpub_profile.json().await.unwrap();
    assert_eq!(
        custom_xpub_profile_json["profile"]["external_receive_path"],
        custom_export.receive_path
    );
    assert_eq!(
        custom_xpub_profile_json["profile"]["execution_enabled"],
        false
    );

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
    let profile_names: Vec<&str> = list_json["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|profile| profile["name"].as_str())
        .collect();
    assert!(profile_names.contains(&"custom-control"));
    assert!(profile_names.contains(&"treasury-receive"));

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

    let custom_export_resp = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/export",
        json!({ "wallet_profile": "custom-control" }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_export_resp.status(), StatusCode::OK);
    let custom_export_json: serde_json::Value = custom_export_resp.json().await.unwrap();
    assert_eq!(custom_export_json["wallet_profile"], "custom-control");
    assert_eq!(custom_export_json["project_account"], 99);
    assert_eq!(
        custom_export_json["account_path"],
        custom_export.account_path
    );
    assert_eq!(
        custom_export_json["receive_path"],
        custom_export.receive_path
    );
    assert_eq!(
        custom_export_json["receive_xpub"],
        custom_export.receive_xpub
    );

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

/// Collect lowercase word tokens from every JSON string *value*, skipping
/// `kind` discriminants (compile-time constants of the audit schema, which
/// legitimately contain words like "seed" and "wallet"). Any mnemonic leak
/// would have to travel through a dynamic string value, so scanning these
/// tokens proves the audit feed carries no mnemonic words.
fn collect_audit_value_tokens(value: &serde_json::Value, tokens: &mut HashSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
                if !token.is_empty() {
                    tokens.insert(token.to_ascii_lowercase());
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_audit_value_tokens(item, tokens);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                if key == "kind" {
                    continue;
                }
                collect_audit_value_tokens(item, tokens);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn seed_wallet_create_generates_mnemonic_and_returns_it_exactly_once() {
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

    // Create a wallet with an explicit word_count of 12.
    let create_12 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-12",
            "word_count": 12,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(create_12.status(), StatusCode::OK);
    let create_12_json: serde_json::Value = create_12.json().await.unwrap();
    assert_eq!(create_12_json["status"], "created");
    let mnemonic = create_12_json["mnemonic"].as_str().unwrap().to_string();
    assert_eq!(mnemonic.split_whitespace().count(), 12);

    // The returned phrase is valid BIP-39 and reproduces the stored profile
    // material when derived independently through sigillum-core.
    assert_eq!(
        sigillum_core::ethereum_mnemonic_word_count(&mnemonic).unwrap(),
        12
    );
    let export =
        sigillum_core::derive_ethereum_xpub_receive_branch_from_mnemonic(&mnemonic, None, 0)
            .unwrap();
    assert_eq!(
        create_12_json["profile"]["receive_xpub"],
        json!(export.receive_xpub)
    );
    let derived_first_address =
        sigillum_core::derive_ethereum_address_from_xpub(&export.receive_xpub, 0)
            .unwrap()
            .address;
    assert_eq!(
        create_12_json["profile"]["first_receive_address"],
        json!(derived_first_address)
    );
    assert_eq!(create_12_json["profile"]["word_count"], 12);
    assert_eq!(create_12_json["profile"]["account_path"], "m/44'/60'/0'");
    assert_eq!(
        create_12_json["profile"]["mnemonic_secret_key"],
        "wallet.seed.gen-12.mnemonic"
    );

    // The created profile appears in the list with the matching address.
    let list = get(&client, addr, "/api/profiles/eth-seed", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    let listed = list_json["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["name"] == "gen-12")
        .expect("created profile is listed");
    assert_eq!(
        listed["first_receive_address"],
        json!(derived_first_address)
    );

    // Creating the same name again must fail instead of overwriting.
    let duplicate = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-12",
            "word_count": 12,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    // Omitting word_count defaults to a 24-word mnemonic.
    let create_default = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-default",
            "project_account": 1,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(create_default.status(), StatusCode::OK);
    let create_default_json: serde_json::Value = create_default.json().await.unwrap();
    assert_eq!(create_default_json["status"], "created");
    let default_mnemonic = create_default_json["mnemonic"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(default_mnemonic.split_whitespace().count(), 24);
    assert_eq!(create_default_json["profile"]["word_count"], 24);
    assert_ne!(default_mnemonic, mnemonic);

    // Unsupported word counts are rejected up front.
    let invalid = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-invalid",
            "word_count": 13,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    // The audit feed records the create event with metadata only.
    let audit = get(&client, addr, "/api/audit?limit=50", Some(&token)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: serde_json::Value = audit.json().await.unwrap();
    let events = audit_json["events"].as_array().unwrap();
    let create_event = events
        .iter()
        .find(|event| {
            event["kind"] == "profiles.eth_seed_wallet.create"
                && event["details"]["name"] == "gen-12"
        })
        .expect("create audit event present");
    assert_eq!(
        create_event["details"],
        json!({
            "name": "gen-12",
            "provider_profile": "mainnet",
            "word_count": 12,
        })
    );

    // No dynamic string value anywhere in the audit feed contains any word of
    // either generated mnemonic.
    let mut value_tokens = HashSet::new();
    collect_audit_value_tokens(&audit_json, &mut value_tokens);
    for word in mnemonic
        .split_whitespace()
        .chain(default_mnemonic.split_whitespace())
    {
        assert!(
            !value_tokens.contains(word),
            "audit feed leaked mnemonic word: {word}"
        );
    }

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
            "execution_enabled": true,
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

    let disabled_wallet_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-disabled",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(disabled_wallet_profile.status(), StatusCode::OK);
    let disabled_enqueue = post_json(
        &client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-disabled",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0x1"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(disabled_enqueue.status(), StatusCode::FORBIDDEN);
    let disabled_enqueue_json: serde_json::Value = disabled_enqueue.json().await.unwrap();
    assert_eq!(
        disabled_enqueue_json["error"],
        "Wallet profile execution is disabled."
    );

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
    assert_eq!(process_json["failures_by_cause"]["provider_error"], 0);
    assert_eq!(process_json["jobs"][0]["state"], "sent");

    let list_after = get(&client, addr, "/api/queue/jobs", Some(&token)).await;
    let list_after_json: serde_json::Value = list_after.json().await.unwrap();
    assert_eq!(list_after_json["jobs"][0]["state"], "sent");
    assert_eq!(
        list_after_json["jobs"][0]["broadcast_transaction_hash_hex"],
        list_after_json["jobs"][0]["transaction_hash_hex"]
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
async fn wallet_inventory_transfer_log_cursors_resume_after_canceled_job_scan_disjoint_ranges() {
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
    let cancel = post_json(
        &client,
        addr,
        "/api/discovery/jobs/cancel",
        json!({ "id": first_json["job"]["id"] }),
        Some(&token),
    )
    .await;
    let cancel_status = cancel.status();
    let cancel_json: serde_json::Value = cancel.json().await.unwrap();
    assert_eq!(
        cancel_status,
        StatusCode::OK,
        "cancel response: {cancel_json}"
    );
    assert_eq!(cancel_json["job"]["status"], "canceled");
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
