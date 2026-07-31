mod common;

use common::{get, post_json, spawn_daemon};
use std::net::SocketAddr;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

#[derive(Clone)]
struct RpcState;

async fn spawn_mock_evm_provider() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    fn rpc_response(request: &serde_json::Value) -> serde_json::Value {
        let method = request["method"].as_str().unwrap_or_default();
        let result = match method {
            "eth_chainId" => json!("0x1"),
            "eth_blockNumber" => json!("0x20"),
            "eth_getBalance" => json!("0xde0b6b3a7640000"),
            "eth_getTransactionCount" => json!("0x7"),
            "eth_call" => {
                let to = request["params"][0]["to"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let data = request["params"][0]["data"].as_str().unwrap_or_default();
                if data.starts_with("0x70a08231")
                    && to == "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                {
                    json!("0x0f4240")
                } else {
                    json!("0x0")
                }
            }
            _ => json!("0x0"),
        };

        json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or_else(|| json!(1)),
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

struct TestDaemon {
    addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    token: String,
}

async fn setup_daemon_with_provider(dir: &TempDir) -> TestDaemon {
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
    let init_status = init.status();
    let init_json: serde_json::Value = init.json().await.unwrap();
    assert_eq!(init_status, StatusCode::OK, "init response: {init_json}");
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let key = post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;
    let key_status = key.status();
    let key_json: serde_json::Value = key.json().await.unwrap();
    assert_eq!(key_status, StatusCode::OK, "api key response: {key_json}");

    let provider = post_json(
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
    let provider_status = provider.status();
    let provider_json: serde_json::Value = provider.json().await.unwrap();
    assert_eq!(
        provider_status,
        StatusCode::OK,
        "provider response: {provider_json}"
    );

    TestDaemon {
        addr,
        handle,
        rpc_handle,
        client,
        token,
    }
}

fn registry_entries_json() -> String {
    serde_json::to_string(&json!([
        {
            "chain_id": 1,
            "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "symbol": "AAA",
            "decimals": 18
        },
        {
            "chain_id": 1,
            "address": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "symbol": "BBB",
            "decimals": 6
        },
        {
            "chain_id": 137,
            "address": "0xcccccccccccccccccccccccccccccccccccccccc",
            "symbol": "CCC",
            "decimals": 18
        }
    ]))
    .unwrap()
}

async fn import_core_list(setup: &TestDaemon) -> serde_json::Value {
    let import = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry/import",
        json!({
            "name": "core-list",
            "entries_json": registry_entries_json(),
        }),
        Some(&setup.token),
    )
    .await;
    let status = import.status();
    let json: serde_json::Value = import.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "import response: {json}");
    json
}

async fn assert_bad_request(response: reqwest::Response, label: &str) -> String {
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
    body
}

#[tokio::test]
async fn inventory_token_registry_import_list_delete_happy_path() {
    let dir = TempDir::new().unwrap();
    let setup = setup_daemon_with_provider(&dir).await;

    let import_json = import_core_list(&setup).await;
    assert_eq!(import_json["status"], "imported");
    assert_eq!(import_json["list"]["source"], "pasted-json");
    assert_eq!(import_json["list"]["entries"].as_array().unwrap().len(), 3);

    let list = get(
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry",
        Some(&setup.token),
    )
    .await;
    let list_status = list.status();
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_status, StatusCode::OK, "list response: {list_json}");
    assert_eq!(list_json["lists"].as_array().unwrap().len(), 1);

    let delete = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry/delete",
        json!({ "name": "core-list" }),
        Some(&setup.token),
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
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry",
        Some(&setup.token),
    )
    .await;
    let list_status = list.status();
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_status, StatusCode::OK, "list response: {list_json}");
    assert_eq!(list_json["lists"].as_array().unwrap().len(), 0);

    setup.handle.abort();
    setup.rpc_handle.abort();
}

#[tokio::test]
async fn inventory_token_registry_scan_probes_only_matching_chain_positive_balances() {
    let dir = TempDir::new().unwrap();
    let setup = setup_daemon_with_provider(&dir).await;
    import_core_list(&setup).await;

    let scan = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/scan/evm",
        json!({
            "wallet_family": "eth-watch",
            "watch_addresses": [{
                "address": "0x9858effd232b4033e47d90003d41ec34ecaeda94"
            }],
            "provider_profile": "mainnet",
            "gap_limit": 1,
            "max_index": 0,
            "probe_token_registry": true
        }),
        Some(&setup.token),
    )
    .await;
    let scan_status = scan.status();
    let scan_json: serde_json::Value = scan.json().await.unwrap();
    assert_eq!(scan_status, StatusCode::OK, "scan response: {scan_json}");

    let holdings = scan_json["holdings"].as_array().unwrap();
    let registry_holdings: Vec<_> = holdings
        .iter()
        .filter(|holding| holding["source"] == "token_registry:core-list")
        .collect();
    assert_eq!(registry_holdings.len(), 1, "holdings: {holdings:?}");
    assert_eq!(registry_holdings[0]["asset_kind"], "erc20");
    assert_eq!(
        registry_holdings[0]["asset_address"],
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(
        holdings
            .iter()
            .all(|holding| holding["asset_address"] != "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(
        holdings
            .iter()
            .all(|holding| holding["asset_address"] != "0xcccccccccccccccccccccccccccccccccccccccc")
    );

    let list = get(
        &setup.client,
        setup.addr,
        "/api/inventory/wallets",
        Some(&setup.token),
    )
    .await;
    let list_status = list.status();
    let list_json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(
        list_status,
        StatusCode::OK,
        "wallet list response: {list_json}"
    );
    let persisted = list_json["holdings"].as_array().unwrap();
    assert!(persisted.iter().any(|holding| {
        holding["source"] == "token_registry:core-list"
            && holding["asset_address"] == "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            && holding["asset_kind"] == "erc20"
    }));

    setup.handle.abort();
    setup.rpc_handle.abort();
}

#[tokio::test]
async fn inventory_token_registry_requires_auth() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let list = get(&client, addr, "/api/inventory/token-registry", None).await;
    assert_eq!(list.status(), StatusCode::UNAUTHORIZED);

    let import = post_json(
        &client,
        addr,
        "/api/inventory/token-registry/import",
        json!({
            "name": "core-list",
            "entries_json": registry_entries_json(),
        }),
        None,
    )
    .await;
    assert_eq!(import.status(), StatusCode::UNAUTHORIZED);

    let delete = post_json(
        &client,
        addr,
        "/api/inventory/token-registry/delete",
        json!({ "name": "core-list" }),
        None,
    )
    .await;
    assert_eq!(delete.status(), StatusCode::UNAUTHORIZED);

    handle.abort();
}

#[tokio::test]
async fn inventory_token_registry_rejects_invalid_imports() {
    let dir = TempDir::new().unwrap();
    let setup = setup_daemon_with_provider(&dir).await;

    assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({
                "name": "bad-list",
                "entries_json": registry_entries_json(),
                "file_path": "/tmp/tokens.json",
            }),
            Some(&setup.token),
        )
        .await,
        "both sources",
    )
    .await;
    assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({ "name": "bad-list" }),
            Some(&setup.token),
        )
        .await,
        "neither source",
    )
    .await;
    assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({
                "name": "bad-list",
                "entries_json": r#"[{"chain_id":1,"address":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","symbol":"AAA"}]"#,
            }),
            Some(&setup.token),
        )
        .await,
        "missing decimals",
    )
    .await;
    assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({
                "name": "bad-list",
                "entries_json": r#"[{"chain_id":1,"address":"not-an-address","symbol":"AAA","decimals":18}]"#,
            }),
            Some(&setup.token),
        )
        .await,
        "bad address",
    )
    .await;
    let network_body = assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({
                "name": "bad-list",
                "file_path": "https://example.com/tokens.json",
            }),
            Some(&setup.token),
        )
        .await,
        "network file path",
    )
    .await;
    assert!(network_body.contains("local"));
    assert!(network_body.contains("D-15"));
    assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/token-registry/import",
            json!({
                "name": "bad-list",
                "entries_json": "[]",
            }),
            Some(&setup.token),
        )
        .await,
        "empty entries",
    )
    .await;
    let scan_body = assert_bad_request(
        post_json(
            &setup.client,
            setup.addr,
            "/api/inventory/scan/evm",
            json!({
                "wallet_family": "eth-watch",
                "watch_addresses": [{
                    "address": "0x9858effd232b4033e47d90003d41ec34ecaeda94"
                }],
                "provider_profile": "mainnet",
                "gap_limit": 1,
                "max_index": 0,
                "probe_token_registry": true
            }),
            Some(&setup.token),
        )
        .await,
        "scan without lists",
    )
    .await;
    assert!(scan_body.contains("probe_token_registry"));

    setup.handle.abort();
    setup.rpc_handle.abort();
}

#[tokio::test]
async fn inventory_token_registry_imports_from_local_file() {
    let dir = TempDir::new().unwrap();
    let setup = setup_daemon_with_provider(&dir).await;
    let token_file = dir.path().join("tokens.json");
    std::fs::write(
        &token_file,
        r#"{"tokens":[{"chainId":1,"address":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","symbol":"AAA","decimals":18}]}"#,
    )
    .unwrap();

    let import = post_json(
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry/import",
        json!({
            "name": "file-list",
            "file_path": token_file.to_string_lossy(),
        }),
        Some(&setup.token),
    )
    .await;
    let status = import.status();
    let json: serde_json::Value = import.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "import response: {json}");
    assert_eq!(json["list"]["source"], "local-file");
    assert_eq!(json["list"]["entries"][0]["chain_id"], 1);

    setup.handle.abort();
    setup.rpc_handle.abort();
}

#[tokio::test]
async fn inventory_token_registry_recovers_from_corrupt_store() {
    let dir = TempDir::new().unwrap();
    let setup = setup_daemon_with_provider(&dir).await;
    import_core_list(&setup).await;
    std::fs::write(dir.path().join("token_registry.json"), b"not json {{{").unwrap();

    let list = get(
        &setup.client,
        setup.addr,
        "/api/inventory/token-registry",
        Some(&setup.token),
    )
    .await;
    let status = list.status();
    let json: serde_json::Value = list.json().await.unwrap();
    assert_eq!(status, StatusCode::OK, "list response: {json}");
    assert_eq!(json["lists"].as_array().unwrap().len(), 1);
    assert_eq!(json["lists"][0]["name"], "core-list");

    setup.handle.abort();
    setup.rpc_handle.abort();
}
