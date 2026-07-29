mod common;

use common::{get, post_json, spawn_daemon, submitted_raw_transaction_hash};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;

const DEFAULT_DESTINATION: &str = "0x1111111111111111111111111111111111111111";
const RPC_TOKEN: &str = "rpc-test-token";
const RPC_AUTH: &str = "Bearer rpc-test-token";
const EXECUTION_PAUSED_REASON: &str =
    "execution_paused: queue execution is paused by the operator kill switch";

async fn spawn_daemon_with_state(
    base_dir: PathBuf,
) -> (
    SocketAddr,
    Arc<sigillum_daemon::AppState>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state, handle)
}

#[derive(Clone, Default)]
struct BroadcastControl {
    first_broadcast_started: Arc<Notify>,
    release_first_broadcast: Arc<Notify>,
    broadcast_count: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct RpcState {
    broadcast_control: Option<BroadcastControl>,
}

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_mock_evm_provider_with_state(RpcState {
        broadcast_control: None,
    })
    .await
}

async fn spawn_holding_mock_evm_provider()
-> (SocketAddr, tokio::task::JoinHandle<()>, BroadcastControl) {
    let control = BroadcastControl::default();
    let (addr, handle) = spawn_mock_evm_provider_with_state(RpcState {
        broadcast_control: Some(control.clone()),
    })
    .await;
    (addr, handle, control)
}

async fn spawn_mock_evm_provider_with_state(
    state: RpcState,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
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
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                responses.push(rpc_response(&state, request).await);
            }
            Value::Array(responses)
        } else {
            rpc_response(&state, &body).await
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

async fn rpc_response(state: &RpcState, request: &Value) -> Value {
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
            if to == "0x1111111111111111111111111111111111111111" && data.starts_with("0x2e7ba6ef")
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
            let is_erc1155_filter = is_nft_filter && event_topic != ERC20_OR_ERC721_TRANSFER_TOPIC;
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
        "eth_sendRawTransaction" => {
            if let Some(control) = state.broadcast_control.as_ref() {
                let broadcast_index = control.broadcast_count.fetch_add(1, Ordering::SeqCst);
                if broadcast_index == 0 {
                    control.first_broadcast_started.notify_one();
                    control.release_first_broadcast.notified().await;
                }
            }
            submitted_raw_transaction_hash(request)
        }
        other => json!({ "unsupported": other }),
    };

    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(json!(1)),
        "result": result,
    })
}

struct StealthSetup {
    token: String,
    stealth_address: Value,
    ephemeral_public_key_hex: Value,
    view_tag_hex: Value,
}

async fn setup_stealth_queue(
    client: &reqwest::Client,
    addr: SocketAddr,
    rpc_addr: SocketAddr,
) -> StealthSetup {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let key = post_json(
        client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": RPC_TOKEN }),
        Some(&token),
    )
    .await;
    assert_eq!(key.status(), StatusCode::OK);

    let provider_profile = post_json(
        client,
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
        client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": DEFAULT_DESTINATION,
            "execution_enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(wallet_profile.status(), StatusCode::OK);

    let export = post_json(
        client,
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
    let export_json: Value = export.json().await.unwrap();

    let generate = post_json(
        client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": export_json["stealth_meta_address"],
            "ephemeral_private_key_hex": hex::encode([7u8; 32]),
        }),
        None,
    )
    .await;
    assert_eq!(generate.status(), StatusCode::OK);
    let generate_json: Value = generate.json().await.unwrap();

    StealthSetup {
        token,
        stealth_address: generate_json["stealth_address"].clone(),
        ephemeral_public_key_hex: generate_json["ephemeral_public_key_hex"].clone(),
        view_tag_hex: generate_json["view_tag_hex"].clone(),
    }
}

async fn enqueue_stealth_transfer(
    client: &reqwest::Client,
    addr: SocketAddr,
    setup: &StealthSetup,
) -> Value {
    let enqueue = post_json(
        client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": setup.stealth_address,
            "ephemeral_public_key_hex": setup.ephemeral_public_key_hex,
            "view_tag_hex": setup.view_tag_hex,
            "value_wei_hex": "0xde0b6b3a7640000"
        }),
        Some(&setup.token),
    )
    .await;
    assert_eq!(enqueue.status(), StatusCode::OK);
    enqueue.json().await.unwrap()
}

async fn process_queue(client: &reqwest::Client, addr: SocketAddr, token: &str) -> Value {
    let process = post_json(client, addr, "/api/queue/process", json!({}), Some(token)).await;
    assert_eq!(process.status(), StatusCode::OK);
    process.json().await.unwrap()
}

fn assert_sent_process_response(process_json: &Value) {
    assert_eq!(process_json["succeeded"], json!(1));
    assert_eq!(process_json["jobs"][0]["state"], json!("sent"));
    assert_eq!(
        process_json["jobs"][0]["broadcast_transaction_hash_hex"],
        process_json["jobs"][0]["transaction_hash_hex"]
    );
    assert!(
        !process_json
            .as_object()
            .expect("process response is object")
            .contains_key("paused_reason")
    );
}

async fn execution_gate_events(
    client: &reqwest::Client,
    addr: SocketAddr,
    token: &str,
) -> Vec<Value> {
    let audit = get(
        client,
        addr,
        "/api/audit?limit=100&kind=treasury.policy.execution_gate.update",
        Some(token),
    )
    .await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: Value = audit.json().await.unwrap();
    let mut events = audit_json["events"].as_array().unwrap().clone();
    events.reverse();
    events
}

fn assert_session_fingerprint(value: &Value, token: &str) {
    let fingerprint = value.as_str().expect("session_fingerprint_hex is a string");
    assert_eq!(fingerprint.len(), 16);
    assert!(fingerprint.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(!token.contains(fingerprint));
}

#[tokio::test]
async fn gates_off_no_policy_keeps_stealth_queue_behavior() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;

    enqueue_stealth_transfer(&client, addr, &setup).await;
    let process_json = process_queue(&client, addr, &setup.token).await;
    assert_sent_process_response(&process_json);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn gates_off_with_policy_keeps_stealth_queue_behavior() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;

    let update = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allowed_destinations": [{ "address": DEFAULT_DESTINATION }],
        }),
        Some(&setup.token),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    enqueue_stealth_transfer(&client, addr, &setup).await;
    let process_json = process_queue(&client, addr, &setup.token).await;
    assert_sent_process_response(&process_json);

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn pause_halts_drain_mid_queue() {
    let dir = TempDir::new().unwrap();
    let base_dir = dir.path().to_path_buf();
    let (addr, state, handle) = spawn_daemon_with_state(base_dir.clone()).await;
    let (rpc_addr, rpc_handle, broadcast_control) = spawn_holding_mock_evm_provider().await;
    let client = reqwest::Client::new();
    let setup = setup_stealth_queue(&client, addr, rpc_addr).await;
    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({ "enabled": false }),
        Some(&setup.token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    for _ in 0..3 {
        enqueue_stealth_transfer(&client, addr, &setup).await;
    }
    let list_before = get(&client, addr, "/api/queue/jobs", Some(&setup.token)).await;
    assert_eq!(list_before.status(), StatusCode::OK);
    let list_before_json: Value = list_before.json().await.unwrap();
    let before_jobs = list_before_json["jobs"].as_array().unwrap().clone();
    assert_eq!(before_jobs.len(), 3);

    let process_task = {
        let client = client.clone();
        let token = setup.token.clone();
        tokio::spawn(async move {
            post_json(&client, addr, "/api/queue/process", json!({}), Some(&token)).await
        })
    };

    tokio::time::timeout(
        Duration::from_secs(5),
        broadcast_control.first_broadcast_started.notified(),
    )
    .await
    .expect("the first broadcast should reach the held mock RPC");

    let pause_task = {
        let client = client.clone();
        let token = setup.token.clone();
        tokio::spawn(async move {
            post_json(&client, addr, "/api/queue/pause", json!({}), Some(&token)).await
        })
    };

    tokio::time::timeout(Duration::from_secs(5), async {
        while !state.queue_execution_pause_latched() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the HTTP pause request should latch before the active drain releases its mutex");

    broadcast_control.release_first_broadcast.notify_one();

    let process = process_task.await.expect("queue process task should join");
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(process_json["processed"], json!(1));
    assert_eq!(process_json["succeeded"], json!(1));
    assert_eq!(
        process_json["paused_reason"],
        json!(EXECUTION_PAUSED_REASON)
    );

    let pause = pause_task.await.expect("queue pause task should join");
    assert_eq!(pause.status(), StatusCode::OK);
    let pause_json: Value = pause.json().await.unwrap();
    assert_eq!(pause_json["status"], json!("paused"));
    assert_eq!(pause_json["execution_paused"], json!(true));
    assert_eq!(broadcast_control.broadcast_count.load(Ordering::SeqCst), 1);

    let list_after = get(&client, addr, "/api/queue/jobs", Some(&setup.token)).await;
    assert_eq!(list_after.status(), StatusCode::OK);
    let list_after_json: Value = list_after.json().await.unwrap();
    let after_jobs = list_after_json["jobs"].as_array().unwrap();
    assert_eq!(after_jobs.len(), 3);
    assert_eq!(after_jobs[0]["state"], json!("sent"));
    assert_eq!(
        after_jobs[0]["broadcast_transaction_hash_hex"],
        after_jobs[0]["transaction_hash_hex"]
    );
    for index in [1usize, 2] {
        assert_eq!(after_jobs[index]["state"], json!("queued"));
        assert_eq!(after_jobs[index]["attempts"], json!(0));
        assert_eq!(
            after_jobs[index]["updated_at_unix"],
            before_jobs[index]["updated_at_unix"]
        );
    }

    let paused_policy = get(&client, addr, "/api/treasury/policy", Some(&setup.token)).await;
    assert_eq!(paused_policy.status(), StatusCode::OK);
    let paused_policy_json: Value = paused_policy.json().await.unwrap();
    assert_eq!(
        paused_policy_json["policy"]["execution_paused"],
        json!(true)
    );

    handle.abort();
    let _ = handle.await;
    drop(state);

    let (restarted_addr, restarted_state, restarted_handle) =
        spawn_daemon_with_state(base_dir).await;
    assert!(restarted_state.queue_execution_pause_latched());

    let unlock = post_json(
        &client,
        restarted_addr,
        "/api/unlock",
        json!({ "passphrase": "correct horse battery staple" }),
        None,
    )
    .await;
    assert_eq!(unlock.status(), StatusCode::OK);
    let unlock_json: Value = unlock.json().await.unwrap();
    let restarted_token = unlock_json["session_token"]
        .as_str()
        .expect("restart unlock returns a session token")
        .to_string();

    let still_paused = process_queue(&client, restarted_addr, &restarted_token).await;
    assert_eq!(still_paused["processed"], json!(0));
    assert_eq!(
        still_paused["paused_reason"],
        json!(EXECUTION_PAUSED_REASON)
    );
    assert_eq!(broadcast_control.broadcast_count.load(Ordering::SeqCst), 1);

    let resume = post_json(
        &client,
        restarted_addr,
        "/api/queue/resume",
        json!({}),
        Some(&restarted_token),
    )
    .await;
    assert_eq!(resume.status(), StatusCode::OK);
    let resume_json: Value = resume.json().await.unwrap();
    assert_eq!(resume_json["status"], json!("resumed"));
    assert_eq!(resume_json["execution_paused"], json!(false));
    assert!(!restarted_state.queue_execution_pause_latched());

    let resumed_policy = get(
        &client,
        restarted_addr,
        "/api/treasury/policy",
        Some(&restarted_token),
    )
    .await;
    assert_eq!(resumed_policy.status(), StatusCode::OK);
    let resumed_policy_json: Value = resumed_policy.json().await.unwrap();
    assert_eq!(
        resumed_policy_json["policy"]["execution_paused"],
        json!(false)
    );

    let resumed_process = process_queue(&client, restarted_addr, &restarted_token).await;
    assert_eq!(resumed_process["processed"], json!(2));
    assert_eq!(resumed_process["succeeded"], json!(2));
    assert!(
        !resumed_process
            .as_object()
            .expect("process response is object")
            .contains_key("paused_reason")
    );
    assert_eq!(broadcast_control.broadcast_count.load(Ordering::SeqCst), 3);

    let final_jobs = get(
        &client,
        restarted_addr,
        "/api/queue/jobs",
        Some(&restarted_token),
    )
    .await;
    assert_eq!(final_jobs.status(), StatusCode::OK);
    let final_jobs_json: Value = final_jobs.json().await.unwrap();
    for job in final_jobs_json["jobs"].as_array().unwrap() {
        assert_eq!(job["state"], json!("sent"));
        assert_eq!(
            job["broadcast_transaction_hash_hex"],
            job["transaction_hash_hex"]
        );
    }

    restarted_handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn pause_and_resume_routes_flip_policy_with_audit() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let pause = post_json(&client, addr, "/api/queue/pause", json!({}), Some(&token)).await;
    assert_eq!(pause.status(), StatusCode::OK);
    let pause_json: Value = pause.json().await.unwrap();
    assert_eq!(pause_json["status"], json!("paused"));
    assert_eq!(pause_json["execution_paused"], json!(true));

    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["enabled"], json!(false));
    assert_eq!(policy_json["policy"]["execution_paused"], json!(true));

    let pause_again = post_json(&client, addr, "/api/queue/pause", json!({}), Some(&token)).await;
    assert_eq!(pause_again.status(), StatusCode::OK);

    let resume = post_json(&client, addr, "/api/queue/resume", json!({}), Some(&token)).await;
    assert_eq!(resume.status(), StatusCode::OK);
    let resume_json: Value = resume.json().await.unwrap();
    assert_eq!(resume_json["status"], json!("resumed"));
    assert_eq!(resume_json["execution_paused"], json!(false));

    let events = execution_gate_events(&client, addr, &token).await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["details"]["gate"], json!("execution_paused"));
    assert_eq!(events[0]["details"]["old_value"], json!(false));
    assert_eq!(events[0]["details"]["new_value"], json!(true));
    assert_eq!(events[1]["details"]["gate"], json!("execution_paused"));
    assert_eq!(events[1]["details"]["old_value"], json!(true));
    assert_eq!(events[1]["details"]["new_value"], json!(false));
    assert_session_fingerprint(&events[0]["details"]["session_fingerprint_hex"], &token);
    assert_session_fingerprint(&events[1]["details"]["session_fingerprint_hex"], &token);

    handle.abort();
}

#[tokio::test]
async fn policy_update_gate_flips_emit_audit_events() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let enable = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "allow_revoke_execution": true,
            "allow_exit_execution": true,
            "allow_claim_execution": true,
            "allow_gas_topups": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(enable.status(), StatusCode::OK);

    let disable = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({ "enabled": true }),
        Some(&token),
    )
    .await;
    assert_eq!(disable.status(), StatusCode::OK);

    let events = execution_gate_events(&client, addr, &token).await;
    assert_eq!(events.len(), 12);
    let gate_names = [
        "allow_plan_execution",
        "allow_sweep_execution",
        "allow_revoke_execution",
        "allow_exit_execution",
        "allow_claim_execution",
        "allow_gas_topups",
    ];
    for (event, gate) in events.iter().take(6).zip(gate_names) {
        assert_eq!(event["details"]["gate"], json!(gate));
        assert_eq!(event["details"]["old_value"], json!(false));
        assert_eq!(event["details"]["new_value"], json!(true));
    }
    for (event, gate) in events.iter().skip(6).zip(gate_names) {
        assert_eq!(event["details"]["gate"], json!(gate));
        assert_eq!(event["details"]["old_value"], json!(true));
        assert_eq!(event["details"]["new_value"], json!(false));
    }

    handle.abort();
}

#[tokio::test]
async fn policy_update_preserves_pause_when_omitted() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let pause = post_json(&client, addr, "/api/queue/pause", json!({}), Some(&token)).await;
    assert_eq!(pause.status(), StatusCode::OK);

    let update = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({ "enabled": true }),
        Some(&token),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);

    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["execution_paused"], json!(true));
    let events = execution_gate_events(&client, addr, &token).await;
    let pause_events = events
        .iter()
        .filter(|event| event["details"]["gate"] == "execution_paused")
        .collect::<Vec<_>>();
    assert_eq!(pause_events.len(), 1);
    assert_eq!(pause_events[0]["details"]["old_value"], json!(false));
    assert_eq!(pause_events[0]["details"]["new_value"], json!(true));

    let explicit_resume = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "execution_paused": false,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(explicit_resume.status(), StatusCode::OK);
    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["execution_paused"], json!(false));
    let events = execution_gate_events(&client, addr, &token).await;
    let pause_events = events
        .iter()
        .filter(|event| event["details"]["gate"] == "execution_paused")
        .collect::<Vec<_>>();
    assert_eq!(pause_events.len(), 2);
    assert_eq!(pause_events[1]["details"]["old_value"], json!(true));
    assert_eq!(pause_events[1]["details"]["new_value"], json!(false));

    handle.abort();
}

#[tokio::test]
async fn pause_and_resume_require_session() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let pause = post_json(&client, addr, "/api/queue/pause", json!({}), None).await;
    assert_eq!(pause.status(), StatusCode::UNAUTHORIZED);
    let resume = post_json(&client, addr, "/api/queue/resume", json!({}), None).await;
    assert_eq!(resume.status(), StatusCode::UNAUTHORIZED);
    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert!(policy_json["policy"].is_null());

    handle.abort();
}

#[tokio::test]
async fn max_fee_per_gas_cap_hex_is_validated() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let invalid = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "max_fee_per_gas_cap_hex": "not-hex",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let valid = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "max_fee_per_gas_cap_hex": "0x77359400",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(valid.status(), StatusCode::OK);
    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(
        policy_json["policy"]["max_fee_per_gas_cap_hex"],
        json!("0x77359400")
    );

    handle.abort();
}

#[tokio::test]
async fn treasury_policy_cap_fields_reject_prefixless_hex() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    for field in [
        "max_step_native_wei_hex",
        "max_plan_native_wei_hex",
        "max_gas_topup_wei_hex",
        "max_fee_per_gas_cap_hex",
    ] {
        let mut invalid_body = json!({
            "enabled": true,
        });
        invalid_body[field] = json!("1000");
        let invalid = post_json(
            &client,
            addr,
            "/api/treasury/policy/update",
            invalid_body,
            Some(&token),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let mut valid_body = json!({
            "enabled": true,
        });
        valid_body[field] = json!("0x1000");
        let valid = post_json(
            &client,
            addr,
            "/api/treasury/policy/update",
            valid_body,
            Some(&token),
        )
        .await;
        assert_eq!(valid.status(), StatusCode::OK);
    }

    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(
        policy_json["policy"]["max_fee_per_gas_cap_hex"],
        json!("0x1000")
    );

    handle.abort();
}

#[tokio::test]
async fn treasury_policy_required_fields_reject_prefixless_hex() {
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
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let invalid_floor = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "1000",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_floor.status(), StatusCode::BAD_REQUEST);

    let invalid_target = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_target_wei_hex": "1000",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid_target.status(), StatusCode::BAD_REQUEST);

    let valid = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "hot_floor_wei_hex": "0x1000",
            "hot_target_wei_hex": "0x2000",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(valid.status(), StatusCode::OK);

    let policy = get(&client, addr, "/api/treasury/policy", Some(&token)).await;
    assert_eq!(policy.status(), StatusCode::OK);
    let policy_json: Value = policy.json().await.unwrap();
    assert_eq!(policy_json["policy"]["hot_floor_wei_hex"], json!("0x1000"));
    assert_eq!(policy_json["policy"]["hot_target_wei_hex"], json!("0x2000"));

    handle.abort();
}
