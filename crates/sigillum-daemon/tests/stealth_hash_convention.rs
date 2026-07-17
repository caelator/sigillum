//! Integration coverage for the ERC-5564 shared-secret hash-convention switch
//! (compressed-point scheme-1 standard, with dual-decode of legacy x-only
//! payments):
//!
//! * `legacy_deposit_record_detected_and_sweepable_after_migration` — a
//!   deposit record written in the pre-switch (schema v2) shape is stamped
//!   `x32` by the store migration, is still detected by the dual-decode check
//!   endpoint, and stays sweepable; a wrong/corrupt convention stamp is
//!   rescued by the signing-side probe and corrected by the check endpoint.
//! * `new_deposit_roundtrips_on_standard_convention` — generation and deposit
//!   creation write the standard `compressed33` convention and sweep with it.
//! * `announcement_scan_finds_both_conventions_in_one_pass` — a single
//!   announcer scan with view-tag prefiltering matches a standard and a
//!   legacy announcement while skipping tag noise.
//! * `watch_only_detection_requires_unlocked_compartment` — detection is
//!   watch-only (viewing key + spending public key; the spending private key
//!   never enters the scan/check path) but still requires the compartment
//!   unlocked: after `/api/lock` zeroizes master keys and clears sessions
//!   there is no viewing-key cache, so scan and check are rejected until
//!   unlock (service layer: `vault_locked`).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};
use sigillum_core::StealthHashConvention;
use tempfile::TempDir;

const DESTINATION: &str = "0x1111111111111111111111111111111111111111";
const CALLER: &str = "0x2222222222222222222222222222222222222222";

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
    /// Announcement logs returned for ERC-5564 announcer `eth_getLogs` filters.
    announcement_logs: std::sync::RwLock<Vec<Value>>,
}

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

async fn spawn_mock_evm_provider() -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    Arc<RpcState>,
) {
    fn rpc_response(state: &RpcState, request: &Value) -> Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_getLogs" => {
                let filter = &request["params"][0];
                let address = filter["address"].as_str().unwrap_or_default();
                if address.eq_ignore_ascii_case(sigillum_core::ERC5564_ANNOUNCER_ADDRESS) {
                    json!(state.announcement_logs.read().unwrap().clone())
                } else {
                    json!([])
                }
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

// ── HTTP helpers ─────────────────────────────────────────────────

async fn post_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = client
        .post(format!("http://{addr}{path}"))
        .json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

async fn post_ok(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> Value {
    let response = post_json(client, addr, path, body, token).await;
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "POST {path} failed: {body}");
    body
}

async fn get_ok(client: &reqwest::Client, addr: SocketAddr, path: &str, token: &str) -> Value {
    let response = client
        .get(format!("http://{addr}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "GET {path} failed: {body}");
    body
}

// ── ERC-5564 announcement log crafting ───────────────────────────

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

fn announcement_topic() -> String {
    let digest = Keccak256::digest(b"Announcement(uint256,address,address,bytes,bytes)");
    format!("0x{}", hex::encode(digest))
}

/// Craft an ERC-5564 announcer log for `payment`, with `metadata` as the
/// on-chain metadata (first byte = view tag).
fn announcement_log(
    stealth_address: &str,
    ephemeral_public_key_hex: &str,
    metadata: &[u8],
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
        "blockNumber": "0x20",
        "transactionHash": format!("0x{}", "55".repeat(32)),
        "logIndex": "0x0",
    })
}

// ── Rig ──────────────────────────────────────────────────────────

struct Rig {
    addr: SocketAddr,
    token: String,
    rpc_state: Arc<RpcState>,
    daemon_handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
}

/// Compartment + provider profile (with static fees) + stealth wallet profile
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
    // Plan task 2.5: stealth sweeps gate under the Sweep execution family —
    // every sweep in this suite needs the master + sweep gates open and the
    // default destination allow-listed.
    post_ok(
        &client,
        addr,
        "/api/treasury/policy/update",
        json!({
            "enabled": true,
            "allow_plan_execution": true,
            "allow_sweep_execution": true,
            "allowed_destinations": [{ "address": DESTINATION }],
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
    async fn export_meta_address(&self) -> String {
        let client = reqwest::Client::new();
        let export = post_ok(
            &client,
            self.addr,
            "/api/wallets/eth-stealth/export",
            json!({ "wallet": "payments", "short_name": "eth" }),
            Some(&self.token),
        )
        .await;
        export["stealth_meta_address"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn deposit_record(&self, id: &str) -> Value {
        let client = reqwest::Client::new();
        let deposits = get_ok(&client, self.addr, "/api/deposits/eth-stealth", &self.token).await;
        deposits["deposits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|deposit| deposit["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("deposit {id} not found in {deposits}"))
    }
}

/// v2 (pre-convention-switch) record JSON: no `stealth_hash_convention` field.
fn legacy_v2_record_json(
    id: &str,
    stealth_meta_address: &str,
    payment: &sigillum_core::EthereumStealthPayment,
) -> Value {
    json!({
        "id": id,
        "status": "pending",
        "asset_kind": "native",
        "wallet_profile": "payments-mainnet",
        "chain_id": 1,
        "wallet_compartment_id": 0,
        "provider_compartment_id": 0,
        "wallet": "payments",
        "short_name": "eth",
        "stealth_meta_address": stealth_meta_address,
        "stealth_address": payment.stealth_address,
        "ephemeral_public_key_hex": payment.ephemeral_public_key_hex,
        // Pre-switch records stored the view tag unprefixed (raw hex).
        "view_tag_hex": payment.view_tag_hex,
        "auto_queue_sweep": false,
        "created_at_unix": 1,
        "updated_at_unix": 1
    })
}

fn write_deposits_file(dir: &TempDir, schema_version: u32, records: Vec<Value>) {
    let envelope = json!({
        "schema": "sigillum.deposits",
        "schema_version": schema_version,
        "data": { "eth_stealth": records },
    });
    std::fs::write(
        dir.path().join("deposits.json"),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────

/// (a) A legacy-convention deposit record written in the pre-switch (schema
/// v2) shape is stamped `x32` by the store migration, stays detectable via the
/// dual-decode check endpoint, and stays sweepable — including when its stored
/// convention stamp is wrong (corrupt), where the signing-side probe rescues
/// it and the check endpoint corrects the stamp.
#[tokio::test]
async fn legacy_deposit_record_detected_and_sweepable_after_migration() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    let meta_address = rig.export_meta_address().await;

    // The payment was made pre-switch: legacy x-only shared-secret hash.
    let legacy_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0xa5u8; 32]),
        StealthHashConvention::XOnly32,
    )
    .unwrap();

    // The store fixture uses the OLD (v2) shape: no convention field.
    write_deposits_file(
        &dir,
        2,
        vec![legacy_v2_record_json(
            "dep_legacy1",
            &meta_address,
            &legacy_payment,
        )],
    );

    // Migration on load: the record is stamped with the legacy convention.
    let record = rig.deposit_record("dep_legacy1").await;
    assert_eq!(
        record["stealth_hash_convention"], "x32",
        "v2 record must be stamped x32 by the migration: {record}"
    );

    // Detection: dual-decode check matches and reports the actual convention.
    let check = post_ok(
        &client,
        rig.addr,
        "/api/wallets/eth-stealth/check",
        json!({
            "wallet": "payments",
            "stealth_address": legacy_payment.stealth_address,
            "ephemeral_public_key_hex": legacy_payment.ephemeral_public_key_hex,
            "view_tag_hex": legacy_payment.view_tag_hex,
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(check["matches"], true);
    assert_eq!(check["stealth_hash_convention"], "x32");

    // Corrupt the stamp: a v3 record claiming the standard convention for a
    // payment that was actually made with the legacy one.
    let mut corrupt_record = legacy_v2_record_json("dep_legacy1", &meta_address, &legacy_payment);
    corrupt_record["stealth_hash_convention"] = json!("compressed33");
    write_deposits_file(&dir, 3, vec![corrupt_record]);
    let record = rig.deposit_record("dep_legacy1").await;
    assert_eq!(record["stealth_hash_convention"], "compressed33");

    // The sweep job inherits the (wrong) stamp...
    let enqueue = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": "dep_legacy1" }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(enqueue["job"]["kind"], "eth_stealth_native_sweep");
    assert_eq!(enqueue["job"]["stealth_hash_convention"], "compressed33");
    let job_id = enqueue["job"]["id"].as_str().unwrap().to_string();

    // ...but the signing-side probe falls back to the legacy convention (the
    // derived-address verification rejects the standard one), so the legacy
    // deposit is sweepable even with a corrupt stamp.
    let process = post_ok(
        &client,
        rig.addr,
        "/api/queue/process",
        json!({ "id": job_id }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(process["succeeded"], 1, "sweep failed: {process}");
    assert_eq!(process["jobs"][0]["state"], "sent");

    // A fresh dual-decode check persists the actual convention back onto the
    // record, correcting the corrupt stamp.
    let check = post_ok(
        &client,
        rig.addr,
        "/api/wallets/eth-stealth/check",
        json!({
            "wallet": "payments",
            "stealth_address": legacy_payment.stealth_address,
            "ephemeral_public_key_hex": legacy_payment.ephemeral_public_key_hex,
            "view_tag_hex": legacy_payment.view_tag_hex,
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(check["matches"], true);
    let record = rig.deposit_record("dep_legacy1").await;
    assert_eq!(
        record["stealth_hash_convention"], "x32",
        "check must re-stamp the detected convention: {record}"
    );

    // Re-sweeping (forced) now carries the corrected stamp and succeeds on
    // the primary path.
    let enqueue = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": "dep_legacy1", "force": true }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(enqueue["job"]["stealth_hash_convention"], "x32");
    let job_id = enqueue["job"]["id"].as_str().unwrap().to_string();
    let process = post_ok(
        &client,
        rig.addr,
        "/api/queue/process",
        json!({ "id": job_id }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(process["succeeded"], 1, "re-sweep failed: {process}");
    assert_eq!(process["jobs"][0]["state"], "sent");

    rig.daemon_handle.abort();
    rig.rpc_handle.abort();
}

/// (b) New writes are standard: generation and deposit creation stamp
/// `compressed33` (byte-identical to the core ScopeLift-compatible
/// derivation), and the deposit sweeps on the standard convention.
#[tokio::test]
async fn new_deposit_roundtrips_on_standard_convention() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    let meta_address = rig.export_meta_address().await;

    // Bare generation: standard convention, byte-identical to the core
    // compressed-point derivation (and distinct from the legacy one).
    let generate = post_ok(
        &client,
        rig.addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": meta_address,
            "ephemeral_private_key_hex": hex::encode([0x0bu8; 32]),
        }),
        None,
    )
    .await;
    assert_eq!(generate["stealth_hash_convention"], "compressed33");
    let standard_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x0bu8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    let legacy_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x0bu8; 32]),
        StealthHashConvention::XOnly32,
    )
    .unwrap();
    assert_eq!(
        generate["stealth_address"].as_str().unwrap(),
        standard_payment.stealth_address
    );
    assert_eq!(
        generate["view_tag_hex"].as_str().unwrap(),
        standard_payment.view_tag_hex
    );
    assert_ne!(
        generate["stealth_address"].as_str().unwrap(),
        legacy_payment.stealth_address
    );

    // Deposit creation stamps the standard convention.
    let create = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/create-native",
        json!({
            "wallet_profile": "payments-mainnet",
            "expected_value_wei_hex": "0x1",
            "auto_queue_sweep": true,
            "min_sweep_value_wei_hex": "0x1",
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(create["deposit"]["stealth_hash_convention"], "compressed33");
    let deposit_id = create["deposit"]["id"].as_str().unwrap().to_string();

    // Refresh detects the funded balance and auto-enqueues the sweep; the job
    // carries the record's standard convention.
    let refresh = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "auto_enqueue": true }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(refresh["detected"], 1);
    assert_eq!(refresh["queued"], 1);
    let deposit = refresh["deposits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|deposit| deposit["id"] == deposit_id)
        .cloned()
        .unwrap();
    assert_eq!(deposit["status"], "sweep_queued");
    let job_id = deposit["queue_job_id"].as_str().unwrap().to_string();

    let job = post_ok(
        &client,
        rig.addr,
        "/api/queue/process",
        json!({ "id": job_id }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(job["succeeded"], 1, "sweep failed: {job}");
    assert_eq!(job["jobs"][0]["state"], "sent");
    assert_eq!(job["jobs"][0]["stealth_hash_convention"], "compressed33");

    rig.daemon_handle.abort();
    rig.rpc_handle.abort();
}

/// (c) One announcer scan pass with view-tag prefiltering finds both a
/// standard-convention and a legacy-convention announcement (the legacy one's
/// on-chain view tag only matches under the legacy probe) while skipping a
/// noise log whose tag matches neither.
#[tokio::test]
async fn announcement_scan_finds_both_conventions_in_one_pass() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let meta_address = rig.export_meta_address().await;

    // One payment per convention, both addressed to the rig's wallet.
    let standard_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x11u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    let legacy_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x22u8; 32]),
        StealthHashConvention::XOnly32,
    )
    .unwrap();
    // What the legacy payment's ephemeral key would have produced under the
    // standard convention (for picking a noise tag distinct from both).
    let legacy_eph_as_standard = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x22u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();

    // Noise: same ephemeral key as the legacy payment, but a view tag that
    // matches neither convention's derived tag, so the prefilter drops it
    // before any address comparison.
    let legacy_tag = hex::decode(&legacy_payment.view_tag_hex).unwrap()[0];
    let standard_tag = hex::decode(&legacy_eph_as_standard.view_tag_hex).unwrap()[0];
    let noise_tag = (0u8..=255)
        .find(|tag| *tag != legacy_tag && *tag != standard_tag)
        .unwrap();

    let logs = vec![
        announcement_log(
            &standard_payment.stealth_address,
            &standard_payment.ephemeral_public_key_hex,
            &hex::decode(&standard_payment.view_tag_hex).unwrap(),
        ),
        announcement_log(
            &legacy_payment.stealth_address,
            &legacy_payment.ephemeral_public_key_hex,
            &hex::decode(&legacy_payment.view_tag_hex).unwrap(),
        ),
        announcement_log(
            "0x000000000000000000000000000000000000dead",
            &legacy_payment.ephemeral_public_key_hex,
            &[noise_tag],
        ),
    ];

    // Publish the logs on the mock provider.
    *rig.rpc_state.announcement_logs.write().unwrap() = logs;
    let client = reqwest::Client::new();

    let scan = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/scan-announcements",
        json!({
            "wallet_profile": "payments-mainnet",
            "from_block": "0x1",
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(scan["scanned"], 3);
    assert_eq!(scan["matched"], 2, "scan result: {scan}");
    assert_eq!(scan["created"], 2);
    assert_eq!(scan["existing"], 0);

    let deposits = scan["deposits"].as_array().unwrap();
    let standard = deposits
        .iter()
        .find(|deposit| {
            deposit["stealth_address"].as_str().unwrap() == standard_payment.stealth_address
        })
        .expect("standard-convention deposit missing");
    let legacy = deposits
        .iter()
        .find(|deposit| {
            deposit["stealth_address"].as_str().unwrap() == legacy_payment.stealth_address
        })
        .expect("legacy-convention deposit missing");
    assert_eq!(standard["stealth_hash_convention"], "compressed33");
    assert_eq!(standard["view_tag_hex"], standard_payment.view_tag_hex);
    assert_eq!(legacy["stealth_hash_convention"], "x32");
    assert_eq!(legacy["view_tag_hex"], legacy_payment.view_tag_hex);

    // A second scan over the same logs finds the same records (idempotent)
    // and leaves their conventions untouched.
    let rescan = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/scan-announcements",
        json!({
            "wallet_profile": "payments-mainnet",
            "from_block": "0x1",
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(rescan["matched"], 2);
    assert_eq!(rescan["created"], 0);
    assert_eq!(rescan["existing"], 2);
    let record = rig
        .deposit_record(legacy["id"].as_str().unwrap())
        .await;
    assert_eq!(record["stealth_hash_convention"], "x32");

    rig.daemon_handle.abort();
    rig.rpc_handle.abort();
}

/// (d) Watch-only detection still requires the compartment unlocked — and
/// there is no viewing-key cache that would let it survive a lock.
///
/// Detection (announcer scan and the check endpoint) runs from the viewing
/// private key + spending PUBLIC key only; the spending private key never
/// enters the detection path. But the viewing key itself derives from the
/// compartment master key, so locking the compartment (zeroizing master
/// keys) must make detection fail `vault_locked` — deliberately: caching the
/// viewing key across a lock would weaken the zeroize-on-lock invariant.
#[tokio::test]
async fn watch_only_detection_requires_unlocked_compartment() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    let meta_address = rig.export_meta_address().await;

    let payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x33u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    *rig.rpc_state.announcement_logs.write().unwrap() = vec![announcement_log(
        &payment.stealth_address,
        &payment.ephemeral_public_key_hex,
        &hex::decode(&payment.view_tag_hex).unwrap(),
    )];

    // Unlocked: the watch-only scan detects the payment.
    let scan = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/scan-announcements",
        json!({
            "wallet_profile": "payments-mainnet",
            "from_block": "0x1",
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(scan["matched"], 1, "scan result: {scan}");
    assert_eq!(
        scan["deposits"][0]["stealth_hash_convention"],
        "compressed33"
    );

    // Lock: master keys are zeroized and sessions cleared. No viewing-key
    // cache survives, so detection is impossible until unlock — the daemon
    // rejects both endpoints (401, no session) rather than silently keep
    // detecting.
    let lock = post_json(&client, rig.addr, "/api/lock", json!({}), Some(&rig.token)).await;
    assert_eq!(lock.status(), StatusCode::OK);

    let scan_locked = post_json(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/scan-announcements",
        json!({
            "wallet_profile": "payments-mainnet",
            "from_block": "0x1",
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(scan_locked.status(), StatusCode::UNAUTHORIZED);
    let body: Value = scan_locked.json().await.unwrap();
    assert_eq!(body["code"], "unauthorized", "scan body: {body}");

    let check_locked = post_json(
        &client,
        rig.addr,
        "/api/wallets/eth-stealth/check",
        json!({
            "wallet": "payments",
            "stealth_address": payment.stealth_address,
            "ephemeral_public_key_hex": payment.ephemeral_public_key_hex,
            "view_tag_hex": payment.view_tag_hex,
        }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(check_locked.status(), StatusCode::UNAUTHORIZED);
    let body: Value = check_locked.json().await.unwrap();
    assert_eq!(body["code"], "unauthorized", "check body: {body}");

    // Unlock again: detection is watch-only as before and matches.
    let unlock = post_ok(
        &client,
        rig.addr,
        "/api/unlock",
        json!({ "passphrase": "correct horse battery staple" }),
        None,
    )
    .await;
    let token = unlock["session_token"].as_str().unwrap().to_string();
    let check = post_ok(
        &client,
        rig.addr,
        "/api/wallets/eth-stealth/check",
        json!({
            "wallet": "payments",
            "stealth_address": payment.stealth_address,
            "ephemeral_public_key_hex": payment.ephemeral_public_key_hex,
            "view_tag_hex": payment.view_tag_hex,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(check["matches"], true);
    assert_eq!(check["stealth_hash_convention"], "compressed33");

    rig.daemon_handle.abort();
    rig.rpc_handle.abort();
}
