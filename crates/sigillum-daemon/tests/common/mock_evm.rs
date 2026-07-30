use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::json;

#[derive(Clone)]
struct RpcState;

pub(crate) async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
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
            "eth_sendRawTransaction" => super::submitted_raw_transaction_hash(request),
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

pub(crate) async fn spawn_erc1155_batch_mock_evm_provider()
-> (SocketAddr, tokio::task::JoinHandle<()>) {
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
pub(crate) struct LogRangeRequest {
    pub(crate) from_block: String,
    pub(crate) to_block: String,
}

#[derive(Clone)]
struct CursorRpcState {
    log_ranges: Arc<Mutex<Vec<LogRangeRequest>>>,
}

pub(crate) async fn spawn_cursor_mock_evm_provider() -> (
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

pub(crate) async fn spawn_activity_mock_evm_provider(
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
