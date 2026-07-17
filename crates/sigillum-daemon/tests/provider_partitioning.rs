//! Provider partitioning for EVM inventory scans (plan task 3.1):
//!
//! (a) two same-chain stub providers each observe a disjoint address subset
//!     whose union equals the full scanned address set;
//! (b) per-chain results are identical to a non-partitioned scan, including
//!     gap-limit accounting;
//! (c) the stable hash assignment maps an address to the same provider
//!     across scans;
//! (d) a single provider per chain keeps behavior byte-identical to the
//!     flag-off scan;
//! (e) a partitioned async scan cancels mid-run and resumes with zero
//!     duplicate observations and disjoint coverage intact.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Same value in every test of this binary, so the parallel test threads
/// never disagree (mirrors tests/scheduler.rs): tests never sleep.
fn disable_jitter() {
    unsafe { std::env::set_var("SIGILLUM_SCAN_PARTITION_JITTER_MAX_MS", "0") };
}

// ── Daemon + recording provider fixtures ─────────────────────────

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

/// Shared state behind the same-chain stub providers: each server records
/// the addresses IT observed via `eth_getBalance`, answers a fixed balance
/// for funded addresses, and can park the Nth GLOBAL balance call (across
/// both providers) so a test can cancel while a scan is definitely in
/// flight. With partitioning engaged there is exactly one balance call per
/// scanned index, so global call N corresponds to address index N-1.
struct PartitionRpcState {
    /// provider name -> observed addresses, in call order.
    observed: Mutex<BTreeMap<String, Vec<String>>>,
    /// addresses answered with `FUNDED_BALANCE`; everything else is empty.
    funded: BTreeSet<String>,
    balance_calls: AtomicUsize,
    /// 1-based global balance call to park at; 0 disarms the gate.
    gate_at_balance_call: AtomicUsize,
    gate_release: tokio::sync::Notify,
    gate_waiting: AtomicBool,
}

impl PartitionRpcState {
    fn new(funded: BTreeSet<String>) -> Self {
        Self {
            observed: Mutex::new(BTreeMap::new()),
            funded,
            balance_calls: AtomicUsize::new(0),
            gate_at_balance_call: AtomicUsize::new(0),
            gate_release: tokio::sync::Notify::new(),
            gate_waiting: AtomicBool::new(false),
        }
    }

    fn observed_sets(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.observed
            .lock()
            .unwrap()
            .iter()
            .map(|(name, addresses)| {
                (name.clone(), addresses.iter().cloned().collect::<BTreeSet<_>>())
            })
            .collect()
    }
}

async fn spawn_recording_provider(
    state: Arc<PartitionRpcState>,
    name: &'static str,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    async fn rpc_handler(
        State((state, name)): State<(Arc<PartitionRpcState>, &'static str)>,
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
                    state
                        .observed
                        .lock()
                        .unwrap()
                        .entry(name.to_string())
                        .or_default()
                        .push(address.clone());
                    let call = state.balance_calls.fetch_add(1, Ordering::SeqCst) + 1;
                    let gate_at = state.gate_at_balance_call.load(Ordering::SeqCst);
                    if gate_at != 0 && call == gate_at {
                        // Register the waiter before announcing the gate so a
                        // release can never be missed.
                        let notified = state.gate_release.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        state.gate_waiting.store(true, Ordering::SeqCst);
                        // A broken test must fail, not hang forever.
                        let _ = tokio::time::timeout(Duration::from_secs(30), notified).await;
                        state.gate_waiting.store(false, Ordering::SeqCst);
                        state.gate_at_balance_call.store(0, Ordering::SeqCst);
                    }
                    if state.funded.contains(&address) {
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
        .with_state((state, name));
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
    rpc: Arc<PartitionRpcState>,
    handle: tokio::task::JoinHandle<()>,
    rpc_handles: Vec<tokio::task::JoinHandle<()>>,
    _dir: TempDir,
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

    async fn wait_for_operation(&self, operation_id: &str, want: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let body = self.get(&format!("/api/operations/{operation_id}")).await;
            if body["operation"]["state"] == want {
                return body;
            }
            assert!(
                Instant::now() < deadline,
                "operation {operation_id} never reached {want}: {body}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_gate(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.rpc.gate_waiting.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "provider gate was never reached");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn release_gate(&self) {
        self.rpc.gate_release.notify_waiters();
    }

    fn balance_calls(&self) -> usize {
        self.rpc.balance_calls.load(Ordering::SeqCst)
    }

    async fn discovery_job(&self, job_id: &str) -> Value {
        let jobs = self.get("/api/discovery/jobs").await;
        jobs["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == job_id)
            .cloned()
            .unwrap_or_else(|| panic!("job {job_id} missing: {jobs}"))
    }

    async fn scan(&self, extra: Value) -> (StatusCode, Value) {
        let mut body = json!({
            "wallet_family": "eth-xpub",
            "wallet_profile": "account-xpub",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.post("/api/inventory/scan/evm", body).await
    }
}

/// Spin up a daemon with one stub provider per name (all on chain 1) and
/// one imported account-xpub wallet profile.
async fn spawn_rig(provider_names: &[&'static str], funded: BTreeSet<String>) -> Rig {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let rpc = Arc::new(PartitionRpcState::new(funded));
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

    let mut rpc_handles = Vec::new();
    for name in provider_names {
        let (rpc_addr, rpc_handle) = spawn_recording_provider(rpc.clone(), name).await;
        rpc_handles.push(rpc_handle);
        let response = client
            .post(format!("http://{addr}/api/profiles/evm/upsert"))
            .bearer_auth(&token)
            .json(&json!({
                "name": name,
                "rpc_url": format!("http://{rpc_addr}/"),
                "auth_token_key": "alchemy",
                "chain_id": 1,
            }))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "provider upsert {name}: {body}");
    }

    let rig = Rig {
        client,
        addr,
        token,
        rpc,
        handle,
        rpc_handles,
        _dir: dir,
    };

    let (status, body) = rig
        .post(
            "/api/api-keys/set",
            json!({ "key": "alchemy", "value": "rpc-test-token" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "api key set: {body}");

    let account_xpub =
        sigillum_core::derive_ethereum_account_xpub_from_mnemonic(TEST_MNEMONIC, None, 0).unwrap();
    let (status, body) = rig
        .post(
            "/api/profiles/eth-xpub/upsert",
            json!({
                "name": "account-xpub",
                "project_account": 0,
                "provider_profile": provider_names[0],
                "external_account_xpub": account_xpub,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "xpub upsert: {body}");

    rig
}

/// The receive addresses the daemon derives for the test xpub, in scan
/// order, lowercased to match the wire form.
fn derived_addresses(count: u32) -> Vec<String> {
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

/// Per-chain scan view collapsed over provider identity: address →
/// (activity, balance) plus the detected-holdings multiset.
fn per_chain_view(
    scan: &Value,
) -> (
    BTreeMap<String, (String, String)>,
    BTreeSet<(String, String, String)>,
) {
    let addresses = scan["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["address"].as_str().unwrap().to_ascii_lowercase(),
                (
                    row["activity_state"].as_str().unwrap().to_string(),
                    row["native_balance_wei_hex"].as_str().unwrap().to_string(),
                ),
            )
        })
        .collect();
    let holdings = scan["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["address"].as_str().unwrap().to_ascii_lowercase(),
                row["asset_kind"].as_str().unwrap().to_string(),
                row["amount_hex"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    (addresses, holdings)
}

fn scanned_indices(scan: &Value) -> BTreeSet<u64> {
    scan["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["address_index"].as_u64().unwrap())
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────

/// (a) PLAN ACCEPTANCE: two same-chain providers each observe a disjoint
/// subset, the union equals the full address set, the job records the
/// partitioning with per-provider counts, and every address row names the
/// provider that actually served it.
#[tokio::test]
async fn partitioned_scan_splits_same_chain_probes_into_disjoint_subsets() {
    disable_jitter();
    let expected: BTreeSet<String> = derived_addresses(10).into_iter().collect();
    let rig = spawn_rig(&["mainnet-a", "mainnet-b"], BTreeSet::new()).await;

    let (status, scan) = rig
        .scan(json!({
            "max_index": 9,
            "gap_limit": 10,
            "partition_providers": true,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    assert_eq!(scan["job"]["status"], "completed");
    assert_eq!(scan["job"]["partition_providers"], json!(true));
    assert_eq!(scan["job"]["addresses_scanned"], json!(10));

    let observed = rig.rpc.observed_sets();
    let a = observed.get("mainnet-a").cloned().unwrap_or_default();
    let b = observed.get("mainnet-b").cloned().unwrap_or_default();
    assert!(
        a.is_disjoint(&b),
        "providers must observe disjoint subsets: a={a:?} b={b:?}"
    );
    assert!(
        !a.is_empty() && !b.is_empty(),
        "both providers must serve a share: a={a:?} b={b:?}"
    );
    let union: BTreeSet<String> = a.union(&b).cloned().collect();
    assert_eq!(union, expected, "union must equal the full address set");

    // Job-level provenance: per-provider counts match what the stubs saw
    // and sum into addresses_scanned.
    let counts = scan["job"]["provider_partition_observations"]
        .as_array()
        .expect("partitioned job carries per-provider counts");
    assert_eq!(counts.len(), 2);
    let counted: usize = counts
        .iter()
        .map(|entry| entry["addresses_observed"].as_u64().unwrap() as usize)
        .sum();
    assert_eq!(counted, 10);
    for entry in counts {
        let name = entry["provider_profile"].as_str().unwrap();
        let want = match name {
            "mainnet-a" => a.len(),
            "mainnet-b" => b.len(),
            other => panic!("unexpected provider in counts: {other}"),
        };
        assert_eq!(entry["addresses_observed"], json!(want), "count for {name}");
        assert_eq!(entry["chain_id"], json!(1));
    }

    // Row-level provenance: each scanned address row names its serving
    // provider, and there is exactly one row per address (no duplicates).
    let rows = scan["addresses"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    for row in rows {
        let address = row["address"].as_str().unwrap().to_ascii_lowercase();
        let provider = row["provider_profile"].as_str().unwrap();
        let served = match provider {
            "mainnet-a" => &a,
            "mainnet-b" => &b,
            other => panic!("unexpected provider on address row: {other}"),
        };
        assert!(
            served.contains(&address),
            "row says {provider} served {address} but the stub never saw it"
        );
    }

    rig.handle.abort();
    for handle in &rig.rpc_handles {
        handle.abort();
    }
}

/// (b) RESULT CORRECTNESS: a partitioned two-provider scan yields the same
/// per-chain holdings and address activity as a non-partitioned
/// single-provider scan over the same wallet — including identical
/// gap-limit accounting (same index coverage with funded indices resetting
/// the empty run).
#[tokio::test]
async fn partitioned_results_match_nonpartitioned_scan_per_chain() {
    disable_jitter();
    let expected = derived_addresses(20);
    // Fund indices 1 and 4: with gap_limit 4 the walk covers 0..=8 in both
    // scans (funded hits reset the empty run; the walk stops after four
    // consecutive empties at index 8).
    let funded: BTreeSet<String> = [expected[1].clone(), expected[4].clone()]
        .into_iter()
        .collect();
    let scan_args = json!({ "max_index": 20, "gap_limit": 4 });

    let reference = spawn_rig(&["mainnet"], funded.clone()).await;
    let (status, reference_scan) = reference.scan(scan_args.clone()).await;
    assert_eq!(status, StatusCode::OK, "reference scan: {reference_scan}");

    let partitioned = spawn_rig(&["mainnet-a", "mainnet-b"], funded).await;
    let (status, partitioned_scan) = partitioned
        .scan(json!({ "max_index": 20, "gap_limit": 4, "partition_providers": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "partitioned scan: {partitioned_scan}");
    let _ = scan_args;

    // Identical per-chain results once provider identity is collapsed.
    assert_eq!(
        per_chain_view(&reference_scan),
        per_chain_view(&partitioned_scan),
        "partitioned scan must observe the same per-chain holdings/activity"
    );
    // Identical gap accounting: same index coverage in both scans.
    assert_eq!(
        scanned_indices(&reference_scan),
        scanned_indices(&partitioned_scan),
    );
    assert_eq!(
        scanned_indices(&reference_scan),
        (0..=8).collect::<BTreeSet<u64>>(),
        "funded indices 1 and 4 reset the empty run; walk stops at 4 empties"
    );
    // One observation per index in both scans (the reference has a single
    // provider; the partitioned scan splits the same count across two).
    assert_eq!(reference_scan["job"]["addresses_scanned"], json!(9));
    assert_eq!(partitioned_scan["job"]["addresses_scanned"], json!(9));
    // Both funded addresses were discovered through whichever provider
    // served them.
    assert_eq!(
        partitioned_scan["job"]["holdings_detected"],
        reference_scan["job"]["holdings_detected"]
    );

    // The partitioned providers jointly covered exactly the scanned
    // indices, each address exactly once.
    let observed = partitioned.rpc.observed_sets();
    let a = observed.get("mainnet-a").cloned().unwrap_or_default();
    let b = observed.get("mainnet-b").cloned().unwrap_or_default();
    assert!(a.is_disjoint(&b));
    let union: BTreeSet<String> = a.union(&b).cloned().collect();
    let covered: BTreeSet<String> = expected[..9].iter().cloned().collect();
    assert_eq!(union, covered);

    reference.handle.abort();
    for handle in &reference.rpc_handles {
        handle.abort();
    }
    partitioned.handle.abort();
    for handle in &partitioned.rpc_handles {
        handle.abort();
    }
}

/// (c) ASSIGNMENT STABILITY: the same address maps to the same provider
/// across two scans of the same wallet and provider set.
#[tokio::test]
async fn partition_assignment_is_stable_across_scans() {
    disable_jitter();
    let rig = spawn_rig(&["mainnet-a", "mainnet-b"], BTreeSet::new()).await;
    let scan_args = json!({
        "max_index": 9,
        "gap_limit": 10,
        "partition_providers": true,
    });

    let (status, first_scan) = rig.scan(scan_args.clone()).await;
    assert_eq!(status, StatusCode::OK, "first scan: {first_scan}");
    let first = rig.rpc.observed_sets();

    rig.rpc.observed.lock().unwrap().clear();
    let (status, second_scan) = rig.scan(scan_args).await;
    assert_eq!(status, StatusCode::OK, "second scan: {second_scan}");
    let second = rig.rpc.observed_sets();

    assert_eq!(
        first, second,
        "assignment must be stable across scans for a fixed provider set"
    );
    // Sanity: the second scan re-observed the full set (this is a fresh
    // walk, not a resume), split the same way.
    assert_eq!(
        second
            .values()
            .map(BTreeSet::len)
            .sum::<usize>(),
        10
    );

    rig.handle.abort();
    for handle in &rig.rpc_handles {
        handle.abort();
    }
}

/// (d) SINGLE PROVIDER PER CHAIN: the flag is inert — the provider sees
/// every address, the job record carries no partition fields, and results
/// match a flag-off scan exactly.
#[tokio::test]
async fn single_provider_per_chain_matches_flag_off_behavior() {
    disable_jitter();
    let expected: BTreeSet<String> = derived_addresses(10).into_iter().collect();
    let funded: BTreeSet<String> = [expected.iter().nth(3).unwrap().clone()]
        .into_iter()
        .collect();

    let flag_on = spawn_rig(&["mainnet"], funded.clone()).await;
    let (status, on) = flag_on
        .scan(json!({
            "max_index": 9,
            "gap_limit": 10,
            "partition_providers": true,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "flag-on scan: {on}");
    // The single provider observed the complete address set.
    let seen = flag_on.rpc.observed_sets();
    assert_eq!(seen.get("mainnet").cloned().unwrap_or_default(), expected);
    // The job record is byte-identical in shape to a non-partitioned job:
    // no partition fields at all.
    assert!(
        on["job"].get("partition_providers").is_none(),
        "single-provider job must not note partitioning: {on}"
    );
    assert!(
        on["job"].get("provider_partition_observations").is_none(),
        "single-provider job must not carry partition counts: {on}"
    );

    let flag_off = spawn_rig(&["mainnet"], funded).await;
    let (status, off) = flag_off.scan(json!({ "max_index": 9, "gap_limit": 10 })).await;
    assert_eq!(status, StatusCode::OK, "flag-off scan: {off}");
    assert!(off["job"].get("partition_providers").is_none());
    assert!(off["job"].get("provider_partition_observations").is_none());

    for key in ["addresses_scanned", "active_addresses", "holdings_detected"] {
        assert_eq!(
            on["job"][key], off["job"][key],
            "job counter {key} must match flag-off"
        );
    }
    assert_eq!(per_chain_view(&on), per_chain_view(&off));
    assert_eq!(scanned_indices(&on), scanned_indices(&off));

    flag_on.handle.abort();
    for handle in &flag_on.rpc_handles {
        handle.abort();
    }
    flag_off.handle.abort();
    for handle in &flag_off.rpc_handles {
        handle.abort();
    }
}

/// (e) PARTITIONED + ASYNC: cancel mid-run, resume, and the two scans
/// together observe every index exactly once — zero duplicate observations,
/// disjoint per-provider coverage intact end to end.
#[tokio::test]
async fn partitioned_async_scan_cancel_and_resume_preserves_disjoint_coverage() {
    disable_jitter();
    let expected: BTreeSet<String> = derived_addresses(10).into_iter().collect();
    let rig = spawn_rig(&["mainnet-a", "mainnet-b"], BTreeSet::new()).await;
    // Park the scan inside the third GLOBAL balance call (index 2's probe;
    // one balance call per index under partitioning).
    rig.rpc.gate_at_balance_call.store(3, Ordering::SeqCst);

    let (status, scan) = rig
        .scan(json!({
            "max_index": 9,
            "gap_limit": 10,
            "partition_providers": true,
            "run_async": true,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "scan response: {scan}");
    assert_eq!(scan["job"]["status"], "running");
    assert_eq!(scan["job"]["partition_providers"], json!(true));
    let job_id = scan["job"]["id"].as_str().unwrap().to_string();
    let operation_id = scan["operation"]["id"].as_str().unwrap().to_string();

    rig.wait_for_gate().await;
    let operation = rig.get(&format!("/api/operations/{operation_id}")).await;
    assert_eq!(operation["operation"]["progress"]["processed"], json!(2));

    let (status, cancel) = rig
        .post("/api/discovery/jobs/cancel", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "cancel response: {cancel}");
    rig.release_gate();
    rig.wait_for_operation(&operation_id, "canceled").await;

    let job = rig.discovery_job(&job_id).await;
    assert_eq!(job["status"], "canceled");
    assert_eq!(job["addresses_scanned"], json!(3));
    assert_eq!(job["partition_providers"], json!(true));
    // Both providers hold a checkpoint at the next unprocessed index.
    for provider in ["mainnet-a", "mainnet-b"] {
        let checkpoint = job["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["provider_profile"] == provider)
            .unwrap_or_else(|| panic!("{provider} checkpoint present: {job}"));
        assert_eq!(checkpoint["next_index"], json!(3));
    }

    // Resume replays the partitioning: the remaining seven indices are
    // observed exactly once each, by their originally-assigned providers.
    let (status, resume) = rig
        .post("/api/discovery/jobs/resume", json!({ "id": job_id }))
        .await;
    assert_eq!(status, StatusCode::OK, "resume response: {resume}");
    let resume_job_id = resume["job"]["id"].as_str().unwrap().to_string();
    let resume_operation_id = resume["operation"]["id"].as_str().unwrap().to_string();
    rig.wait_for_operation(&resume_operation_id, "completed").await;

    let resumed = rig.discovery_job(&resume_job_id).await;
    assert_eq!(resumed["status"], "completed");
    assert_eq!(resumed["partition_providers"], json!(true));
    assert_eq!(
        resumed["addresses_scanned"],
        json!(7),
        "resumed job must only scan the missing indices: {resumed}"
    );
    let resumed_counts = resumed["provider_partition_observations"]
        .as_array()
        .expect("resumed partitioned job carries per-provider counts");
    let counted: usize = resumed_counts
        .iter()
        .map(|entry| entry["addresses_observed"].as_u64().unwrap() as usize)
        .sum();
    assert_eq!(counted, 7);

    // Zero duplicate observations: the providers saw ten balance calls in
    // total, no address was served twice by anyone, coverage is disjoint,
    // and the union is the full address set.
    assert_eq!(
        rig.balance_calls(),
        10,
        "providers must see no re-scanned indices"
    );
    let logs = rig.rpc.observed.lock().unwrap();
    let mut union = BTreeSet::new();
    for (name, addresses) in logs.iter() {
        let unique: BTreeSet<_> = addresses.iter().collect();
        assert_eq!(
            unique.len(),
            addresses.len(),
            "provider {name} re-observed an address"
        );
        for address in addresses {
            assert!(
                union.insert(address.clone()),
                "address {address} observed by two providers"
            );
        }
    }
    assert_eq!(union, expected);

    // Persisted inventory holds exactly one row per address.
    let inventory = rig.get("/api/inventory/wallets").await;
    assert_eq!(
        inventory["addresses"].as_array().unwrap().len(),
        10,
        "no duplicate address rows: {inventory}"
    );

    rig.handle.abort();
    for handle in &rig.rpc_handles {
        handle.abort();
    }
}
