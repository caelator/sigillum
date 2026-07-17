//! End-to-end coverage for `GET /api/events` (plan task 1.3, decision D-D):
//! authentication (401 without a valid full session), the connect-time
//! snapshot, `operation` events streamed for an async discovery scan,
//! `queue` events for enqueue + drain transitions, `status` events for
//! lock/unlock/compartment switch, and fan-out to many concurrent
//! subscribers. The passive-read idle-lock semantics are covered separately
//! in `events_idle.rs` (it needs a process-global policy override).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use tempfile::TempDir;

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const ONE_ETH_HEX: &str = "0xde0b6b3a7640000";

// ── Daemon + provider fixtures ───────────────────────────────────

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

/// Combined stub EVM provider: balance-gated for scan cancel control, and
/// full send pipeline support for queue drains.
struct StubRpcState {
    balance_calls: AtomicUsize,
    gate_at_balance_call: AtomicUsize,
    gate_release: tokio::sync::Notify,
    gate_waiting: AtomicBool,
}

async fn spawn_stub_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, Arc<StubRpcState>) {
    let state = Arc::new(StubRpcState {
        balance_calls: AtomicUsize::new(0),
        gate_at_balance_call: AtomicUsize::new(0),
        gate_release: tokio::sync::Notify::new(),
        gate_waiting: AtomicBool::new(false),
    });

    async fn rpc_handler(
        State(state): State<Arc<StubRpcState>>,
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
        let requests = body.as_array().cloned().unwrap_or_else(|| vec![body]);
        let mut responses = Vec::new();
        for request in &requests {
            let method = request["method"].as_str().unwrap_or_default();
            let result: Value = match method {
                "eth_chainId" => json!("0x1"),
                "eth_blockNumber" => json!("0x20"),
                "eth_getBalance" => {
                    let call = state.balance_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    let gate_at = state.gate_at_balance_call.load(Ordering::SeqCst);
                    if gate_at != 0 && call == gate_at {
                        let notified = state.gate_release.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        state.gate_waiting.store(true, Ordering::SeqCst);
                        // A broken test must fail, not hang forever.
                        let _ = tokio::time::timeout(Duration::from_secs(30), notified).await;
                        state.gate_waiting.store(false, Ordering::SeqCst);
                        state.gate_at_balance_call.store(0, Ordering::SeqCst);
                    }
                    json!(ONE_ETH_HEX)
                }
                "eth_getTransactionCount" => json!("0x7"),
                "eth_feeHistory" => json!({
                    "oldestBlock": "0x1",
                    "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
                    "gasUsedRatio": [0.5]
                }),
                "eth_maxPriorityFeePerGas" => json!("0x59682f00"),
                "eth_call" => json!("0x"),
                "eth_getLogs" => json!([]),
                "eth_sendRawTransaction" => {
                    let raw = request["params"][0]
                        .as_str()
                        .expect("eth_sendRawTransaction carries raw transaction hex");
                    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
                        .expect("submitted raw transaction is valid hex");
                    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
                }
                _ => json!("0x0"),
            };
            responses.push(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": result
            }));
        }
        (StatusCode::OK, Json(Value::Array(responses)))
    }

    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, state)
}

// ── SSE reader ───────────────────────────────────────────────────

/// Minimal SSE client for assertions: reads frames off the wire, skipping
/// heartbeat comments, and hands back (`event`, `data`) pairs.
struct SseReader {
    response: reqwest::Response,
    buffer: String,
}

impl SseReader {
    async fn connect(client: &reqwest::Client, addr: SocketAddr, token: &str) -> Self {
        let response = client
            .get(format!("http://{addr}/api/events"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        Self {
            response,
            buffer: String::new(),
        }
    }

    async fn next_frame(&mut self) -> Option<(String, String)> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                let mut name = None;
                let mut data_lines = Vec::new();
                for line in frame.lines() {
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    if let Some(value) = line.strip_prefix("event: ") {
                        name = Some(value.to_string());
                    } else if let Some(value) = line.strip_prefix("data: ") {
                        data_lines.push(value.to_string());
                    }
                }
                if let Some(name) = name {
                    return Some((name, data_lines.join("\n")));
                }
                continue; // heartbeat-only frame
            }
            match self.response.chunk().await.unwrap() {
                Some(chunk) => self.buffer.push_str(&String::from_utf8_lossy(&chunk)),
                None => return None,
            }
        }
    }

    /// Read frames until an event named `name` whose payload satisfies
    /// `pred` arrives; fails the test on timeout or stream end.
    async fn wait_for_event(
        &mut self,
        name: &str,
        pred: impl Fn(&Value) -> bool,
    ) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for `{name}` event"
            );
            let (event, data) = tokio::time::timeout(Duration::from_secs(15), self.next_frame())
                .await
                .expect("stream stalled")
                .expect("stream ended unexpectedly");
            if event != name {
                continue;
            }
            let data: Value = serde_json::from_str(&data).expect("event data is JSON");
            if pred(&data) {
                return data;
            }
        }
    }
}

// ── Test rig ─────────────────────────────────────────────────────

struct Rig {
    client: reqwest::Client,
    addr: SocketAddr,
    token: String,
    rpc: Arc<StubRpcState>,
    handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    _dir: TempDir,
    stealth_address: String,
    ephemeral_public_key_hex: String,
    view_tag_hex: String,
}

impl Rig {
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .client
            .post(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
    }

    async fn subscribe(&self) -> SseReader {
        let mut reader = SseReader::connect(&self.client, self.addr, &self.token).await;
        // Every connection opens with the snapshot frame.
        let snapshot = reader.wait_for_event("snapshot", |_| true).await;
        assert_eq!(snapshot["v"], json!(1));
        reader
    }

    async fn enqueue_transfer(&self) -> String {
        let (status, body) = self
            .post(
                "/api/queue/enqueue/eth-stealth-transfer",
                json!({
                    "wallet_profile": "payments-mainnet",
                    "stealth_address": self.stealth_address,
                    "ephemeral_public_key_hex": self.ephemeral_public_key_hex,
                    "view_tag_hex": self.view_tag_hex,
                    "value_wei_hex": "0x1"
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "enqueue response: {body}");
        body["job"]["id"].as_str().unwrap().to_string()
    }

    fn shutdown(self) {
        self.handle.abort();
        self.rpc_handle.abort();
    }
}

async fn spawn_rig() -> Rig {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, rpc) = spawn_stub_evm_provider().await;
    let client = reqwest::Client::new();

    let init = client
        .post(format!("http://{addr}/api/compartment/init"))
        .json(&json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }))
        .send()
        .await
        .unwrap();
    let init_json: Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let mut rig = Rig {
        client,
        addr,
        token,
        rpc,
        handle,
        rpc_handle,
        _dir: dir,
        stealth_address: String::new(),
        ephemeral_public_key_hex: String::new(),
        view_tag_hex: String::new(),
    };

    let (status, body) = rig
        .post(
            "/api/api-keys/set",
            json!({ "key": "alchemy", "value": "rpc-test-token" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "api key set: {body}");

    let (status, body) = rig
        .post(
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
        )
        .await;
    assert_eq!(status, StatusCode::OK, "provider upsert: {body}");

    let (status, body) = rig
        .post(
            "/api/profiles/eth-stealth/upsert",
            json!({
                "name": "payments-mainnet",
                "wallet": "payments",
                "short_name": "eth",
                "provider_profile": "mainnet",
                "default_destination_address": "0x1111111111111111111111111111111111111111",
                "execution_enabled": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "wallet upsert: {body}");

    let (status, export) = rig
        .post(
            "/api/wallets/eth-stealth/export",
            json!({ "wallet": "payments", "short_name": "eth" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "wallet export: {export}");

    let (status, generate) = rig
        .post(
            "/api/wallets/eth-stealth/generate",
            json!({
                "stealth_meta_address": export["stealth_meta_address"],
                "ephemeral_private_key_hex": hex::encode([7u8; 32]),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "stealth generate: {generate}");

    rig.stealth_address = generate["stealth_address"].as_str().unwrap().to_string();
    rig.ephemeral_public_key_hex = generate["ephemeral_public_key_hex"]
        .as_str()
        .unwrap()
        .to_string();
    rig.view_tag_hex = generate["view_tag_hex"].as_str().unwrap().to_string();

    let account_xpub =
        sigillum_core::derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
    let (status, body) = rig
        .post(
            "/api/profiles/eth-xpub/upsert",
            json!({
                "name": "account-xpub",
                "project_account": 0,
                "provider_profile": "mainnet",
                "external_account_xpub": account_xpub,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "xpub upsert: {body}");

    rig
}

// ── Tests ────────────────────────────────────────────────────────

/// The stream is authenticated like every other route: no token, a bogus
/// token (header or query), or a capability-scoped session are all rejected.
#[tokio::test]
async fn events_requires_a_full_session() {
    let rig = spawn_rig().await;

    for request in [
        rig.client.get(format!("http://{}/api/events", rig.addr)),
        rig.client
            .get(format!("http://{}/api/events", rig.addr))
            .bearer_auth("bogus-token"),
        rig.client.get(format!(
            "http://{}/api/events?session=bogus-token",
            rig.addr
        )),
    ] {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "unauthorized", "body: {body}");
    }

    // A capability-scoped session is not a full session: 403 with the scope
    // denial code.
    let (status, capability) = rig
        .post(
            "/api/auth/capability",
            json!({ "scopes": ["deposits:read"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "capability mint: {capability}");
    let capability_token = capability["session_token"].as_str().unwrap();
    let response = rig
        .client
        .get(format!("http://{}/api/events", rig.addr))
        .bearer_auth(capability_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "capability_scope_denied", "body: {body}");

    rig.shutdown();
}

/// The first frame on every connection is a `snapshot` with the current
/// lock status, for header- and query-token auth alike (EventSource cannot
/// set headers; see the route docs for the loopback-only trade-off).
#[tokio::test]
async fn snapshot_on_connect_via_header_and_query_token() {
    let rig = spawn_rig().await;

    // Header auth.
    let mut reader = SseReader::connect(&rig.client, rig.addr, &rig.token).await;
    let snapshot = reader.wait_for_event("snapshot", |_| true).await;
    assert_eq!(snapshot["v"], json!(1));
    assert_eq!(snapshot["locked"], json!(false));
    assert_eq!(snapshot["active_compartment_id"], json!(0));
    assert_eq!(snapshot["operations"], json!([]));

    // Query-token auth (the EventSource path).
    let response = rig
        .client
        .get(format!(
            "http://{}/api/events?session={}",
            rig.addr, rig.token
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut query_reader = SseReader {
        response,
        buffer: String::new(),
    };
    let snapshot = query_reader.wait_for_event("snapshot", |_| true).await;
    assert_eq!(snapshot["locked"], json!(false));

    rig.shutdown();
}

/// An async discovery scan streams its create/state/progress transitions as
/// `operation` events; a subscriber connecting mid-run sees the live
/// operation in its snapshot.
#[tokio::test]
async fn operation_transitions_stream_to_subscriber() {
    let rig = spawn_rig().await;
    // Park the scan inside the second balance call so the run is definitely
    // live while subscribers attach.
    rig.rpc.gate_at_balance_call.store(2, Ordering::SeqCst);

    let mut reader = rig.subscribe().await;

    let (status, scan) = rig
        .post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-xpub",
                "wallet_profile": "account-xpub",
                "provider_profile": "mainnet",
                "max_index": 3,
                "gap_limit": 10,
                "run_async": true,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    let operation_id = scan["operation"]["id"].as_str().unwrap().to_string();

    // Create transition.
    let created = reader
        .wait_for_event("operation", |data| {
            data["operation"]["id"] == json!(operation_id)
                && data["operation"]["state"] == "running"
        })
        .await;
    assert_eq!(created["v"], json!(1));
    assert_eq!(created["operation"]["kind"], "inventory_scan_evm");

    // The scan is parked mid-run: a second subscriber's snapshot lists it.
    let mut mid_run_reader = SseReader::connect(&rig.client, rig.addr, &rig.token).await;
    let snapshot = mid_run_reader
        .wait_for_event("snapshot", |data| {
            data["operations"]
                .as_array()
                .is_some_and(|ops| ops.iter().any(|op| op["id"] == json!(operation_id)))
        })
        .await;
    let listed = snapshot["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["id"] == json!(operation_id))
        .unwrap();
    assert_eq!(listed["state"], "running");

    // Cancel mid-run, then release the gate: cancel_requested, then the
    // terminal canceled transition.
    let (status, cancel) = rig
        .post(&format!("/api/operations/{operation_id}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel: {cancel}");
    reader
        .wait_for_event("operation", |data| {
            data["operation"]["id"] == json!(operation_id)
                && data["operation"]["state"] == "cancel_requested"
        })
        .await;

    rig.rpc.gate_release.notify_waiters();
    let terminal = reader
        .wait_for_event("operation", |data| {
            data["operation"]["id"] == json!(operation_id)
                && data["operation"]["state"] == "canceled"
        })
        .await;
    assert!(terminal["operation"]["completed_at_unix"].is_number());

    rig.shutdown();
}

/// Enqueue and drain transitions stream as `queue` events carrying the job
/// id and the new state.
#[tokio::test]
async fn queue_transitions_stream_to_subscriber() {
    let rig = spawn_rig().await;
    let mut reader = rig.subscribe().await;

    let job_id = rig.enqueue_transfer().await;
    let queued = reader
        .wait_for_event("queue", |data| data["job_id"] == json!(job_id))
        .await;
    assert_eq!(queued["v"], json!(1));
    assert_eq!(queued["state"], "queued");

    // Drain synchronously; the events buffer in the stream while the POST
    // runs and are read afterwards.
    let (status, drain) = rig.post("/api/queue/process", json!({})).await;
    assert_eq!(status, StatusCode::OK, "drain response: {drain}");

    for expected_state in ["prepared", "submitted_unknown", "sent"] {
        reader
            .wait_for_event("queue", |data| {
                data["job_id"] == json!(job_id) && data["state"] == json!(expected_state)
            })
            .await;
    }

    rig.shutdown();
}

/// Lock, unlock, and compartment switch stream as `status` events.
#[tokio::test]
async fn status_events_for_lock_unlock_and_compartment_switch() {
    let rig = spawn_rig().await;
    let mut reader = rig.subscribe().await;

    let (status, switched) = rig
        .post("/api/compartment/switch", json!({ "id": 0 }))
        .await;
    assert_eq!(status, StatusCode::OK, "switch: {switched}");
    let event = reader
        .wait_for_event("status", |data| data["kind"] == "compartment_switched")
        .await;
    assert_eq!(event["v"], json!(1));
    assert_eq!(event["active_compartment_id"], json!(0));

    let (status, locked) = rig.post("/api/lock", json!({})).await;
    assert_eq!(status, StatusCode::OK, "lock: {locked}");
    let event = reader
        .wait_for_event("status", |data| data["kind"] == "locked")
        .await;
    assert!(event.get("active_compartment_id").is_none());

    let (status, unlocked) = rig
        .post(
            "/api/unlock",
            json!({ "passphrase": "correct horse battery staple" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "unlock: {unlocked}");
    let event = reader
        .wait_for_event("status", |data| data["kind"] == "unlocked")
        .await;
    assert_eq!(event["active_compartment_id"], json!(0));

    rig.shutdown();
}

/// Many concurrent subscribers each receive every event.
#[tokio::test]
async fn fanout_to_eight_subscribers() {
    let rig = spawn_rig().await;
    let mut readers = Vec::new();
    for _ in 0..8 {
        readers.push(rig.subscribe().await);
    }

    let job_id = rig.enqueue_transfer().await;
    for reader in &mut readers {
        let event = reader
            .wait_for_event("queue", |data| data["job_id"] == json!(job_id))
            .await;
        assert_eq!(event["state"], "queued");
    }

    rig.shutdown();
}
