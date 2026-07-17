//! `SIGILLUM_SCHEDULER_DISABLE=1` must produce zero background activity
//! (plan task 1.6, scenario c).
//!
//! This lives in its own test binary because it overrides
//! `SIGILLUM_SCHEDULER_DISABLE` (and the tick intervals) process-wide before
//! the daemon's startup config is read — the env would conflict with the
//! scheduler-enabled tests in `scheduler.rs` (the `events_idle.rs`
//! precedent). Keep it the ONLY test in this file.

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
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const DEFAULT_DESTINATION: &str = "0x1111111111111111111111111111111111111111";
const RPC_TOKEN: &str = "rpc-test-token";
const RPC_AUTH: &str = "Bearer rpc-test-token";

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
    // The exact spawn call the production entry point makes; with
    // SIGILLUM_SCHEDULER_DISABLE set it must produce no background task.
    sigillum_daemon::spawn_scheduler(state.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state, handle)
}

#[derive(Clone, Default)]
struct RpcControl {
    broadcast_count: Arc<AtomicUsize>,
}

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, RpcControl) {
    async fn rpc_handler(
        State(control): State<RpcControl>,
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
                responses.push(rpc_response(&control, request));
            }
            Value::Array(responses)
        } else {
            rpc_response(&control, &body)
        };
        (StatusCode::OK, Json(payload))
    }

    fn rpc_response(control: &RpcControl, request: &Value) -> Value {
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
            "eth_call" => json!("0x0f4240"),
            "eth_getLogs" => json!([]),
            "eth_sendRawTransaction" => {
                control.broadcast_count.fetch_add(1, Ordering::SeqCst);
                let raw = request["params"][0]
                    .as_str()
                    .expect("eth_sendRawTransaction carries raw transaction hex");
                let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
                    .expect("submitted raw transaction is valid hex");
                json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
            }
            other => json!({ "unsupported": other }),
        };
        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(json!(1)),
            "result": result,
        })
    }

    let control = RpcControl::default();
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(control.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, control)
}

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn disabled_scheduler_produces_no_background_activity() {
    // Must land before `build_router` reads the startup config: the loop is
    // disabled even though the tick is configured aggressively short.
    unsafe {
        std::env::set_var("SIGILLUM_SCHEDULER_DISABLE", "1");
        std::env::set_var("SIGILLUM_SCHEDULER_QUEUE_TICK_SECS", "1");
        std::env::set_var("SIGILLUM_SCHEDULER_REFRESH_SECS", "1");
    }

    let dir = TempDir::new().unwrap();
    let (addr, state, handle) = spawn_daemon_with_state(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, control) = spawn_mock_evm_provider().await;
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

    let key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": RPC_TOKEN }),
        Some(&token),
    )
    .await;
    assert_eq!(key.status(), StatusCode::OK);

    // Plan task 2.5: stealth transfers/sweeps gate under the Sweep execution
    // family — the enqueue below needs an enabled policy with the master +
    // sweep gates open and the default destination allow-listed.
    let policy = post_json(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "allowed_destinations": [{ "address": DEFAULT_DESTINATION }],
        }),
        Some(&token),
    )
    .await;
    assert_eq!(policy.status(), StatusCode::OK);

    for (path, body) in [
        (
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
        ),
        (
            "/api/profiles/eth-stealth/upsert",
            json!({
                "name": "payments-mainnet",
                "wallet": "payments",
                "short_name": "eth",
                "provider_profile": "mainnet",
                "default_destination_address": DEFAULT_DESTINATION,
                "execution_enabled": true,
            }),
        ),
    ] {
        let response = post_json(&client, addr, path, body, Some(&token)).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({ "wallet": "payments", "short_name": "eth" }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: Value = export.json().await.unwrap();
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
    assert_eq!(generate.status(), StatusCode::OK);
    let generate_json: Value = generate.json().await.unwrap();

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

    // Several would-be ticks pass: with the loop disabled nothing advances,
    // no operation is registered, and the status block reports the disabled
    // configuration with no tick history.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let status = state.scheduler_status();
    assert!(!status.enabled, "{status:?}");
    assert_eq!(status.queue_tick_secs, 1, "{status:?}");
    assert_eq!(status.last_tick_at_unix, None, "{status:?}");
    assert_eq!(status.last_cycle_outcome, None, "{status:?}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 0);
    assert!(state.list_operations(50).is_empty());

    let jobs = client
        .get(format!("http://{addr}/api/queue/jobs"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let job = &jobs["jobs"][0];
    assert_eq!(job["state"], json!("queued"), "{job}");
    assert_eq!(job["attempts"], json!(0), "{job}");

    // Sanity: the request-driven drain path itself is unaffected.
    let process = post_json(&client, addr, "/api/queue/process", json!({}), Some(&token)).await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: Value = process.json().await.unwrap();
    assert_eq!(process_json["succeeded"], json!(1), "{process_json}");
    assert_eq!(control.broadcast_count.load(Ordering::SeqCst), 1);

    handle.abort();
    rpc_handle.abort();
}
