//! Forget/prune for the at-rest linkage ledger (plan task 3.2):
//!
//! (a) a scanned-address prune removes rows from the store; a re-scan that
//!     does not re-derive the pruned index leaves it gone, a re-scan that
//!     does re-derive it re-observes a FRESH row (documented semantics:
//!     pruning removes history, not derivation), and purged counterparty
//!     bindings never resurrect;
//! (b) retired-allocation purge deletes the record and its binding, active
//!     allocations are refused with 409, unknown ids with 404;
//! (c) a profile delete with `prune_inventory: true` removes the profile,
//!     its inventory rows, scan state, receive allocations (active ones
//!     retire-then-purged), and counterparty bindings in one operation with
//!     one audit event carrying the per-store counts, while the flag
//!     absent/false preserves the legacy behavior byte-identically;
//! (d) auth and validation paths: no session -> 401, selector-less prune ->
//!     400 validation_failed, unmatched selectors -> 404, active purge ->
//!     409 conflict.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const FUNDED_BALANCE: &str = "0xde0b6b3a7640000";

// ── Daemon + stub provider fixtures ──────────────────────────────

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

/// Minimal EVM stub: answers chain id, block number, and a fixed balance for
/// funded addresses; everything else reads as empty.
async fn spawn_stub_provider(
    funded: BTreeSet<String>,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    async fn rpc_handler(
        State(funded): State<BTreeSet<String>>,
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
                    let address = request["params"][0]
                        .as_str()
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if funded.contains(&address) {
                        json!(FUNDED_BALANCE)
                    } else {
                        json!("0x0")
                    }
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
        .with_state(funded);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

// ── Test rig ─────────────────────────────────────────────────────

struct Rig {
    client: reqwest::Client,
    addr: SocketAddr,
    token: String,
    handle: tokio::task::JoinHandle<()>,
    rpc_handle: tokio::task::JoinHandle<()>,
    _dir: TempDir,
}

impl Rig {
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.post_as(path, body, Some(&self.token)).await
    }

    async fn post_as(&self, path: &str, body: Value, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = self.client.post(format!("http://{}{path}", self.addr));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.json(&body).send().await.unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
    }

    async fn get(&self, path: &str) -> Value {
        let response = self
            .client
            .get(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
        body
    }

    async fn scan(&self, wallet_profile: &str, max_index: u32) -> (StatusCode, Value) {
        self.post(
            "/api/inventory/scan/evm",
            json!({
                "wallet_profile": wallet_profile,
                "max_index": max_index,
                "gap_limit": 100,
            }),
        )
        .await
    }

    async fn inventory_addresses(&self) -> Vec<Value> {
        self.get("/api/inventory/wallets").await["addresses"]
            .as_array()
            .unwrap()
            .clone()
    }

    async fn inventory_rows_for(&self, wallet_profile: &str) -> Vec<Value> {
        self.inventory_addresses()
            .await
            .into_iter()
            .filter(|row| row["wallet_profile"] == wallet_profile)
            .collect()
    }

    async fn allocations(&self) -> Vec<Value> {
        self.get("/api/treasury/receive-addresses").await["allocations"]
            .as_array()
            .unwrap()
            .clone()
    }

    async fn audit_events(&self, kind: &str) -> Vec<Value> {
        self.get("/api/audit?limit=100").await["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == kind)
            .cloned()
            .collect()
    }
}

/// The xpub profile's derived receive addresses, in scan order.
fn xpub_addresses(count: u32) -> Vec<String> {
    let account_xpub =
        sigillum_core::derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
    let branch =
        sigillum_core::derive_ethereum_receive_branch_from_account_xpub(&account_xpub, 0).unwrap();
    (0..count)
        .map(|index| {
            sigillum_core::derive_ethereum_address_from_imported_xpub(&branch.receive_xpub, index)
                .unwrap()
                .address
                .to_ascii_lowercase()
        })
        .collect()
}

/// The seed profile's derived receive addresses, in scan order.
fn seed_addresses(count: u32) -> Vec<String> {
    let export =
        sigillum_core::derive_ethereum_xpub_receive_branch_from_mnemonic(TEST_MNEMONIC, None, 0)
            .unwrap();
    (0..count)
        .map(|index| {
            sigillum_core::derive_ethereum_address_from_xpub(&export.receive_xpub, index)
                .unwrap()
                .address
                .to_ascii_lowercase()
        })
        .collect()
}

/// Spin up a daemon with one stub provider and one imported account-xpub
/// wallet profile; `funded` addresses answer `FUNDED_BALANCE`.
async fn spawn_rig(funded: BTreeSet<String>) -> Rig {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
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

    let (rpc_addr, rpc_handle) = spawn_stub_provider(funded).await;
    let rig = Rig {
        client,
        addr,
        token,
        handle,
        rpc_handle,
        _dir: dir,
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
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "provider upsert: {body}");

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

async fn create_party(rig: &Rig, name: &str) -> String {
    let (status, body) = rig
        .post("/api/treasury/parties", json!({ "name": name }))
        .await;
    assert_eq!(status, StatusCode::OK, "party create: {body}");
    body["party"]["id"].as_str().unwrap().to_string()
}

async fn allocate(rig: &Rig, wallet_profile: &str, counterparty_id: Option<&str>) -> Value {
    let mut body = json!({
        "wallet_profile": wallet_profile,
        "purpose": format!("purpose-for-{wallet_profile}"),
    });
    if let Some(id) = counterparty_id {
        body["counterparty_id"] = json!(id);
    }
    let (status, body) = rig
        .post("/api/treasury/receive-addresses/allocate", body)
        .await;
    assert_eq!(status, StatusCode::OK, "allocate: {body}");
    body["allocation"].clone()
}

// ── (a) scanned-address prune + re-scan semantics ────────────────

#[tokio::test]
async fn prune_removes_rows_and_rescan_never_resurrect_bindings() {
    let funded: BTreeSet<String> = [xpub_addresses(4)[3].clone()].into_iter().collect();
    let prune_target = xpub_addresses(4)[3].clone();
    let rig = spawn_rig(funded).await;

    let (status, scan) = rig.scan("account-xpub", 5).await;
    assert_eq!(status, StatusCode::OK, "scan: {scan}");
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 6);

    // A party + allocation binding exists before the prune; address pruning
    // must not touch it.
    let party_id = create_party(&rig, "Acme").await;
    let allocation = allocate(&rig, "account-xpub", Some(&party_id)).await;
    assert_eq!(allocation["status"], "active");

    // Prune the one funded row (index 3) by address.
    let (status, body) = rig
        .post(
            "/api/inventory/addresses/delete",
            json!({ "address": prune_target.clone() }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "prune: {body}");
    assert_eq!(body["status"], "pruned");
    assert_eq!(body["pruned"]["addresses"], json!(1));
    assert_eq!(body["pruned"]["holdings"], json!(1));
    assert_eq!(body["pruned"]["block_cursors"], json!(0));

    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .all(|row| row["address"].as_str().unwrap() != prune_target)
    );

    // A re-scan that does NOT re-derive the pruned index leaves it gone.
    let (status, rescan) = rig.scan("account-xpub", 2).await;
    assert_eq!(status, StatusCode::OK, "rescan: {rescan}");
    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .all(|row| row["address"].as_str().unwrap() != prune_target),
        "pruned row must stay gone when the re-scan does not re-derive it"
    );

    // A re-scan that DOES re-derive the index re-observes it as a fresh row
    // (documented: pruning removes history, not derivation).
    let (status, rescan) = rig.scan("account-xpub", 5).await;
    assert_eq!(status, StatusCode::OK, "full rescan: {rescan}");
    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows.len(), 6);
    let revived = rows
        .iter()
        .find(|row| row["address"].as_str().unwrap() == prune_target)
        .expect("re-derived index is re-observed");
    assert!(revived["first_seen_at_unix"].as_u64().unwrap() > 0);

    // The allocation and its counterparty binding survived address pruning.
    let allocations = rig.allocations().await;
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0]["counterparty_id"], json!(party_id));

    // Retire (via rotate) and purge the original allocation, then prove no
    // re-scan resurrects the purged binding.
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/rotate",
            json!({ "allocation_id": allocation["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "rotate: {body}");
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": allocation["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "purge: {body}");
    assert_eq!(body["counterparty_binding_removed"], json!(true));

    let (status, rescan) = rig.scan("account-xpub", 5).await;
    assert_eq!(status, StatusCode::OK, "post-purge rescan: {rescan}");
    let allocations = rig.allocations().await;
    assert_eq!(
        allocations.len(),
        1,
        "the purged allocation must never resurrect: {allocations:?}"
    );
    assert!(
        allocations
            .iter()
            .all(|entry| entry["id"] != allocation["id"])
    );
    // The counterparty record itself always remains.
    let parties = rig.get("/api/treasury/parties").await["parties"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0]["id"], json!(party_id));

    // The prune audit event carries counts, never the address value.
    let events = rig.audit_events("wallet_inventory.addresses.prune").await;
    assert_eq!(events.len(), 1);
    let details = &events[0]["details"];
    assert_eq!(details["scoped_by_address"], json!(true));
    assert_eq!(details["addresses"], json!(1));
    assert_eq!(details["holdings"], json!(1));
    assert!(
        details.get("address").is_none(),
        "the pruned address must not land in the audit log: {details}"
    );

    rig.handle.abort();
    rig.rpc_handle.abort();
}

// ── (b) retired-allocation purge ─────────────────────────────────

#[tokio::test]
async fn retired_allocation_purge_lifecycle_and_conflicts() {
    let rig = spawn_rig(BTreeSet::new()).await;

    // Active allocations refuse to purge with 409.
    let active = allocate(&rig, "account-xpub", None).await;
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": active["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "active purge: {body}");
    assert_eq!(body["code"], "conflict");
    assert_eq!(rig.allocations().await.len(), 1);

    // Unknown ids are a 404.
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": "alloc_missing" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown purge: {body}");
    assert_eq!(body["code"], "not_found");

    // Rotate retires the original; the retired record purges cleanly.
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/rotate",
            json!({ "allocation_id": active["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "rotate: {body}");
    assert_eq!(rig.allocations().await.len(), 2);

    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": active["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "retired purge: {body}");
    assert_eq!(body["status"], "purged");
    assert_eq!(body["counterparty_binding_removed"], json!(false));
    let remaining = rig.allocations().await;
    assert_eq!(remaining.len(), 1);
    assert!(remaining.iter().all(|entry| entry["id"] != active["id"]));

    // A bound retired allocation reports the binding removal; the
    // counterparty record remains.
    let party_id = create_party(&rig, "Bound Party").await;
    let bound = allocate(&rig, "account-xpub", Some(&party_id)).await;
    let (status, _) = rig
        .post(
            "/api/treasury/receive-addresses/rotate",
            json!({ "allocation_id": bound["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": bound["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "bound purge: {body}");
    assert_eq!(body["counterparty_binding_removed"], json!(true));
    let parties = rig.get("/api/treasury/parties").await["parties"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(parties.len(), 1);

    let purge_events = rig.audit_events("treasury.receive.purge").await;
    assert_eq!(purge_events.len(), 2);
    assert!(
        purge_events
            .iter()
            .any(|event| event["details"]["counterparty_binding_removed"] == json!(true))
    );

    rig.handle.abort();
    rig.rpc_handle.abort();
}

// ── (c) profile-delete cascade ───────────────────────────────────

async fn upsert_seed_profile(rig: &Rig, name: &str) {
    let (status, body) = rig
        .post(
            "/api/profiles/eth-seed/upsert",
            json!({
                "name": name,
                "mnemonic": TEST_MNEMONIC,
                "project_account": 0,
                "provider_profile": "mainnet",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "seed upsert: {body}");
}

#[tokio::test]
async fn evm_provider_delete_with_prune_inventory_forgets_its_observations() {
    let rig = spawn_rig(BTreeSet::new()).await;

    // Two extra providers no wallet profile references: one deleted with the
    // flag off (legacy orphaning), one with the cascade on.
    let mut legacy_handles = Vec::new();
    for name in ["legacy-a", "legacy-b"] {
        let (rpc_addr, rpc_handle) = spawn_stub_provider(BTreeSet::new()).await;
        let (status, body) = rig
            .post(
                "/api/profiles/evm/upsert",
                json!({
                    "name": name,
                    "rpc_url": format!("http://{rpc_addr}/"),
                    "auth_token_key": "alchemy",
                    "chain_id": 1,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "provider upsert {name}: {body}");

        let (status, scan) = rig
            .post(
                "/api/inventory/scan/evm",
                json!({
                    "wallet_profile": "account-xpub",
                    "provider_profile": name,
                    "max_index": 1,
                    "gap_limit": 100,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "scan via {name}: {scan}");
        legacy_handles.push(rpc_handle);
    }
    let rows_for = |rows: &[Value], provider: &str| {
        rows.iter()
            .filter(|row| row["provider_profile"] == provider)
            .count()
    };
    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows_for(&rows, "legacy-a"), 2);
    assert_eq!(rows_for(&rows, "legacy-b"), 2);

    // Flag off: today's orphaning behavior, byte-identical response shape.
    let (status, body) = rig
        .post("/api/profiles/evm/delete", json!({ "name": "legacy-a" }))
        .await;
    assert_eq!(status, StatusCode::OK, "legacy-a delete: {body}");
    assert!(body.get("pruned_inventory").is_none());
    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows_for(&rows, "legacy-a"), 2, "flag-off delete keeps rows");

    // Cascade on: only that provider's observation rows and scan jobs die.
    let (status, body) = rig
        .post(
            "/api/profiles/evm/delete",
            json!({ "name": "legacy-b", "prune_inventory": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "legacy-b delete: {body}");
    let summary = &body["pruned_inventory"];
    assert_eq!(summary["addresses"], json!(2), "summary: {summary}");
    assert_eq!(summary["jobs"], json!(1), "summary: {summary}");
    assert_eq!(
        summary["allocations_active"],
        json!(0),
        "summary: {summary}"
    );
    let rows = rig.inventory_rows_for("account-xpub").await;
    assert_eq!(rows_for(&rows, "legacy-b"), 0, "cascade forgets its rows");
    assert_eq!(
        rows_for(&rows, "legacy-a"),
        2,
        "other providers keep theirs"
    );

    // A provider still referenced by a wallet profile stays 409, flag or not.
    let (status, body) = rig
        .post(
            "/api/profiles/evm/delete",
            json!({ "name": "mainnet", "prune_inventory": true }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "referenced provider: {body}");
    assert_eq!(body["code"], "conflict");

    rig.handle.abort();
    rig.rpc_handle.abort();
    for handle in legacy_handles {
        handle.abort();
    }
}

#[tokio::test]
async fn account_index_selector_prunes_one_derivation_branch() {
    let funded: BTreeSet<String> = [xpub_addresses(3)[0].clone()].into_iter().collect();
    let rig = spawn_rig(funded).await;
    let (status, scan) = rig.scan("account-xpub", 2).await;
    assert_eq!(status, StatusCode::OK, "scan: {scan}");
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 3);

    // Every row of this single-account profile sits on account 0: pruning
    // account 0 forgets all three rows and the funded holding.
    let (status, body) = rig
        .post(
            "/api/inventory/addresses/delete",
            json!({ "account_index": 0 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "account prune: {body}");
    assert_eq!(body["pruned"]["addresses"], json!(3));
    assert_eq!(body["pruned"]["holdings"], json!(1));
    assert!(rig.inventory_rows_for("account-xpub").await.is_empty());

    // An account branch nothing was ever scanned under matches nothing.
    let (status, body) = rig
        .post(
            "/api/inventory/addresses/delete",
            json!({ "account_index": 7 }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown account: {body}");
    assert_eq!(body["code"], "not_found");

    rig.handle.abort();
    rig.rpc_handle.abort();
}

#[tokio::test]
async fn profile_delete_without_flag_preserves_legacy_behavior() {
    let rig = spawn_rig(BTreeSet::new()).await;
    upsert_seed_profile(&rig, "seed-main").await;

    let (status, scan) = rig.scan("seed-main", 2).await;
    assert_eq!(status, StatusCode::OK, "scan: {scan}");
    assert!(!rig.inventory_rows_for("seed-main").await.is_empty());
    allocate(&rig, "seed-main", None).await;

    // Legacy path: the flag absent. The response carries no prune summary
    // and every inventory row + allocation survives (today's orphaning).
    let (status, body) = rig
        .post(
            "/api/profiles/eth-seed/delete",
            json!({ "name": "seed-main" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "delete: {body}");
    assert_eq!(body["status"], "deleted");
    assert!(
        body.get("pruned_inventory").is_none(),
        "legacy delete response must be byte-identical (no pruned_inventory): {body}"
    );
    assert!(
        !rig.inventory_rows_for("seed-main").await.is_empty(),
        "legacy delete leaves scanned history behind"
    );
    assert_eq!(
        rig.allocations().await.len(),
        1,
        "legacy delete leaves the allocation orphaned"
    );
    assert!(
        rig.audit_events("wallet_inventory.profile_prune")
            .await
            .is_empty()
    );

    rig.handle.abort();
    rig.rpc_handle.abort();
}

#[tokio::test]
async fn profile_delete_with_prune_inventory_cascades_atomically() {
    let funded: BTreeSet<String> = [seed_addresses(1)[0].clone()].into_iter().collect();
    let rig = spawn_rig(funded).await;
    upsert_seed_profile(&rig, "seed-main").await;

    // Seed-main history: 3 receive rows (0..=2, one funded) + 3 control
    // rows (sponsor/hot/treasury) = 6 rows, 1 holding row.
    let (status, scan) = rig.scan("seed-main", 2).await;
    assert_eq!(status, StatusCode::OK, "seed scan: {scan}");
    assert_eq!(rig.inventory_rows_for("seed-main").await.len(), 6);
    // Control profile history that must survive the cascade.
    let (status, scan) = rig.scan("account-xpub", 2).await;
    assert_eq!(status, StatusCode::OK, "xpub scan: {scan}");
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 3);

    let party_id = create_party(&rig, "Cascade Party").await;
    let retired = allocate(&rig, "seed-main", Some(&party_id)).await;
    let (status, body) = rig
        .post(
            "/api/treasury/receive-addresses/rotate",
            json!({ "allocation_id": retired["id"] }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "rotate: {body}");
    let surviving_allocation = allocate(&rig, "account-xpub", Some(&party_id)).await;
    assert_eq!(rig.allocations().await.len(), 3);

    let (status, body) = rig
        .post(
            "/api/profiles/eth-seed/delete",
            json!({ "name": "seed-main", "prune_inventory": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "cascade delete: {body}");
    assert_eq!(body["status"], "deleted");

    // One response summary with exact per-store counts.
    let summary = &body["pruned_inventory"];
    assert_eq!(summary["addresses"], json!(6), "summary: {summary}");
    assert_eq!(summary["holdings"], json!(1), "summary: {summary}");
    assert_eq!(summary["jobs"], json!(1), "summary: {summary}");
    assert_eq!(
        summary["allocations_active"],
        json!(1),
        "summary: {summary}"
    );
    assert_eq!(
        summary["allocations_retired"],
        json!(1),
        "summary: {summary}"
    );
    assert_eq!(
        summary["counterparty_bindings"],
        json!(2),
        "summary: {summary}"
    );

    // The profile's rows are gone; the control profile's rows survive.
    assert!(rig.inventory_rows_for("seed-main").await.is_empty());
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 3);

    // Only the control allocation survives; the party itself remains.
    let allocations = rig.allocations().await;
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0]["id"], surviving_allocation["id"]);
    let parties = rig.get("/api/treasury/parties").await["parties"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(parties.len(), 1);

    // The seed-only scan job is gone; the xpub scan job survives.
    let jobs = rig.get("/api/discovery/jobs").await["jobs"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["wallet_profiles"], json!(["account-xpub"]));

    // One audit event carries the same per-store counts.
    let events = rig.audit_events("wallet_inventory.profile_prune").await;
    assert_eq!(events.len(), 1, "exactly one cascade audit event");
    let details = &events[0]["details"];
    assert_eq!(details["profile_kind"], "eth-seed");
    assert_eq!(details["name"], "seed-main");
    for key in [
        "addresses",
        "holdings",
        "jobs",
        "allocations_active",
        "allocations_retired",
        "counterparty_bindings",
    ] {
        assert_eq!(details[key], summary[key], "audit count {key}");
    }

    // Re-scanning the surviving profile stays healthy and never recreates
    // the purged allocations.
    let (status, rescan) = rig.scan("account-xpub", 2).await;
    assert_eq!(status, StatusCode::OK, "post-cascade rescan: {rescan}");
    assert_eq!(rig.allocations().await.len(), 1);

    rig.handle.abort();
    rig.rpc_handle.abort();
}

#[tokio::test]
async fn xpub_profile_delete_with_prune_inventory_cascades() {
    let rig = spawn_rig(BTreeSet::new()).await;
    let (status, scan) = rig.scan("account-xpub", 3).await;
    assert_eq!(status, StatusCode::OK, "scan: {scan}");
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 4);
    allocate(&rig, "account-xpub", None).await;

    let (status, body) = rig
        .post(
            "/api/profiles/eth-xpub/delete",
            json!({ "name": "account-xpub", "prune_inventory": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "cascade delete: {body}");
    let summary = &body["pruned_inventory"];
    assert_eq!(summary["addresses"], json!(4), "summary: {summary}");
    assert_eq!(
        summary["allocations_active"],
        json!(1),
        "summary: {summary}"
    );
    assert!(rig.inventory_rows_for("account-xpub").await.is_empty());
    assert!(rig.allocations().await.is_empty());

    rig.handle.abort();
    rig.rpc_handle.abort();
}

// ── (d) auth + validation paths ──────────────────────────────────

#[tokio::test]
async fn prune_routes_require_session_and_validate_selectors() {
    let rig = spawn_rig(BTreeSet::new()).await;
    let (status, scan) = rig.scan("account-xpub", 1).await;
    assert_eq!(status, StatusCode::OK, "scan: {scan}");

    // No session -> 401 on every new verb and the cascade flag path.
    for (path, body) in [
        (
            "/api/inventory/addresses/delete",
            json!({ "address": xpub_addresses(1)[0] }),
        ),
        (
            "/api/treasury/receive-addresses/purge",
            json!({ "allocation_id": "alloc_x" }),
        ),
        (
            "/api/profiles/eth-xpub/delete",
            json!({ "name": "account-xpub", "prune_inventory": true }),
        ),
    ] {
        let (status, body) = rig.post_as(path, body, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        assert_eq!(body["code"], "unauthorized");
    }

    // Selector-less prune -> 400 validation_failed.
    let (status, body) = rig.post("/api/inventory/addresses/delete", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty prune: {body}");
    assert_eq!(body["code"], "validation_failed");

    // Unknown wallet_family -> 400 validation_failed.
    let (status, body) = rig
        .post(
            "/api/inventory/addresses/delete",
            json!({ "wallet_family": "btc" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad family: {body}");
    assert_eq!(body["code"], "validation_failed");

    // Selectors matching nothing -> 404 not_found.
    let (status, body) = rig
        .post(
            "/api/inventory/addresses/delete",
            json!({ "wallet_profile": "no-such-profile" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unmatched prune: {body}");
    assert_eq!(body["code"], "not_found");

    // Profile delete of a missing profile still 404s with the flag set.
    let (status, body) = rig
        .post(
            "/api/profiles/eth-xpub/delete",
            json!({ "name": "no-such-profile", "prune_inventory": true }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "missing profile: {body}");
    assert_eq!(body["code"], "not_found");

    // Nothing was deleted by the error paths above.
    assert_eq!(rig.inventory_rows_for("account-xpub").await.len(), 2);

    rig.handle.abort();
    rig.rpc_handle.abort();
}
