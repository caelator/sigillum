mod common;

use common::{get_json, post_json, spawn_daemon};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get as axum_get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::task::JoinHandle;

const OWNER: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const CONTRACT_OPTED: &str = "0xaaaa000000000000000000000000000000000aaa";
const CONTRACT_OTHER: &str = "0xbbbb000000000000000000000000000000000bbb";
const CONTRACT_IPFS: &str = "0xcccc000000000000000000000000000000000ccc";
const TOKEN_ID_HEX: &str = "0x000000000000000000000000000000000000000000000000000000000000007b";

#[derive(Clone)]
struct RpcState {
    metadata_base_url: String,
    token_uri_calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct MetadataState {
    hits: Arc<Mutex<HashMap<String, usize>>>,
}

struct MockServers {
    rpc_addr: SocketAddr,
    metadata_addr: SocketAddr,
    rpc_handle: JoinHandle<()>,
    metadata_handle: JoinHandle<()>,
    token_uri_calls: Arc<Mutex<Vec<String>>>,
    metadata_hits: Arc<Mutex<HashMap<String, usize>>>,
}

impl MockServers {
    async fn spawn() -> Self {
        let (metadata_addr, metadata_handle, metadata_hits) = spawn_metadata_server().await;
        let (rpc_addr, rpc_handle, token_uri_calls) = spawn_mock_evm_provider(metadata_addr).await;
        Self {
            rpc_addr,
            metadata_addr,
            rpc_handle,
            metadata_handle,
            token_uri_calls,
            metadata_hits,
        }
    }

    fn token_uri_calls(&self) -> Vec<String> {
        self.token_uri_calls.lock().unwrap().clone()
    }

    fn metadata_hits(&self) -> HashMap<String, usize> {
        self.metadata_hits.lock().unwrap().clone()
    }

    fn abort(self) {
        self.rpc_handle.abort();
        self.metadata_handle.abort();
    }
}

async fn spawn_mock_evm_provider(
    metadata_addr: SocketAddr,
) -> (SocketAddr, JoinHandle<()>, Arc<Mutex<Vec<String>>>) {
    let token_uri_calls = Arc::new(Mutex::new(Vec::new()));
    let state = RpcState {
        metadata_base_url: format!("http://{metadata_addr}"),
        token_uri_calls: Arc::clone(&token_uri_calls),
    };
    let app = Router::new()
        .route("/", post(rpc_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, token_uri_calls)
}

async fn rpc_handler(
    State(state): State<RpcState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let authorized = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some("Bearer rpc-test-token");
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        );
    }

    let payload = if let Some(requests) = body.as_array() {
        Value::Array(
            requests
                .iter()
                .map(|request| rpc_response(request, &state))
                .collect(),
        )
    } else {
        rpc_response(&body, &state)
    };
    (StatusCode::OK, Json(payload))
}

fn rpc_response(request: &Value, state: &RpcState) -> Value {
    let method = request["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_blockNumber" => json!("0x20"),
        "eth_call" => {
            let to = request["params"][0]["to"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let data = request["params"][0]["data"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if data.starts_with("0xc87b56dd") || data.starts_with("0x0e89341c") {
                state.token_uri_calls.lock().unwrap().push(to.clone());
                if to == CONTRACT_OPTED {
                    json!(abi_string(&format!(
                        "{}/meta/opted.json",
                        state.metadata_base_url
                    )))
                } else if to == CONTRACT_IPFS {
                    json!(abi_string("ipfs://bafyfakecid/1.json"))
                } else {
                    json!(abi_string(&format!(
                        "{}/meta/other.json",
                        state.metadata_base_url
                    )))
                }
            } else {
                json!("0x0")
            }
        }
        _ => json!({ "unsupported": method }),
    };
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn abi_string(value: &str) -> String {
    let mut data = hex::encode(value.as_bytes());
    while data.len() % 64 != 0 {
        data.push('0');
    }
    format!("0x{}{}{}", abi_word(32), abi_word(value.len()), data)
}

fn abi_word(value: usize) -> String {
    format!("{value:064x}")
}

async fn spawn_metadata_server() -> (
    SocketAddr,
    JoinHandle<()>,
    Arc<Mutex<HashMap<String, usize>>>,
) {
    let hits = Arc::new(Mutex::new(HashMap::new()));
    let app = Router::new()
        .route("/meta/opted.json", axum_get(metadata_handler))
        .route("/ipfs/bafyfakecid/1.json", axum_get(metadata_handler))
        .fallback(metadata_handler)
        .with_state(MetadataState {
            hits: Arc::clone(&hits),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle, hits)
}

async fn metadata_handler(
    State(state): State<MetadataState>,
    request: Request,
) -> (StatusCode, Json<Value>) {
    let path = request.uri().path().to_string();
    *state.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let name = match path.as_str() {
        "/meta/opted.json" => "Mock Opted Collection",
        "/ipfs/bafyfakecid/1.json" => "Mock IPFS Collection",
        _ => "SHOULD NOT BE FETCHED",
    };
    (StatusCode::OK, Json(json!({ "name": name })))
}

async fn response_json(response: reqwest::Response, expected: StatusCode) -> Value {
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, expected, "response body: {body}");
    body
}

async fn expect_status(response: reqwest::Response, expected: StatusCode) {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(status, expected, "response body: {body}");
}

async fn init_session(client: &reqwest::Client, addr: SocketAddr) -> String {
    let init_json = response_json(
        post_json(
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
        .await,
        StatusCode::OK,
    )
    .await;
    init_json["session_token"].as_str().unwrap().to_string()
}

async fn configure_provider(
    client: &reqwest::Client,
    daemon_addr: SocketAddr,
    token: &str,
    rpc_addr: SocketAddr,
) {
    expect_status(
        post_json(
            client,
            daemon_addr,
            "/api/api-keys/set",
            json!({ "key": "alchemy", "value": "rpc-test-token" }),
            Some(token),
        )
        .await,
        StatusCode::OK,
    )
    .await;

    expect_status(
        post_json(
            client,
            daemon_addr,
            "/api/profiles/evm/upsert",
            json!({
                "name": "mainnet",
                "rpc_url": format!("http://{rpc_addr}/"),
                "auth_token_key": "alchemy",
                "chain_id": 1,
            }),
            Some(token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
}

fn seed_inventory(base_dir: &Path, transaction_count: u64, contracts: &[&str]) {
    let holdings = contracts
        .iter()
        .enumerate()
        .map(|(index, contract)| nft_holding(index, contract))
        .collect::<Vec<_>>();
    let envelope = json!({
        "schema": "sigillum.wallet-inventory",
        "schema_version": 14,
        "data": {
            "addresses": [{
                "id": "addr_owner",
                "wallet_family": "eth-watch",
                "wallet_profile": "watch:test",
                "provider_profile": "mainnet",
                "chain_id": 1,
                "address": OWNER,
                "derivation_path": "m/44'/60'/0'/0/0",
                "address_index": 0,
                "activity_state": "funded",
                "native_balance_wei_hex": "0x1",
                "transaction_count": transaction_count,
                "source": "local-rpc",
                "first_seen_at_unix": 1,
                "last_checked_at_unix": 2,
            }],
            "holdings": holdings,
        },
    });
    std::fs::write(
        base_dir.join("wallet_inventory.json"),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
}

fn nft_holding(index: usize, contract: &str) -> Value {
    json!({
        "id": format!("holding_{index}"),
        "wallet_family": "eth-watch",
        "wallet_profile": "watch:test",
        "provider_profile": "mainnet",
        "chain_id": 1,
        "address": OWNER,
        "derivation_path": "m/44'/60'/0'/0/0",
        "asset_kind": "erc721",
        "asset_address": contract,
        "token_id_hex": TOKEN_ID_HEX,
        "amount_hex": "0x1",
        "status": "detected",
        "source": "erc721-transfer-log",
        "first_seen_at_unix": 1,
        "last_checked_at_unix": 2,
    })
}

async fn opt_in_collection(
    client: &reqwest::Client,
    daemon_addr: SocketAddr,
    token: &str,
    contract: &str,
) -> Value {
    response_json(
        post_json(
            client,
            daemon_addr,
            "/api/inventory/nft-metadata/opt-ins/upsert",
            json!({
                "chain_id": 1,
                "contract_address": contract,
                "enabled": true,
            }),
            Some(token),
        )
        .await,
        StatusCode::OK,
    )
    .await
}

fn find_entry<'a>(entries: &'a [Value], contract: &str) -> &'a Value {
    entries
        .iter()
        .find(|entry| {
            entry["chain_id"] == 1
                && entry["contract_address"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(contract))
                && entry["token_id_hex"] == TOKEN_ID_HEX
        })
        .unwrap_or_else(|| panic!("missing cache entry for {contract}"))
}

fn find_holding<'a>(holdings: &'a [Value], contract: &str) -> &'a Value {
    holdings
        .iter()
        .find(|holding| {
            holding["asset_address"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(contract))
        })
        .unwrap_or_else(|| panic!("missing holding for {contract}"))
}

fn is_64_hex(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn array_field_len(value: &Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

#[tokio::test]
async fn opted_in_collection_fetch_caches_metadata_with_provenance() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_session(&client, daemon_addr).await;
    configure_provider(&client, daemon_addr, &token, servers.rpc_addr).await;
    seed_inventory(dir.path(), 7, &[CONTRACT_OPTED, CONTRACT_OTHER]);

    let upsert_json = opt_in_collection(&client, daemon_addr, &token, CONTRACT_OPTED).await;
    assert_eq!(upsert_json["opt_in"]["enabled"], true);

    let opt_ins_json = response_json(
        get_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/opt-ins",
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert!(
        opt_ins_json["opt_ins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|opt_in| {
                opt_in["chain_id"] == 1
                    && opt_in["contract_address"] == CONTRACT_OPTED
                    && opt_in["enabled"] == true
            })
    );

    let fetch_json = response_json(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({}),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fetch_json["fetched"], 1);
    let fetch_entries = fetch_json["entries"].as_array().unwrap();
    let entry = find_entry(fetch_entries, CONTRACT_OPTED);
    let expected_uri = format!("http://{}/meta/opted.json", servers.metadata_addr);
    assert_eq!(entry["name"], "Mock Opted Collection");
    assert_eq!(entry["metadata_uri"], expected_uri);
    assert_eq!(entry["fetched_uri"], expected_uri);
    assert!(entry["fetched_at_unix"].as_u64().unwrap() > 0);
    assert!(is_64_hex(&entry["content_sha256"]));

    let inventory_json = response_json(
        get_json(&client, daemon_addr, "/api/inventory/wallets", Some(&token)).await,
        StatusCode::OK,
    )
    .await;
    let cache = inventory_json["nft_metadata_cache"].as_array().unwrap();
    let cached_entry = find_entry(cache, CONTRACT_OPTED);
    assert_eq!(cached_entry["name"], "Mock Opted Collection");
    assert_eq!(cached_entry["metadata_uri"], expected_uri);
    let holdings = inventory_json["holdings"].as_array().unwrap();
    let opted_holding = find_holding(holdings, CONTRACT_OPTED);
    assert_eq!(opted_holding["metadata_name"], "Mock Opted Collection");

    assert_eq!(servers.token_uri_calls(), vec![CONTRACT_OPTED.to_string()]);
    let hits = servers.metadata_hits();
    assert_eq!(hits.get("/meta/opted.json").copied(), Some(1));
    assert_eq!(hits.len(), 1, "unexpected metadata hits: {hits:?}");

    daemon_handle.abort();
    servers.abort();
}

#[tokio::test]
async fn no_opt_in_fetches_nothing() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_session(&client, daemon_addr).await;
    configure_provider(&client, daemon_addr, &token, servers.rpc_addr).await;
    seed_inventory(dir.path(), 7, &[CONTRACT_OPTED, CONTRACT_OTHER]);

    let fetch_all = response_json(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({}),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fetch_all["fetched"], 0);
    assert_eq!(array_field_len(&fetch_all, "entries"), 0);
    assert_eq!(array_field_len(&fetch_all, "skipped"), 0);

    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({
                "chain_id": 1,
                "contract_address": CONTRACT_OTHER,
            }),
            Some(&token),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(servers.token_uri_calls().is_empty());
    assert!(servers.metadata_hits().is_empty());

    daemon_handle.abort();
    servers.abort();
}

#[tokio::test]
async fn nft_metadata_routes_require_session() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    for token in [None, Some("bogus-token")] {
        expect_status(
            get_json(
                &client,
                daemon_addr,
                "/api/inventory/nft-metadata/opt-ins",
                token,
            )
            .await,
            StatusCode::UNAUTHORIZED,
        )
        .await;
        expect_status(
            post_json(
                &client,
                daemon_addr,
                "/api/inventory/nft-metadata/opt-ins/upsert",
                json!({
                    "chain_id": 1,
                    "contract_address": CONTRACT_OPTED,
                }),
                token,
            )
            .await,
            StatusCode::UNAUTHORIZED,
        )
        .await;
        expect_status(
            post_json(
                &client,
                daemon_addr,
                "/api/inventory/nft-metadata/fetch",
                json!({}),
                token,
            )
            .await,
            StatusCode::UNAUTHORIZED,
        )
        .await;
    }
    assert!(servers.token_uri_calls().is_empty());
    assert!(servers.metadata_hits().is_empty());

    daemon_handle.abort();
    servers.abort();
}

#[tokio::test]
async fn nft_metadata_validation_failures() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_session(&client, daemon_addr).await;

    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/opt-ins/upsert",
            json!({
                "chain_id": 1,
                "contract_address": "not-an-address",
            }),
            Some(&token),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/opt-ins/upsert",
            json!({
                "chain_id": 0,
                "contract_address": CONTRACT_OPTED,
            }),
            Some(&token),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/settings",
            json!({
                "ipfs_gateway_url": "ftp://bad",
            }),
            Some(&token),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({
                "limit": 0,
            }),
            Some(&token),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(servers.token_uri_calls().is_empty());
    assert!(servers.metadata_hits().is_empty());

    daemon_handle.abort();
    servers.abort();
}

#[tokio::test]
async fn ipfs_uri_skipped_without_gateway_then_fetched_with_gateway() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_session(&client, daemon_addr).await;
    configure_provider(&client, daemon_addr, &token, servers.rpc_addr).await;
    seed_inventory(dir.path(), 7, &[CONTRACT_IPFS]);
    opt_in_collection(&client, daemon_addr, &token, CONTRACT_IPFS).await;

    let skipped_json = response_json(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({}),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(skipped_json["fetched"], 0);
    let skipped = skipped_json["skipped"].as_array().unwrap();
    assert!(skipped.iter().any(|skip| {
        skip["contract_address"] == CONTRACT_IPFS
            && skip["token_id_hex"] == TOKEN_ID_HEX
            && skip["reason"] == "ipfs_gateway_not_configured"
    }));
    assert!(servers.metadata_hits().is_empty());

    let gateway_url = format!("http://{}/ipfs/", servers.metadata_addr);
    response_json(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/settings",
            json!({
                "ipfs_gateway_url": gateway_url,
            }),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;

    let fetched_json = response_json(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({}),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fetched_json["fetched"], 1);
    let entries = fetched_json["entries"].as_array().unwrap();
    let entry = find_entry(entries, CONTRACT_IPFS);
    assert_eq!(entry["name"], "Mock IPFS Collection");
    assert!(
        entry["fetched_uri"]
            .as_str()
            .unwrap()
            .starts_with(&gateway_url)
    );
    let hits = servers.metadata_hits();
    assert_eq!(hits.get("/ipfs/bafyfakecid/1.json").copied(), Some(1));

    daemon_handle.abort();
    servers.abort();
}

#[tokio::test]
async fn airdropped_collection_flagged_with_reasons() {
    let dir = TempDir::new().unwrap();
    let servers = MockServers::spawn().await;
    let (daemon_addr, daemon_handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();
    let token = init_session(&client, daemon_addr).await;
    configure_provider(&client, daemon_addr, &token, servers.rpc_addr).await;
    seed_inventory(dir.path(), 0, &[CONTRACT_OPTED]);
    opt_in_collection(&client, daemon_addr, &token, CONTRACT_OPTED).await;

    expect_status(
        post_json(
            &client,
            daemon_addr,
            "/api/inventory/nft-metadata/fetch",
            json!({}),
            Some(&token),
        )
        .await,
        StatusCode::OK,
    )
    .await;

    let inventory_json = response_json(
        get_json(&client, daemon_addr, "/api/inventory/wallets", Some(&token)).await,
        StatusCode::OK,
    )
    .await;
    let cache = inventory_json["nft_metadata_cache"].as_array().unwrap();
    let entry = find_entry(cache, CONTRACT_OPTED);
    assert_eq!(entry["spam_label"], "suspected_airdrop");
    let reasons = entry["spam_reasons"].as_array().unwrap();
    assert!(reasons.contains(&json!("received_without_outbound_activity")));
    assert!(reasons.contains(&json!("no_matching_operator_approval")));

    daemon_handle.abort();
    servers.abort();
}
