//! Integration coverage for the stealth deposit gas story (operator-surface
//! plan task 2.4):
//!
//! * `create_erc20_with_request_gas_produces_eip5564_token_metadata` — produce:
//!   a gas-requesting ERC-20 deposit's announcement carries the EIP-5564 token
//!   SHOULD-layout metadata (transfer selector + token contract + expected
//!   amount); the default keeps view-tag-only metadata.
//! * `create_native_with_request_gas_totals_payment_plus_gas` — produce: the
//!   native SHOULD layout totals expected value + requested gas (explicit or
//!   the provider's static sweep estimate).
//! * `scan_autopopulates_deposit_from_metadata_hints` — consume: announcements
//!   carrying token/native layout hints create deposits with the hinted asset
//!   kind, token contract, and expected amount — no operator `--token-address`.
//! * `funded_needs_gas_transitions_to_funded_when_gas_arrives` — refresh
//!   notices payer/sponsor-attached native gas on a `funded_needs_gas`
//!   deposit and moves it to `funded`.
//! * `erc20_deposit_gas_topup_then_sweep_end_to_end` — sponsor fund_gas: a
//!   gas-starved ERC-20 deposit enqueues a sponsor top-up (1.5x the sweep's
//!   estimated gas) ahead of its sweep; the sweep stays blocked until the
//!   top-up broadcasts AND the gas is visible on-chain, then sweeps.
//! * `cross_party_sponsor_funding_warns_or_hard_blocks_per_policy` — linkage:
//!   one sponsor funding deposits of DIFFERENT parties warns when
//!   `block_cross_party_linkage` is off and hard-blocks
//!   (`policy_violation: cross_party_linkage`) when on.

use std::collections::BTreeMap;
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
const DESTINATION_B: &str = "0x9999999999999999999999999999999999999999";
const CALLER: &str = "0x2222222222222222222222222222222222222222";
const TOKEN: &str = "0x2222222222222222222222222222222222222222";
/// Static fee basis of the rig's provider profile (2 gwei max fee).
const MAX_FEE_HEX: &str = "0x77359400";
/// 1.5x the ERC-20 sweep gas estimate: 2 gwei x 65_000 x 1.5 = 195e12 wei.
const EXPECTED_TOPUP_WEI_HEX: &str = "0xb159f9bb3000";

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
    /// Per-address native balance overrides (lowercased address -> wei hex);
    /// every other address reads as 1 ETH (a funded sponsor by default).
    balances: std::sync::RwLock<BTreeMap<String, String>>,
}

fn submitted_raw_transaction_hash(request: &Value) -> Value {
    let raw = request["params"][0]
        .as_str()
        .expect("eth_sendRawTransaction carries raw transaction hex");
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw))
        .expect("submitted raw transaction is valid hex");
    json!(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>, Arc<RpcState>) {
    fn rpc_response(state: &RpcState, request: &Value) -> Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => {
                let address = request["params"][0]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let balances = state.balances.read().unwrap();
                json!(balances
                    .get(&address)
                    .cloned()
                    .unwrap_or_else(|| "0xde0b6b3a7640000".to_string()))
            }
            // Any calldata carries a balanceOf-style probe: report 1_000_000
            // raw token units so ERC-20 deposits read as funded.
            "eth_call" => json!("0x0f4240"),
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
    let mut request = client.post(format!("http://{addr}{path}")).json(&body);
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

/// Craft an ERC-5564 announcer log for a payment, with `metadata` as the
/// on-chain metadata (first byte = view tag).
fn announcement_log(stealth_address: &str, ephemeral_public_key_hex: &str, metadata: &[u8]) -> Value {
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

/// Compartment + provider profile (static fees) + stealth wallet profile
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
            "max_fee_per_gas_hex": MAX_FEE_HEX,
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
        export["stealth_meta_address"].as_str().unwrap().to_string()
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

    async fn queue_jobs(&self) -> Vec<Value> {
        let client = reqwest::Client::new();
        let jobs = get_ok(&client, self.addr, "/api/queue/jobs", &self.token).await;
        jobs["jobs"].as_array().unwrap().clone()
    }

    async fn set_balance(&self, address: &str, balance_wei_hex: &str) {
        self.rpc_state
            .balances
            .write()
            .unwrap()
            .insert(address.to_ascii_lowercase(), balance_wei_hex.to_string());
    }

    async fn set_policy(&self, allow_gas_topups: bool, block_cross_party_linkage: bool) {
        let client = reqwest::Client::new();
        post_ok(
            &client,
            self.addr,
            "/api/treasury/policy/update",
            json!({
                "enabled": true,
                "allow_gas_topups": allow_gas_topups,
                "max_gas_topup_wei_hex": EXPECTED_TOPUP_WEI_HEX,
                "block_cross_party_linkage": block_cross_party_linkage,
                // Plan task 2.5: stealth sweeps gate under the same Sweep
                // family as seed sweeps — the master + sweep gates must be
                // on for any sweep in this suite to enqueue/drain. The 2.4
                // gas-topup behavior stays keyed to `allow_gas_topups` only.
                "allow_plan_execution": true,
                "allow_sweep_execution": true,
                // An enabled policy fail-closes destinations not on the
                // allowlist; both test sweep destinations are allowed.
                "allowed_destinations": [
                    { "address": DESTINATION },
                    { "address": DESTINATION_B },
                ],
            }),
            Some(&self.token),
        )
        .await;
    }

    async fn create_erc20_deposit(
        &self,
        request_gas: bool,
        auto_queue_sweep: bool,
        sweep_destination: Option<&str>,
    ) -> Value {
        let client = reqwest::Client::new();
        let create = post_ok(
            &client,
            self.addr,
            "/api/deposits/eth-stealth/create-erc20",
            json!({
                "wallet_profile": "payments-mainnet",
                "token_address": TOKEN,
                "expected_amount_hex": "0x1000",
                "auto_queue_sweep": auto_queue_sweep,
                "sweep_destination_address": sweep_destination,
                "request_gas": request_gas,
            }),
            Some(&self.token),
        )
        .await;
        create["deposit"].clone()
    }

    fn abort(self) {
        self.daemon_handle.abort();
        self.rpc_handle.abort();
    }
}

fn metadata_hints(metadata_hex: &str) -> sigillum_core::Erc5564MetadataHints {
    let raw = hex::decode(metadata_hex.strip_prefix("0x").unwrap_or(metadata_hex)).unwrap();
    sigillum_core::decode_erc5564_metadata_hints(&raw).expect("metadata carries SHOULD-layout hints")
}

fn u64_word(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

// ── Tests ────────────────────────────────────────────────────────

/// (a-produce) A gas-requesting ERC-20 deposit's announcement metadata follows
/// the EIP-5564 token SHOULD layout; the native layout totals payment + gas.
#[tokio::test]
async fn create_with_request_gas_produces_eip5564_metadata_layouts() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();

    // ERC-20 with explicit gas amount + expected token amount.
    let create = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/create-erc20",
        json!({
            "wallet_profile": "payments-mainnet",
            "token_address": TOKEN,
            "expected_amount_hex": "0x1000",
            "request_gas": true,
            "gas_amount_wei_hex": "0x5208",
        }),
        Some(&rig.token),
    )
    .await;
    let deposit = &create["deposit"];
    assert_eq!(deposit["requested_gas_wei_hex"], "0x5208");
    let metadata_hex = deposit["announcement"]["metadata_hex"].as_str().unwrap();
    let hint = metadata_hints(metadata_hex);
    let sigillum_core::Erc5564MetadataHints::Token {
        function_selector,
        token_address,
        amount,
    } = hint
    else {
        panic!("expected token hints, got {hint:?}");
    };
    assert_eq!(
        function_selector,
        sigillum_core::ERC5564_METADATA_ERC20_TRANSFER_SELECTOR
    );
    assert_eq!(token_address, TOKEN);
    assert_eq!(amount, u64_word(0x1000));
    // The hinted metadata is embedded verbatim in the announce calldata.
    let calldata_hex = deposit["announcement"]["calldata_hex"].as_str().unwrap();
    assert!(
        calldata_hex.contains(metadata_hex.trim_start_matches("0x")),
        "calldata must embed the hinted metadata"
    );

    // Without request_gas the metadata stays minimal (view tag only).
    let create_default = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/create-erc20",
        json!({
            "wallet_profile": "payments-mainnet",
            "token_address": TOKEN,
        }),
        Some(&rig.token),
    )
    .await;
    let default_metadata = create_default["deposit"]["announcement"]["metadata_hex"]
        .as_str()
        .unwrap();
    assert_eq!(
        default_metadata.len(),
        2,
        "default metadata is the 1-byte view tag: {default_metadata}"
    );
    assert!(create_default["deposit"]["requested_gas_wei_hex"].is_null());

    // Native with explicit expected value + gas: the native layout totals both.
    let create_native = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/create-native",
        json!({
            "wallet_profile": "payments-mainnet",
            "expected_value_wei_hex": "0x100",
            "request_gas": true,
            "gas_amount_wei_hex": "0x5208",
        }),
        Some(&rig.token),
    )
    .await;
    let native_hint = metadata_hints(
        create_native["deposit"]["announcement"]["metadata_hex"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        native_hint,
        sigillum_core::Erc5564MetadataHints::Native {
            amount_wei: u64_word(0x100 + 0x5208),
        }
    );
    assert_eq!(create_native["deposit"]["requested_gas_wei_hex"], "0x5208");
    // The record's expected amount stays the payment only; the gas rides on
    // top inside the payer-facing metadata total.
    assert_eq!(create_native["deposit"]["expected_amount_hex"], "0x100");

    // Native with request_gas and NO explicit gas amount: the provider's
    // static sweep gas estimate (2 gwei x 21_000 = 42e12 wei) is requested.
    let create_estimated = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/create-native",
        json!({
            "wallet_profile": "payments-mainnet",
            "request_gas": true,
        }),
        Some(&rig.token),
    )
    .await;
    let estimated_hint = metadata_hints(
        create_estimated["deposit"]["announcement"]["metadata_hex"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        estimated_hint,
        sigillum_core::Erc5564MetadataHints::Native {
            amount_wei: u64_word(42_000_000_000_000),
        }
    );
    assert_eq!(
        create_estimated["deposit"]["requested_gas_wei_hex"],
        "0x2632e314a000"
    );

    rig.abort();
}

/// (b-consume) A scan over announcements carrying SHOULD-layout hints creates
/// deposits whose asset kind, token contract, and expected amount come from
/// the metadata — no operator-supplied token address.
#[tokio::test]
async fn scan_autopopulates_deposit_from_metadata_hints() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    let meta_address = rig.export_meta_address().await;

    // Token-layout announcement for this wallet.
    let token_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x61u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    let view_tag = hex::decode(&token_payment.view_tag_hex).unwrap()[0];
    let token_metadata = sigillum_core::encode_erc5564_metadata_erc20_transfer(
        view_tag,
        TOKEN,
        &u64_word(0x2a),
    )
    .unwrap();
    // Native-layout announcement for this wallet.
    let native_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x62u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();
    let native_view_tag = hex::decode(&native_payment.view_tag_hex).unwrap()[0];
    let native_metadata =
        sigillum_core::encode_erc5564_metadata_native(native_view_tag, &u64_word(0x100));
    // View-tag-only noise announcement (no hints): still detected, no asset info.
    let bare_payment = sigillum_core::generate_ethereum_stealth_address(
        &meta_address,
        Some([0x63u8; 32]),
        StealthHashConvention::Compressed33,
    )
    .unwrap();

    rig.rpc_state.announcement_logs.write().unwrap().extend([
        announcement_log(
            &token_payment.stealth_address,
            &token_payment.ephemeral_public_key_hex,
            &hex::decode(&token_metadata).unwrap(),
        ),
        announcement_log(
            &native_payment.stealth_address,
            &native_payment.ephemeral_public_key_hex,
            &hex::decode(&native_metadata).unwrap(),
        ),
        announcement_log(
            &bare_payment.stealth_address,
            &bare_payment.ephemeral_public_key_hex,
            &[hex::decode(&bare_payment.view_tag_hex).unwrap()[0]],
        ),
    ]);

    // Scan WITHOUT a token_address: hints drive the created records.
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
    assert_eq!(scan["matched"], 3, "scan: {scan}");
    assert_eq!(scan["created"], 3, "scan: {scan}");

    let deposits = scan["deposits"].as_array().unwrap();
    let token_deposit = deposits
        .iter()
        .find(|deposit| deposit["stealth_address"] == token_payment.stealth_address)
        .expect("token deposit in scan response");
    assert_eq!(token_deposit["asset_kind"], "erc20");
    assert_eq!(token_deposit["token_address"], TOKEN);
    assert_eq!(token_deposit["expected_amount_hex"], "0x2a");

    let native_deposit = deposits
        .iter()
        .find(|deposit| deposit["stealth_address"] == native_payment.stealth_address)
        .expect("native deposit in scan response");
    assert_eq!(native_deposit["asset_kind"], "native");
    assert_eq!(native_deposit["expected_amount_hex"], "0x100");

    let bare_deposit = deposits
        .iter()
        .find(|deposit| deposit["stealth_address"] == bare_payment.stealth_address)
        .expect("bare deposit in scan response");
    assert_eq!(bare_deposit["asset_kind"], "native");
    assert!(bare_deposit["expected_amount_hex"].is_null());

    // A rescan finds no duplicates (hint-resolved identities match).
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
    assert_eq!(rescan["matched"], 3, "rescan: {rescan}");
    assert_eq!(rescan["created"], 0, "rescan: {rescan}");
    assert_eq!(rescan["existing"], 3, "rescan: {rescan}");

    rig.abort();
}

/// (c-refresh) A `funded_needs_gas` ERC-20 deposit moves to `funded` once
/// native gas shows up on the stealth address.
#[tokio::test]
async fn funded_needs_gas_transitions_to_funded_when_gas_arrives() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();

    let deposit = rig.create_erc20_deposit(false, false, None).await;
    let deposit_id = deposit["id"].as_str().unwrap().to_string();
    let stealth_address = deposit["stealth_address"].as_str().unwrap().to_string();
    // Tokens arrive (the mock reports a token balance) but no native gas.
    rig.set_balance(&stealth_address, "0x0").await;

    let refresh = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "id": deposit_id, "auto_enqueue": true }),
        Some(&rig.token),
    )
    .await;
    let record = &refresh["deposits"][0];
    assert_eq!(record["status"], "funded_needs_gas", "refresh: {refresh}");
    assert_eq!(refresh["queued"], 0, "no auto-sweep without request flag: {refresh}");

    // Gas arrives (payer-attached or manual): the next refresh flips it.
    rig.set_balance(&stealth_address, EXPECTED_TOPUP_WEI_HEX).await;
    let refresh = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "id": deposit_id, "auto_enqueue": true }),
        Some(&rig.token),
    )
    .await;
    let record = &refresh["deposits"][0];
    // auto_queue_sweep is OFF for this deposit: the record now reads plain
    // `funded` (sweep-ready) instead of `funded_needs_gas`.
    assert_eq!(record["status"], "funded", "refresh: {refresh}");
    assert_eq!(refresh["queued"], 0, "refresh: {refresh}");
    assert!(
        record["gas_topup_job_id"].is_null(),
        "no policy top-ups configured, no sponsor job: {record}"
    );

    rig.abort();
}

/// (d-e2e) Sponsor fund_gas through the queue: gas-starved ERC-20 deposit →
/// sponsor top-up job → sweep dependent on it → top-up broadcasts → sweep
/// stays blocked until the gas is on-chain → sweep executes.
#[tokio::test]
async fn erc20_deposit_gas_topup_then_sweep_end_to_end() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    rig.set_policy(true, false).await;

    let deposit = rig.create_erc20_deposit(true, true, None).await;
    let deposit_id = deposit["id"].as_str().unwrap().to_string();
    let stealth_address = deposit["stealth_address"].as_str().unwrap().to_string();
    rig.set_balance(&stealth_address, "0x0").await;

    // Refresh: tokens observed, gas short → sponsor top-up + dependent sweep.
    let refresh = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "id": deposit_id, "auto_enqueue": true }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(refresh["queued"], 1, "refresh: {refresh}");
    let record = &refresh["deposits"][0];
    assert_eq!(record["status"], "sweep_queued", "record: {record}");
    let topup_job_id = record["gas_topup_job_id"]
        .as_str()
        .expect("sponsor top-up tracked on the deposit")
        .to_string();
    assert_eq!(record["gas_topup_job_state"], "queued");
    let sweep_job_id = record["queue_job_id"].as_str().unwrap().to_string();

    // The sweep depends on the top-up; the top-up pays 1.5x the sweep's
    // estimated gas from the derived sponsor to the stealth address.
    let jobs = rig.queue_jobs().await;
    let topup_job = jobs
        .iter()
        .find(|job| job["id"] == topup_job_id)
        .expect("top-up job in queue");
    assert_eq!(topup_job["kind"], "eth_stealth_gas_topup");
    assert_eq!(topup_job["destination_address"], stealth_address);
    assert_eq!(topup_job["value_wei_hex"], EXPECTED_TOPUP_WEI_HEX);
    let sponsor_address = topup_job["sponsor_address"].as_str().unwrap().to_string();
    assert!(sponsor_address.starts_with("0x"));
    assert!(!sponsor_address.eq_ignore_ascii_case(&stealth_address));
    let sweep_job = jobs
        .iter()
        .find(|job| job["id"] == sweep_job_id)
        .expect("sweep job in queue");
    assert_eq!(sweep_job["kind"], "eth_stealth_erc20_sweep");
    assert_eq!(
        sweep_job["prerequisite_job_ids"],
        json!([topup_job_id.clone()]),
        "sweep must depend on the top-up: {sweep_job}"
    );

    // One drain: the top-up broadcasts; the sweep's dependency clears but its
    // own on-chain gas check still blocks (the top-up has not confirmed).
    let drain = post_ok(
        &client,
        rig.addr,
        "/api/queue/process",
        json!({}),
        Some(&rig.token),
    )
    .await;
    let topup_after = drain["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == topup_job_id)
        .expect("top-up in drain response");
    assert_eq!(topup_after["state"], "sent", "drain: {drain}");
    let sweep_after = drain["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == sweep_job_id)
        .expect("sweep in drain response");
    assert_eq!(sweep_after["state"], "blocked", "drain: {drain}");
    assert!(
        sweep_after["last_error"]
            .as_str()
            .unwrap_or_default()
            .contains("lacks native gas"),
        "sweep must remain blocked until gas confirms: {sweep_after}"
    );

    // The top-up confirms on-chain (gas visible at the stealth address): the
    // sweep now executes.
    rig.set_balance(&stealth_address, EXPECTED_TOPUP_WEI_HEX).await;
    let drained = post_ok(
        &client,
        rig.addr,
        "/api/queue/process",
        json!({ "id": sweep_job_id }),
        Some(&rig.token),
    )
    .await;
    assert_eq!(drained["succeeded"], 1, "sweep failed: {drained}");
    assert_eq!(drained["jobs"][0]["state"], "sent");

    // The deposit mirrors both job states after the next refresh (no
    // auto-enqueue: the mock reports the token balance forever, so an
    // auto-enqueueing refresh would start a fresh sweep).
    let refresh = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/refresh",
        json!({ "id": deposit_id, "auto_enqueue": false }),
        Some(&rig.token),
    )
    .await;
    let record = &refresh["deposits"][0];
    assert_eq!(record["status"], "sweep_sent", "record: {record}");
    assert_eq!(record["gas_topup_job_state"], "sent", "record: {record}");

    rig.abort();
}

/// (d-linkage) One derived sponsor funding deposits of DIFFERENT parties:
/// warns when `block_cross_party_linkage` is off, hard-blocks when on.
#[tokio::test]
async fn cross_party_sponsor_funding_warns_or_hard_blocks_per_policy() {
    let dir = TempDir::new().unwrap();
    let rig = spawn_rig(&dir).await;
    let client = reqwest::Client::new();
    rig.set_policy(true, false).await;

    // Two ERC-20 deposits on the same wallet (hence the same derived
    // sponsor), each with its OWN sweep destination so only the sponsor axis
    // can link them.
    let deposit_a = rig.create_erc20_deposit(true, true, Some(DESTINATION)).await;
    let deposit_b = rig.create_erc20_deposit(true, true, Some(DESTINATION_B)).await;
    let id_a = deposit_a["id"].as_str().unwrap().to_string();
    let id_b = deposit_b["id"].as_str().unwrap().to_string();
    for deposit in [&deposit_a, &deposit_b] {
        rig.set_balance(deposit["stealth_address"].as_str().unwrap(), "0x0")
            .await;
    }

    // Tag the deposits to two different parties.
    let mut party_ids = Vec::new();
    for name in ["Alpha", "Beta"] {
        let party = post_ok(
            &client,
            rig.addr,
            "/api/treasury/parties",
            json!({ "name": name }),
            Some(&rig.token),
        )
        .await;
        party_ids.push(party["party"]["id"].as_str().unwrap().to_string());
    }
    for (deposit_id, party_id) in [(&id_a, &party_ids[0]), (&id_b, &party_ids[1])] {
        post_ok(
            &client,
            rig.addr,
            "/api/receiving/deposits/tag",
            json!({ "deposit_id": deposit_id, "counterparty_id": party_id }),
            Some(&rig.token),
        )
        .await;
    }

    // Party Alpha's deposit sweeps first: its sponsor top-up is enqueued
    // without any linkage (no other sponsored party yet).
    let enqueue_a = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": id_a }),
        Some(&rig.token),
    )
    .await;
    assert!(
        enqueue_a["linkage_warning"].is_null(),
        "first sponsored deposit links nothing: {enqueue_a}"
    );
    assert!(
        enqueue_a["risk_findings"]
            .as_array()
            .map(|findings| findings.is_empty())
            .unwrap_or(true),
        "first sponsored deposit yields no risk findings: {enqueue_a}"
    );
    let record_a = rig.deposit_record(&id_a).await;
    assert!(
        record_a["gas_topup_job_id"].is_string(),
        "top-up tracked for party Alpha's deposit: {record_a}"
    );

    // Party Beta's deposit now shares the same derived sponsor: the enqueue
    // succeeds (policy off) but surfaces the shared-sponsor warning.
    let enqueue_b = post_ok(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": id_b }),
        Some(&rig.token),
    )
    .await;
    let warning = enqueue_b["linkage_warning"].as_str().unwrap_or_default();
    assert!(
        warning.contains("shared gas sponsor links this party"),
        "shared sponsor across parties must warn: {enqueue_b}"
    );
    let jobs = rig.queue_jobs().await;
    let topups: Vec<&Value> = jobs
        .iter()
        .filter(|job| job["kind"] == "eth_stealth_gas_topup")
        .collect();
    assert_eq!(topups.len(), 2, "both top-ups enqueued: {jobs:?}");
    assert_eq!(
        topups[0]["sponsor_address"], topups[1]["sponsor_address"],
        "same wallet derives the same sponsor"
    );

    // Plan task 3.5: the shared-sponsor detection also surfaces a structured
    // `common_gas_funder` risk finding (advisory — the enqueue succeeded).
    let findings = enqueue_b["risk_findings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(findings.len(), 1, "risk findings: {enqueue_b}");
    assert_eq!(findings[0]["category"], "common_gas_funder");
    assert_eq!(findings[0]["subject_type"], "gas_funder");
    assert_eq!(
        findings[0]["subject"], topups[0]["sponsor_address"],
        "finding subject is the shared sponsor: {enqueue_b}"
    );

    // Hard-block: with `block_cross_party_linkage` on, a THIRD party's deposit
    // cannot get sponsor funding at all (policy_violation).
    let deposit_c = rig.create_erc20_deposit(true, true, Some(DESTINATION)).await;
    let id_c = deposit_c["id"].as_str().unwrap().to_string();
    rig.set_balance(
        deposit_c["stealth_address"].as_str().unwrap(),
        "0x0",
    )
    .await;
    let party_c = post_ok(
        &client,
        rig.addr,
        "/api/treasury/parties",
        json!({ "name": "Gamma" }),
        Some(&rig.token),
    )
    .await;
    post_ok(
        &client,
        rig.addr,
        "/api/receiving/deposits/tag",
        json!({
            "deposit_id": id_c,
            "counterparty_id": party_c["party"]["id"].as_str().unwrap(),
        }),
        Some(&rig.token),
    )
    .await;
    rig.set_policy(true, true).await;

    let blocked = post_json(
        &client,
        rig.addr,
        "/api/deposits/eth-stealth/enqueue-sweep",
        json!({ "id": id_c }),
        Some(&rig.token),
    )
    .await;
    let blocked_status = blocked.status();
    let blocked_body: Value = blocked.json().await.unwrap();
    assert_eq!(
        blocked_status,
        StatusCode::FORBIDDEN,
        "cross-party sponsor funding must hard-block: {blocked_body}"
    );
    assert_eq!(blocked_body["action"], "cross_party_linkage");
    // Nothing was enqueued for the blocked deposit.
    let record_c = rig.deposit_record(&id_c).await;
    assert!(
        record_c["queue_job_id"].is_null() && record_c["gas_topup_job_id"].is_null(),
        "hard-blocked enqueue leaves no jobs behind: {record_c}"
    );

    rig.abort();
}
