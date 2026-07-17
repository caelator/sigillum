//! List endpoint pagination, filtering, and sorting (plan task 1.5).
//!
//! Covers the six previously unbounded list endpoints:
//! `/api/queue/jobs`, `/api/inventory/wallets`, `/api/deposits/eth-stealth`,
//! `/api/plans/consolidation`, `/api/risk/findings`, `/api/discovery/jobs`.
//!
//! For each: the parameterless call is the legacy response (full list in
//! store order, no `pagination` key); limit/offset windows, filters, and
//! sorts behave as documented; invalid values fail with 400
//! `validation_failed` naming the parameter.
//!
//! Stores are seeded directly as versioned JSON envelopes before the daemon
//! starts (list endpoints load them per request), which keeps timestamps and
//! statuses fully deterministic.

use std::net::SocketAddr;
use std::path::Path;

use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

// ── Daemon rig ─────────────────────────────────────────────────────

async fn spawn_daemon(base_dir: std::path::PathBuf) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, _state) =
        sigillum_daemon::build_router(base_dir, addr.port()).expect("router should initialize");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

struct Rig {
    client: reqwest::Client,
    addr: SocketAddr,
    token: String,
    _dir: TempDir,
    handle: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn spawn() -> Self {
        let dir = TempDir::new().unwrap();
        seed_queue(dir.path());
        seed_deposits(dir.path());
        seed_inventory(dir.path());
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
        Self {
            client,
            addr,
            token,
            _dir: dir,
            handle,
        }
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .client
            .get(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        (status, body)
    }

    async fn get_ok(&self, path: &str) -> Value {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
        body
    }

    fn shutdown(self) {
        self.handle.abort();
    }
}

fn ids<'a>(body: &'a Value, key: &str) -> Vec<&'a str> {
    body[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} is an array: {body}"))
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect()
}

/// Asserts the legacy (parameterless) contract: 200, no `pagination` key,
/// and the list in store order.
fn assert_legacy(body: &Value, key: &str, want_ids: &[&str]) {
    assert!(
        body.get("pagination").is_none(),
        "legacy response must not carry pagination: {body}"
    );
    assert_eq!(ids(body, key), want_ids);
}

async fn assert_validation_failed(rig: &Rig, path: &str, param: &str) {
    let (status, body) = rig.get(path).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "GET {path}: {body}");
    assert_eq!(body["code"], "validation_failed", "GET {path} code: {body}");
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains(&format!("'{param}'")),
        "GET {path} error must name '{param}': {message}"
    );
}

fn assert_pagination(body: &Value, total: u64, limit: u64, offset: u64, has_more: bool) {
    assert_eq!(
        body["pagination"],
        json!({
            "total": total,
            "limit": limit,
            "offset": offset,
            "has_more": has_more,
        }),
        "pagination envelope: {body}"
    );
}

// ── Store seeds ────────────────────────────────────────────────────

fn write_store(base_dir: &Path, name: &str, schema: &str, version: u32, data: Value) {
    let envelope = json!({
        "schema": schema,
        "schema_version": version,
        "data": data,
    });
    std::fs::write(
        base_dir.join(name),
        serde_json::to_vec_pretty(&envelope).unwrap(),
    )
    .unwrap();
}

fn queue_job(id: &str, state: &str, created: u64, updated: u64, payload: Value) -> Value {
    let mut job = json!({
        "id": id,
        "state": state,
        "attempts": 0,
        "created_at_unix": created,
        "updated_at_unix": updated,
    });
    job.as_object_mut()
        .unwrap()
        .extend(payload.as_object().unwrap().clone());
    job
}

fn seed_queue(base_dir: &Path) {
    let stealth = json!({
        "kind": "eth_stealth_transfer",
        "wallet_profile": "payments",
        "stealth_address": "0x0000000000000000000000000000000000000001",
        "ephemeral_public_key_hex": "0x02",
        "value_wei_hex": "0x1",
    });
    let sweep = json!({
        "kind": "eth_stealth_native_sweep",
        "wallet_profile": "payments",
        "stealth_address": "0x0000000000000000000000000000000000000001",
        "ephemeral_public_key_hex": "0x02",
    });
    let erc20 = json!({
        "kind": "eth_stealth_erc20_transfer",
        "wallet_profile": "payments",
        "stealth_address": "0x0000000000000000000000000000000000000001",
        "ephemeral_public_key_hex": "0x02",
        "token_address": "0x00000000000000000000000000000000000000aa",
        "recipient_address": "0x00000000000000000000000000000000000000bb",
        "amount_hex": "0x1",
    });
    let plan_step = json!({
        "kind": "plan_step_execution",
        "plan_id": "plan_1",
        "step_id": "step_1",
        "chain_id": 1,
        "source_address": "0x0000000000000000000000000000000000000001",
        "derivation_path": "m/44'/60'/0'/0/0",
        "wallet_family": "eth-seed",
        "wallet_profile": "seed-a",
        "provider_profile": "mainnet",
        "action": "sweep_native",
        "asset_kind": "native",
        "amount_hex": "0x1",
        "call_label": "native.transfer(value)",
        "call_target_address": "0x0000000000000000000000000000000000000002",
        "call_data_hex": "0x",
        "simulation_evidence_hash_hex": "ab".repeat(32),
        "prerequisite_job_ids": [],
    });
    let seed_sweep = json!({
        "kind": "eth_seed_native_sweep",
        "wallet_profile": "seed-a",
        "address": "0x0000000000000000000000000000000000000003",
        "derivation_path": "m/44'/60'/0'/0/0",
    });
    write_store(
        base_dir,
        "queue.json",
        "sigillum.queue",
        5,
        json!({
            "jobs": [
                queue_job("job_1", "queued", 100, 500, stealth),
                queue_job("job_2", "blocked", 200, 400, sweep),
                queue_job("job_3", "confirmed", 300, 300, erc20),
                queue_job("job_4", "operator_action_required", 400, 200, plan_step),
                queue_job("job_5", "failed_terminal", 500, 100, seed_sweep),
            ]
        }),
    );
}

fn deposit(
    id: &str,
    status: &str,
    chain_id: u64,
    counterparty_id: Option<&str>,
    created: u64,
    updated: u64,
) -> Value {
    let mut deposit = json!({
        "id": id,
        "status": status,
        "asset_kind": "native",
        "wallet_profile": "payments",
        "chain_id": chain_id,
        "wallet": "0x0000000000000000000000000000000000000009",
        "short_name": "eth",
        "stealth_meta_address": "st:eth:0xabc",
        "stealth_address": "0x0000000000000000000000000000000000000001",
        "ephemeral_public_key_hex": "0x02",
        "view_tag_hex": "0x7f",
        "auto_queue_sweep": false,
        "created_at_unix": created,
        "updated_at_unix": updated,
    });
    if let Some(counterparty_id) = counterparty_id {
        deposit["counterparty_id"] = json!(counterparty_id);
    }
    deposit
}

fn seed_deposits(base_dir: &Path) {
    write_store(
        base_dir,
        "deposits.json",
        "sigillum.deposits",
        2,
        json!({
            "eth_stealth": [
                deposit("dep_1", "pending", 1, Some("party_a"), 100, 400),
                deposit("dep_2", "funded", 1, Some("party_b"), 200, 300),
                deposit("dep_3", "sweep_queued", 137, None, 300, 200),
                deposit("dep_4", "underfunded", 1, Some("party_a"), 400, 100),
            ]
        }),
    );
}

fn inventory_address(
    id: &str,
    chain_id: u64,
    activity_state: &str,
    balance: &str,
    address: &str,
    last_checked: u64,
) -> Value {
    json!({
        "id": id,
        "wallet_family": "eth-seed",
        "wallet_profile": "seed-a",
        "provider_profile": "mainnet",
        "chain_id": chain_id,
        "address": address,
        "derivation_path": "m/44'/60'/0'/0/0",
        "address_index": 0,
        "activity_state": activity_state,
        "native_balance_wei_hex": balance,
        "transaction_count": 0,
        "source": "local-rpc",
        "first_seen_at_unix": 1,
        "last_checked_at_unix": last_checked,
    })
}

fn discovery_job(id: &str, status: &str, started: u64, completed: Option<u64>) -> Value {
    let mut job = json!({
        "id": id,
        "status": status,
        "source": "local-rpc",
        "gap_limit": 20,
        "max_index": 100,
        "addresses_scanned": 0,
        "active_addresses": 0,
        "holdings_detected": 0,
        "started_at_unix": started,
    });
    if let Some(completed) = completed {
        job["completed_at_unix"] = json!(completed);
    }
    job
}

fn risk_finding(
    id: &str,
    risk_level: &str,
    category: &str,
    chain_id: u64,
    first_seen: u64,
) -> Value {
    json!({
        "id": id,
        "category": category,
        "risk_level": risk_level,
        "status": "open",
        "wallet_family": "eth-seed",
        "wallet_profile": "seed-a",
        "provider_profile": "mainnet",
        "chain_id": chain_id,
        "address": "0x0000000000000000000000000000000000000001",
        "subject_type": "address",
        "subject": "0x0000000000000000000000000000000000000001",
        "source": "local-risk-engine",
        "recommendation": "Review.",
        "first_seen_at_unix": first_seen,
        "last_checked_at_unix": first_seen,
    })
}

fn consolidation_plan(id: &str, status: &str, created: u64, updated: u64) -> Value {
    json!({
        "id": id,
        "status": status,
        "chain_id": 1,
        "created_at_unix": created,
        "updated_at_unix": updated,
        "summary": {
            "total_steps": 0,
            "blocked_steps": 0,
            "review_required_steps": 0,
            "approved_steps": 0,
            "executable_steps": 0,
            "value_items": 0,
        },
        "steps": [],
    })
}

fn seed_inventory(base_dir: &Path) {
    write_store(
        base_dir,
        "wallet_inventory.json",
        "sigillum.wallet-inventory",
        20,
        json!({
            "addresses": [
                inventory_address("addr_1", 1, "funded", "0x1", "0x00000000000000000000000000000000000000aa", 400),
                inventory_address("addr_2", 1, "empty", "0x0", "0x00000000000000000000000000000000000000bb", 100),
                inventory_address("addr_3", 137, "active", "0x0", "0x00000000000000000000000000000000000000cc", 300),
                inventory_address("addr_4", 1, "funded", "0x2", "0x00000000000000000000000000000000000000dd", 200),
            ],
            "jobs": [
                discovery_job("dj_1", "running", 100, None),
                discovery_job("dj_2", "completed", 200, Some(500)),
                discovery_job("dj_3", "failed", 300, Some(400)),
            ],
            "holdings": [],
            "risk_findings": [
                risk_finding("f_1", "critical", "watch_only_value", 1, 100),
                risk_finding("f_2", "high", "risky_approval", 1, 200),
                risk_finding("f_3", "medium", "dormant_wallet", 137, 300),
                risk_finding("f_4", "low", "claim_candidate", 1, 400),
            ],
            "consolidation_plans": [
                consolidation_plan("plan_1", "review_required", 100, 300),
                consolidation_plan("plan_2", "approved", 200, 200),
                consolidation_plan("plan_3", "blocked", 300, 100),
            ],
        }),
    );
}

// ── /api/queue/jobs ────────────────────────────────────────────────

#[tokio::test]
async fn queue_jobs_legacy_response_has_no_pagination_key() {
    let rig = Rig::spawn().await;
    let body = rig.get_ok("/api/queue/jobs").await;
    assert_legacy(
        &body,
        "jobs",
        &["job_1", "job_2", "job_3", "job_4", "job_5"],
    );
    // Only the `jobs` key is present (byte-identical legacy shape).
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["jobs"]);
    rig.shutdown();
}

#[tokio::test]
async fn queue_jobs_pagination_windows_and_boundaries() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/queue/jobs?limit=2").await;
    assert_eq!(ids(&body, "jobs"), ["job_1", "job_2"]);
    assert_pagination(&body, 5, 2, 0, true);

    let body = rig.get_ok("/api/queue/jobs?limit=2&offset=4").await;
    assert_eq!(ids(&body, "jobs"), ["job_5"]);
    assert_pagination(&body, 5, 2, 4, false);

    // Exact page: window reaches the end exactly.
    let body = rig.get_ok("/api/queue/jobs?limit=5").await;
    assert_eq!(ids(&body, "jobs").len(), 5);
    assert_pagination(&body, 5, 5, 0, false);

    // Offset beyond the end: empty window, has_more false.
    let body = rig.get_ok("/api/queue/jobs?limit=2&offset=9").await;
    assert!(body["jobs"].as_array().unwrap().is_empty());
    assert_pagination(&body, 5, 2, 9, false);

    // Offset-only: window is the remainder.
    let body = rig.get_ok("/api/queue/jobs?offset=3").await;
    assert_eq!(ids(&body, "jobs"), ["job_4", "job_5"]);
    assert_pagination(&body, 5, 2, 3, false);

    rig.shutdown();
}

#[tokio::test]
async fn queue_jobs_filters() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/queue/jobs?state=confirmed").await;
    assert_eq!(ids(&body, "jobs"), ["job_3"]);
    assert!(body.get("pagination").is_none());

    let body = rig
        .get_ok("/api/queue/jobs?state=operator_action_required")
        .await;
    assert_eq!(ids(&body, "jobs"), ["job_4"]);

    let body = rig.get_ok("/api/queue/jobs?kind=plan_step_execution").await;
    assert_eq!(ids(&body, "jobs"), ["job_4"]);

    let body = rig
        .get_ok("/api/queue/jobs?kind=eth_stealth_transfer")
        .await;
    assert_eq!(ids(&body, "jobs"), ["job_1"]);

    // chain_id matches only payloads that carry one (plan_step_execution).
    let body = rig.get_ok("/api/queue/jobs?chain_id=1").await;
    assert_eq!(ids(&body, "jobs"), ["job_4"]);
    let body = rig.get_ok("/api/queue/jobs?chain_id=137").await;
    assert!(body["jobs"].as_array().unwrap().is_empty());

    // Combined filter + window.
    let body = rig
        .get_ok("/api/queue/jobs?state=failed_terminal&limit=1")
        .await;
    assert_eq!(ids(&body, "jobs"), ["job_5"]);
    assert_pagination(&body, 1, 1, 0, false);

    rig.shutdown();
}

#[tokio::test]
async fn queue_jobs_sorts() {
    let rig = Rig::spawn().await;

    // Default direction for time fields is desc (newest first).
    let body = rig.get_ok("/api/queue/jobs?sort=created").await;
    assert_eq!(
        ids(&body, "jobs"),
        ["job_5", "job_4", "job_3", "job_2", "job_1"]
    );

    let body = rig.get_ok("/api/queue/jobs?sort=updated&order=asc").await;
    assert_eq!(
        ids(&body, "jobs"),
        ["job_5", "job_4", "job_3", "job_2", "job_1"]
    );

    // Sort + window compose.
    let body = rig.get_ok("/api/queue/jobs?sort=created&limit=2").await;
    assert_eq!(ids(&body, "jobs"), ["job_5", "job_4"]);
    assert_pagination(&body, 5, 2, 0, true);

    rig.shutdown();
}

#[tokio::test]
async fn queue_jobs_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/queue/jobs?state=bogus", "state").await;
    assert_validation_failed(&rig, "/api/queue/jobs?kind=bogus", "kind").await;
    assert_validation_failed(&rig, "/api/queue/jobs?sort=bogus", "sort").await;
    assert_validation_failed(&rig, "/api/queue/jobs?order=sideways", "order").await;
    assert_validation_failed(&rig, "/api/queue/jobs?limit=abc", "limit").await;
    assert_validation_failed(&rig, "/api/queue/jobs?offset=-1", "offset").await;
    assert_validation_failed(&rig, "/api/queue/jobs?chain_id=abc", "chain_id").await;
    // order without sort is rejected rather than silently dropped.
    assert_validation_failed(&rig, "/api/queue/jobs?order=asc", "order").await;
    rig.shutdown();
}

// ── /api/inventory/wallets ─────────────────────────────────────────

#[tokio::test]
async fn inventory_wallets_legacy_response_and_sibling_lists() {
    let rig = Rig::spawn().await;
    let body = rig.get_ok("/api/inventory/wallets").await;
    assert_legacy(
        &body,
        "addresses",
        &["addr_1", "addr_2", "addr_3", "addr_4"],
    );
    // Sibling lists ride along untouched.
    assert_eq!(ids(&body, "jobs"), ["dj_1", "dj_2", "dj_3"]);

    // Pagination applies to `addresses` only.
    let body = rig.get_ok("/api/inventory/wallets?limit=2").await;
    assert_eq!(ids(&body, "addresses"), ["addr_1", "addr_2"]);
    assert_pagination(&body, 4, 2, 0, true);
    assert_eq!(ids(&body, "jobs").len(), 3, "jobs list is not paginated");

    let body = rig.get_ok("/api/inventory/wallets?limit=2&offset=2").await;
    assert_eq!(ids(&body, "addresses"), ["addr_3", "addr_4"]);
    assert_pagination(&body, 4, 2, 2, false);

    let body = rig.get_ok("/api/inventory/wallets?offset=9").await;
    assert!(body["addresses"].as_array().unwrap().is_empty());
    assert_pagination(&body, 4, 0, 9, false);

    rig.shutdown();
}

#[tokio::test]
async fn inventory_wallets_filters() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/inventory/wallets?chain_id=1").await;
    assert_eq!(ids(&body, "addresses"), ["addr_1", "addr_2", "addr_4"]);

    let body = rig.get_ok("/api/inventory/wallets?funded=true").await;
    assert_eq!(ids(&body, "addresses"), ["addr_1", "addr_4"]);

    let body = rig.get_ok("/api/inventory/wallets?funded=false").await;
    assert_eq!(ids(&body, "addresses"), ["addr_2", "addr_3"]);

    let body = rig
        .get_ok("/api/inventory/wallets?chain_id=1&funded=true")
        .await;
    assert_eq!(ids(&body, "addresses"), ["addr_1", "addr_4"]);

    rig.shutdown();
}

#[tokio::test]
async fn inventory_wallets_sorts() {
    let rig = Rig::spawn().await;

    // `address` defaults to asc.
    let body = rig.get_ok("/api/inventory/wallets?sort=address").await;
    assert_eq!(
        ids(&body, "addresses"),
        ["addr_1", "addr_2", "addr_3", "addr_4"]
    );

    let body = rig
        .get_ok("/api/inventory/wallets?sort=address&order=desc")
        .await;
    assert_eq!(
        ids(&body, "addresses"),
        ["addr_4", "addr_3", "addr_2", "addr_1"]
    );

    // `last_scanned` defaults to desc (most recently scanned first).
    let body = rig.get_ok("/api/inventory/wallets?sort=last_scanned").await;
    assert_eq!(
        ids(&body, "addresses"),
        ["addr_1", "addr_3", "addr_4", "addr_2"]
    );

    let body = rig
        .get_ok("/api/inventory/wallets?sort=last_scanned&order=asc")
        .await;
    assert_eq!(
        ids(&body, "addresses"),
        ["addr_2", "addr_4", "addr_3", "addr_1"]
    );

    rig.shutdown();
}

#[tokio::test]
async fn inventory_wallets_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/inventory/wallets?funded=maybe", "funded").await;
    assert_validation_failed(&rig, "/api/inventory/wallets?sort=balance", "sort").await;
    assert_validation_failed(&rig, "/api/inventory/wallets?chain_id=x", "chain_id").await;
    assert_validation_failed(&rig, "/api/inventory/wallets?limit=", "limit").await;
    rig.shutdown();
}

// ── /api/deposits/eth-stealth ──────────────────────────────────────

#[tokio::test]
async fn deposits_legacy_and_windows() {
    let rig = Rig::spawn().await;
    let body = rig.get_ok("/api/deposits/eth-stealth").await;
    assert_legacy(&body, "deposits", &["dep_1", "dep_2", "dep_3", "dep_4"]);

    let body = rig.get_ok("/api/deposits/eth-stealth?limit=3").await;
    assert_eq!(ids(&body, "deposits"), ["dep_1", "dep_2", "dep_3"]);
    assert_pagination(&body, 4, 3, 0, true);

    let body = rig.get_ok("/api/deposits/eth-stealth?offset=3").await;
    assert_eq!(ids(&body, "deposits"), ["dep_4"]);
    assert_pagination(&body, 4, 1, 3, false);

    rig.shutdown();
}

#[tokio::test]
async fn deposits_filters() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/deposits/eth-stealth?status=funded").await;
    assert_eq!(ids(&body, "deposits"), ["dep_2"]);

    let body = rig
        .get_ok("/api/deposits/eth-stealth?status=sweep_queued")
        .await;
    assert_eq!(ids(&body, "deposits"), ["dep_3"]);

    let body = rig.get_ok("/api/deposits/eth-stealth?chain_id=137").await;
    assert_eq!(ids(&body, "deposits"), ["dep_3"]);

    let body = rig
        .get_ok("/api/deposits/eth-stealth?counterparty_id=party_a")
        .await;
    assert_eq!(ids(&body, "deposits"), ["dep_1", "dep_4"]);

    let body = rig
        .get_ok("/api/deposits/eth-stealth?counterparty_id=nobody")
        .await;
    assert!(body["deposits"].as_array().unwrap().is_empty());

    rig.shutdown();
}

#[tokio::test]
async fn deposits_sorts() {
    let rig = Rig::spawn().await;

    let body = rig
        .get_ok("/api/deposits/eth-stealth?sort=created&order=asc")
        .await;
    assert_eq!(ids(&body, "deposits"), ["dep_1", "dep_2", "dep_3", "dep_4"]);

    let body = rig.get_ok("/api/deposits/eth-stealth?sort=updated").await;
    assert_eq!(ids(&body, "deposits"), ["dep_1", "dep_2", "dep_3", "dep_4"]);

    let body = rig
        .get_ok("/api/deposits/eth-stealth?sort=updated&order=asc&limit=2")
        .await;
    assert_eq!(ids(&body, "deposits"), ["dep_4", "dep_3"]);
    assert_pagination(&body, 4, 2, 0, true);

    rig.shutdown();
}

#[tokio::test]
async fn deposits_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/deposits/eth-stealth?status=bogus", "status").await;
    assert_validation_failed(&rig, "/api/deposits/eth-stealth?sort=id", "sort").await;
    rig.shutdown();
}

// ── /api/plans/consolidation ───────────────────────────────────────

#[tokio::test]
async fn consolidation_plans_legacy_filter_sort_window() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/plans/consolidation").await;
    assert_legacy(&body, "plans", &["plan_1", "plan_2", "plan_3"]);

    let body = rig.get_ok("/api/plans/consolidation?status=approved").await;
    assert_eq!(ids(&body, "plans"), ["plan_2"]);

    let body = rig.get_ok("/api/plans/consolidation?status=blocked").await;
    assert_eq!(ids(&body, "plans"), ["plan_3"]);

    // Default direction for `updated` is desc.
    let body = rig.get_ok("/api/plans/consolidation?sort=updated").await;
    assert_eq!(ids(&body, "plans"), ["plan_1", "plan_2", "plan_3"]);

    let body = rig
        .get_ok("/api/plans/consolidation?sort=created&order=asc&limit=1&offset=1")
        .await;
    assert_eq!(ids(&body, "plans"), ["plan_2"]);
    assert_pagination(&body, 3, 1, 1, true);

    rig.shutdown();
}

#[tokio::test]
async fn consolidation_plans_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/plans/consolidation?status=bogus", "status").await;
    assert_validation_failed(&rig, "/api/plans/consolidation?sort=steps", "sort").await;
    rig.shutdown();
}

// ── /api/risk/findings ─────────────────────────────────────────────

#[tokio::test]
async fn risk_findings_legacy_filters_sorts() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/risk/findings").await;
    assert_legacy(&body, "findings", &["f_1", "f_2", "f_3", "f_4"]);

    let body = rig.get_ok("/api/risk/findings?severity=high").await;
    assert_eq!(ids(&body, "findings"), ["f_2"]);

    let body = rig.get_ok("/api/risk/findings?kind=risky_approval").await;
    assert_eq!(ids(&body, "findings"), ["f_2"]);

    // kind is a free-form exact match: no validation, empty on no match.
    let body = rig.get_ok("/api/risk/findings?kind=not_a_category").await;
    assert!(body["findings"].as_array().unwrap().is_empty());

    let body = rig.get_ok("/api/risk/findings?chain_id=137").await;
    assert_eq!(ids(&body, "findings"), ["f_3"]);

    // Severity sort ranks critical first under the default desc.
    let body = rig.get_ok("/api/risk/findings?sort=severity").await;
    assert_eq!(ids(&body, "findings"), ["f_1", "f_2", "f_3", "f_4"]);

    let body = rig
        .get_ok("/api/risk/findings?sort=severity&order=asc")
        .await;
    assert_eq!(ids(&body, "findings"), ["f_4", "f_3", "f_2", "f_1"]);

    let body = rig.get_ok("/api/risk/findings?sort=found_at").await;
    assert_eq!(ids(&body, "findings"), ["f_4", "f_3", "f_2", "f_1"]);

    let body = rig
        .get_ok("/api/risk/findings?severity=critical&limit=1")
        .await;
    assert_eq!(ids(&body, "findings"), ["f_1"]);
    assert_pagination(&body, 1, 1, 0, false);

    rig.shutdown();
}

#[tokio::test]
async fn risk_findings_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/risk/findings?severity=bogus", "severity").await;
    assert_validation_failed(&rig, "/api/risk/findings?sort=category", "sort").await;
    rig.shutdown();
}

// ── /api/discovery/jobs ────────────────────────────────────────────

#[tokio::test]
async fn discovery_jobs_legacy_filter_sort_window() {
    let rig = Rig::spawn().await;

    let body = rig.get_ok("/api/discovery/jobs").await;
    assert_legacy(&body, "jobs", &["dj_1", "dj_2", "dj_3"]);

    let body = rig.get_ok("/api/discovery/jobs?state=running").await;
    assert_eq!(ids(&body, "jobs"), ["dj_1"]);

    let body = rig.get_ok("/api/discovery/jobs?state=failed").await;
    assert_eq!(ids(&body, "jobs"), ["dj_3"]);

    let body = rig.get_ok("/api/discovery/jobs?sort=created").await;
    assert_eq!(ids(&body, "jobs"), ["dj_3", "dj_2", "dj_1"]);

    // `updated` = completed_at_unix, falling back to started_at_unix.
    let body = rig.get_ok("/api/discovery/jobs?sort=updated").await;
    assert_eq!(ids(&body, "jobs"), ["dj_2", "dj_3", "dj_1"]);

    let body = rig.get_ok("/api/discovery/jobs?limit=1&offset=2").await;
    assert_eq!(ids(&body, "jobs"), ["dj_3"]);
    assert_pagination(&body, 3, 1, 2, false);

    rig.shutdown();
}

#[tokio::test]
async fn discovery_jobs_invalid_params_are_validation_failed() {
    let rig = Rig::spawn().await;
    assert_validation_failed(&rig, "/api/discovery/jobs?state=bogus", "state").await;
    assert_validation_failed(&rig, "/api/discovery/jobs?order=asc", "order").await;
    rig.shutdown();
}
