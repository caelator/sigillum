//! Integration coverage for the persisted ERC-5564 announcement-scan cursors
//! (plan task 2.6):
//!
//! * `scans_resume_incrementally_from_the_persisted_cursor` — a first scan
//!   with no stored cursor starts at `earliest`; the next `from_block`-less
//!   scan resumes at cursor+1 without re-reading old blocks (asserted on the
//!   exact `eth_getLogs` filter ranges the stub provider received).
//! * `explicit_from_block_wins_and_never_drags_the_cursor_backward` — a
//!   manual `from_block` always overrides the cursor, and a rescan of old
//!   blocks leaves the cursor at its monotonic high.
//! * `cursor_survives_a_daemon_restart` — the cursor lives in the deposits
//!   store, so a fresh daemon over the same base dir resumes from it.
//! * `reset_cursor_reanchors_the_scan_range` — `reset_cursor` drops the
//!   stored cursor, scans from the given/default range, and re-anchors.
//! * `empty_range_anchors_the_cursor_at_the_scanned_head` — a range with no
//!   announcement logs still advances the cursor (numeric `to_block`, or the
//!   chain head for the default `latest`), so empty history is not re-read.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sigillum_core::StealthHashConvention;
use tempfile::TempDir;

const DESTINATION: &str = "0x1111111111111111111111111111111111111111";
const CALLER: &str = "0x2222222222222222222222222222222222222222";
/// The stub provider's chain head (`eth_blockNumber`).
const HEAD: &str = "0x20";

// ── Daemon + mock provider fixtures ──────────────────────────────

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

#[derive(Default)]
struct RpcState {
    /// Announcement logs served for ERC-5564 announcer `eth_getLogs` filters.
    announcement_logs: std::sync::RwLock<Vec<Value>>,
    /// Every `(fromBlock, toBlock)` filter range the daemon scanned, in order.
    get_logs_ranges: std::sync::RwLock<Vec<(String, String)>>,
}

fn announcement_topic() -> String {
    use sha3::{Digest, Keccak256};
    let digest = Keccak256::digest(b"Announcement(uint256,address,address,bytes,bytes)");
    format!("0x{}", hex::encode(digest))
}

fn abi_word(value: usize) -> String {
    format!("{value:064x}")
}

fn abi_dynamic_bytes(bytes: &[u8]) -> String {
    let mut out = abi_word(bytes.len());
    let mut padded = bytes.to_vec();
    let padding = (32 - (padded.len() % 32)) % 32;
    padded.resize(padded.len() + padding, 0);
    out.push_str(&hex::encode(padded));
    out
}

fn padded_address_topic(address: &str) -> String {
    let raw = address.trim_start_matches("0x");
    format!("0x{raw:0>64}")
}

/// Craft an ERC-5564 announcer log at `block_hex` for `payment`, with
/// `metadata` as the on-chain metadata (first byte = view tag).
fn announcement_log(
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    metadata: &[u8],
    block_hex: &str,
) -> Value {
    let ephemeral_public_key = hex::decode(ephemeral_public_key_hex).unwrap();
    let first_tail = abi_dynamic_bytes(&ephemeral_public_key);
    let second_offset = 64 + first_tail.len() / 2;
    let data = format!(
        "0x{}{}{}{}",
        abi_word(64),
        abi_word(second_offset),
        first_tail,
        abi_dynamic_bytes(metadata),
    );
    json!({
        "address": sigillum_core::ERC5564_ANNOUNCER_ADDRESS,
        "topics": [
            announcement_topic(),
            format!("0x{:064x}", 1u64),
            padded_address_topic(stealth_address),
            padded_address_topic(CALLER),
        ],
        "data": data,
        "blockNumber": block_hex,
        "transactionHash": format!("0x{}", "55".repeat(32)),
        "logIndex": "0x0",
    })
}

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, Arc<RpcState>) {
    fn rpc_response(state: &RpcState, request: &Value) -> Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!(HEAD),
            "eth_getLogs" => {
                let filter = &request["params"][0];
                state.get_logs_ranges.write().unwrap().push((
                    filter["fromBlock"].as_str().unwrap_or_default().to_string(),
                    filter["toBlock"].as_str().unwrap_or_default().to_string(),
                ));
                json!(state.announcement_logs.read().unwrap().clone())
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
        State(state): State<Arc<RpcState>>,
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
                    .map(|request| rpc_response(&state, request))
                    .collect(),
            )
        } else {
            rpc_response(&state, &body)
        };
        (StatusCode::OK, Json(payload))
    }

    let state = Arc::new(RpcState::default());
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

// ── Rig ──────────────────────────────────────────────────────────

struct Rig {
    addr: SocketAddr,
    token: String,
    rpc_state: Arc<RpcState>,
    daemon_handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
}

/// Compartment + provider profile + stealth wallet profile
/// `payments-mainnet` (wallet `payments`, default destination set).
async fn spawn_rig(dir: &TempDir) -> Rig {
    let (addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle, rpc_state) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();

    let init = post_ok(
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
    let token = init["session_token"].as_str().unwrap().to_string();

    post_ok(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;
    post_ok(
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
    post_ok(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": DESTINATION,
        }),
        Some(&token),
    )
    .await;

    Rig {
        addr,
        token,
        rpc_state,
        daemon_handle,
        rpc_handle,
    }
}

impl Rig {
    async fn unlock(&self) -> String {
        let client = reqwest::Client::new();
        let unlock = post_ok(
            &client,
            self.addr,
            "/api/unlock",
            json!({ "passphrase": "correct horse battery staple" }),
            None,
        )
        .await;
        unlock["session_token"].as_str().unwrap().to_string()
    }

    async fn export_meta_address(&self, token: &str) -> String {
        let client = reqwest::Client::new();
        let export = post_ok(
            &client,
            self.addr,
            "/api/wallets/eth-stealth/export",
            json!({ "wallet": "payments", "short_name": "eth" }),
            Some(token),
        )
        .await;
        export["stealth_meta_address"].as_str().unwrap().to_string()
    }

    /// Scan with no explicit range and return the response.
    async fn scan(&self, token: &str, extra: Value) -> Value {
        let client = reqwest::Client::new();
        let mut body = json!({ "wallet_profile": "payments-mainnet" });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        post_ok(
            &client,
            self.addr,
            "/api/deposits/eth-stealth/scan-announcements",
            body,
            Some(token),
        )
        .await
    }

    fn scanned_ranges(&self) -> Vec<(String, String)> {
        self.rpc_state.get_logs_ranges.read().unwrap().clone()
    }

    fn abort(self) {
        self.daemon_handle.abort();
        self.rpc_handle.abort();
    }
}

// ── HTTP helpers ─────────────────────────────────────────────────

async fn post_ok(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> Value {
    let mut request = client.post(format!("http://{addr}{path}")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{path} failed: {:?}",
        response.text().await
    );
    response.json().await.unwrap()
}

/// One payment against the rig wallet, served as one announcement log at
/// `block_hex`.
async fn serve_payment_log(rig: &Rig, token: &str, ephemeral: [u8; 32], block_hex: &str) {
    let meta_address = rig.export_meta_address(token).await;
    let payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some(ephemeral),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    *rig.rpc_state.announcement_logs.write().unwrap() = vec![announcement_log(
        &payment.stealth_address,
        &payment.ephemeral_public_key_hex,
        &hex::decode(&payment.view_tag_hex).unwrap(),
        block_hex,
    )];
}

// ── Tests ────────────────────────────────────────────────────────

/// (a) With no stored cursor the first scan covers `earliest..latest` and
/// anchors at the highest processed log block; the next `from_block`-less
/// scan resumes exactly at cursor+1 — no old block is re-requested.
#[tokio::test]
async fn scans_resume_incrementally_from_the_persisted_cursor() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    serve_payment_log(&rig, &rig.token, [0x44u8; 32], "0x20").await;

    let first = rig.scan(&rig.token, json!({})).await;
    assert_eq!(first["from_block"], "earliest");
    assert_eq!(first["to_block"], "latest");
    assert_eq!(first["scanned"], 1);

    let second = rig.scan(&rig.token, json!({})).await;
    assert_eq!(second["from_block"], "0x21");

    assert_eq!(
        rig.scanned_ranges(),
        vec![
            ("earliest".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
        ],
        "the second scan must resume at cursor+1, never re-reading old blocks"
    );

    rig.abort();
}

/// (b) An explicit `from_block` always wins over the cursor (manual rescan),
/// and rescanning old blocks never drags the cursor backward.
#[tokio::test]
async fn explicit_from_block_wins_and_never_drags_the_cursor_backward() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    serve_payment_log(&rig, &rig.token, [0x45u8; 32], "0x20").await;

    // Seed the cursor via a default scan (anchors at the log block 0x20).
    let first = rig.scan(&rig.token, json!({})).await;
    assert_eq!(first["from_block"], "earliest");

    // Manual rescan from an older block: the explicit range is used...
    let rescan = rig.scan(&rig.token, json!({ "from_block": "0x1" })).await;
    assert_eq!(rescan["from_block"], "0x1");
    // ...but the cursor stays at its monotonic high, so the following
    // default scan still resumes at 0x21 (not at 0x2).
    let after = rig.scan(&rig.token, json!({})).await;
    assert_eq!(after["from_block"], "0x21");

    assert_eq!(
        rig.scanned_ranges(),
        vec![
            ("earliest".to_string(), "latest".to_string()),
            ("0x1".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
        ]
    );

    rig.abort();
}

/// (c) The cursor persists in the deposits store, so a restarted daemon
/// resumes from it (no `from_block` needed across restarts).
#[tokio::test]
async fn cursor_survives_a_daemon_restart() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    serve_payment_log(&rig, &rig.token, [0x46u8; 32], "0x20").await;

    let first = rig.scan(&rig.token, json!({})).await;
    assert_eq!(first["from_block"], "earliest");
    assert_eq!(rig.scanned_ranges().len(), 1);

    // Restart the daemon over the same base dir (the deposits store — and
    // with it the cursor — is on disk); the mock provider keeps serving.
    let rpc_state = rig.rpc_state.clone();
    rig.daemon_handle.abort();
    let (restarted_addr, restarted_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let restarted = Rig {
        addr: restarted_addr,
        token: String::new(),
        rpc_state,
        daemon_handle: restarted_handle,
        rpc_handle: rig.rpc_handle,
    };
    let token = restarted.unlock().await;

    let second = restarted.scan(&token, json!({})).await;
    assert_eq!(second["from_block"], "0x21");
    assert_eq!(
        restarted.scanned_ranges(),
        vec![
            ("earliest".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
        ],
        "the restarted daemon must resume from the persisted cursor"
    );

    restarted.abort();
}

/// (d) `reset_cursor` drops the stored cursor: the scan re-covers the given
/// range (or `earliest` by default) and re-anchors the cursor from it.
#[tokio::test]
async fn reset_cursor_reanchors_the_scan_range() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    serve_payment_log(&rig, &rig.token, [0x47u8; 32], "0x20").await;

    let first = rig.scan(&rig.token, json!({})).await;
    assert_eq!(first["from_block"], "earliest");
    let resumed = rig.scan(&rig.token, json!({})).await;
    assert_eq!(resumed["from_block"], "0x21");

    // Reset without an explicit range: back to `earliest`, re-anchor at the
    // processed log block.
    let reset = rig.scan(&rig.token, json!({ "reset_cursor": true })).await;
    assert_eq!(reset["from_block"], "earliest");
    let after = rig.scan(&rig.token, json!({})).await;
    assert_eq!(after["from_block"], "0x21");

    assert_eq!(
        rig.scanned_ranges(),
        vec![
            ("earliest".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
            ("earliest".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
        ]
    );

    rig.abort();
}

/// (e) A range with no announcement logs at all still advances the cursor —
/// anchored at the chain head for the default `latest` upper bound — so
/// empty history is never re-read.
#[tokio::test]
async fn empty_range_anchors_the_cursor_at_the_scanned_head() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    // No logs served: the scan covers `earliest..latest` (= head 0x20) and
    // finds nothing.
    let first = rig.scan(&rig.token, json!({})).await;
    assert_eq!(first["from_block"], "earliest");
    assert_eq!(first["scanned"], 0);

    let second = rig.scan(&rig.token, json!({})).await;
    assert_eq!(
        second["from_block"], "0x21",
        "an empty scan must anchor the cursor at the chain head ({HEAD})"
    );

    assert_eq!(
        rig.scanned_ranges(),
        vec![
            ("earliest".to_string(), "latest".to_string()),
            ("0x21".to_string(), "latest".to_string()),
        ]
    );

    rig.abort();
}
