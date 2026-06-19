use std::collections::HashSet;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::{OwnedMutexGuard, oneshot};
use tokio::time::sleep;

#[derive(Clone, Debug, Default)]
pub struct StubDaemonConfig {
    pub wallet_profiles: Vec<WalletProfileFixture>,
    pub provider_profiles: Vec<ProviderProfileFixture>,
    pub reject_wallet_profiles: HashSet<String>,
    pub reject_export_wallet_profiles: HashSet<String>,
}

#[derive(Clone, Debug)]
pub struct WalletProfileFixture {
    pub name: String,
    pub wallet: String,
    pub short_name: String,
    pub provider_profile: String,
    pub compartment_id: usize,
    pub chain_id: Option<u64>,
    pub default_destination_address: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderProfileFixture {
    pub name: String,
    pub rpc_url: String,
    pub auth_token_key: Option<String>,
    pub compartment_id: usize,
    pub chain_id: u64,
    pub max_priority_fee_per_gas_hex: Option<String>,
    pub max_fee_per_gas_hex: Option<String>,
    pub native_gas_limit: Option<u64>,
    pub erc20_gas_limit: Option<u64>,
}

pub struct StubDaemon {
    addr: SocketAddr,
    state: Arc<StubState>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

struct StubState {
    config: StubDaemonConfig,
    export_calls: AtomicUsize,
    generate_calls: AtomicUsize,
    native_deposit_calls: AtomicUsize,
    erc20_deposit_calls: AtomicUsize,
    delete_deposit_calls: AtomicUsize,
    deleted_deposit_ids: Mutex<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StubDaemonCounts {
    pub export_calls: usize,
    pub generate_calls: usize,
    pub native_deposit_calls: usize,
    pub erc20_deposit_calls: usize,
    pub delete_deposit_calls: usize,
}

impl StubDaemon {
    pub async fn spawn(
        config: StubDaemonConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let state = Arc::new(StubState {
            config,
            export_calls: AtomicUsize::new(0),
            generate_calls: AtomicUsize::new(0),
            native_deposit_calls: AtomicUsize::new(0),
            erc20_deposit_calls: AtomicUsize::new(0),
            delete_deposit_calls: AtomicUsize::new(0),
            deleted_deposit_ids: Mutex::new(Vec::new()),
        });

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;

        let app = Router::new()
            .route("/api/status", get(status))
            .route("/api/profiles/eth-stealth", get(list_wallet_profiles))
            .route("/api/profiles/evm", get(list_provider_profiles))
            .route("/api/wallets/eth-stealth/export", post(export_meta_address))
            .route("/api/wallets/eth-stealth/generate", post(generate_address))
            .route(
                "/api/deposits/eth-stealth/create-native",
                post(create_native_deposit),
            )
            .route(
                "/api/deposits/eth-stealth/create-erc20",
                post(create_erc20_deposit),
            )
            .route("/api/deposits/eth-stealth/delete", post(delete_deposit))
            .with_state(state.clone());

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(err) = serve.await {
                tracing::warn!("stub daemon exited with error: {err}");
            }
        });

        Ok(Self {
            addr,
            state,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn counts(&self) -> StubDaemonCounts {
        StubDaemonCounts {
            export_calls: self.state.export_calls.load(Ordering::SeqCst),
            generate_calls: self.state.generate_calls.load(Ordering::SeqCst),
            native_deposit_calls: self.state.native_deposit_calls.load(Ordering::SeqCst),
            erc20_deposit_calls: self.state.erc20_deposit_calls.load(Ordering::SeqCst),
            delete_deposit_calls: self.state.delete_deposit_calls.load(Ordering::SeqCst),
        }
    }

    pub fn deleted_deposit_ids(&self) -> Vec<String> {
        self.state
            .deleted_deposit_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for StubDaemon {
    fn drop(&mut self) {
        if let Some(tx) = self
            .shutdown_tx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
        {
            let _ = tx.send(());
        }
    }
}

pub struct GatewayHarness {
    child: Child,
    tempdir: TempDir,
    _suite_guard: OwnedMutexGuard<()>,
    pub base_url: String,
    pub db_path: PathBuf,
    pub client: reqwest::Client,
}

impl GatewayHarness {
    pub async fn spawn(
        stub_url: &str,
        admin_key: &str,
        rate_limit_rps: u64,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let suite_guard = gateway_suite_lock().clone().lock_owned().await;
        let tempdir = TempDir::new()?;
        let db_path = tempdir.path().join("gateway.db");
        let port = pick_free_port()?;
        let bind_addr = format!("127.0.0.1:{port}");
        let base_url = format!("http://{bind_addr}");

        let mut command = Command::new(env!("CARGO_BIN_EXE_sigillum-gateway"));
        command
            .env("GATEWAY_ADMIN_KEY", admin_key)
            .env("GATEWAY_BIND_ADDR", &bind_addr)
            .env(
                "GATEWAY_DATABASE_URL",
                format!("sqlite://{}?mode=rwc", db_path.display()),
            )
            .env("SIGILLUM_DAEMON_URL", stub_url)
            .env("SIGILLUM_DAEMON_SESSION_TOKEN", "stub-session-token")
            .env("GATEWAY_POLL_INTERVAL_SECS", "3600")
            .env("GATEWAY_RATE_LIMIT_RPS", rate_limit_rps.to_string())
            .env("GATEWAY_AUTH_CACHE_TTL_SECS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn()?;
        let harness = Self {
            child,
            tempdir,
            _suite_guard: suite_guard,
            base_url,
            db_path,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()?,
        };
        Ok(harness)
    }

    pub async fn wait_until_ready(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let health_url = format!("{}/api/v1/health", self.base_url);
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait()? {
                return Err(format!("gateway exited early with status {status}").into());
            }

            match self.client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => {}
            }

            sleep(Duration::from_millis(50)).await;
        }

        Err("gateway did not become ready in time".into())
    }

    pub fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    pub async fn request_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
        bearer: Option<&str>,
    ) -> reqwest::Response {
        let mut request = self.client.request(method, self.url(path)).json(&body);
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("request should succeed")
    }

    pub async fn get(&self, path: &str, bearer: Option<&str>) -> reqwest::Response {
        let mut request = self.client.get(self.url(path));
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("request should succeed")
    }

    pub async fn sqlite_row_count(
        &self,
        project_id: &str,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let connection = rusqlite::Connection::open(&self.db_path)?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM payments WHERE project_id = ?",
            rusqlite::params![project_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count)
    }

    pub async fn sqlite_payment_by_idempotency(
        &self,
        project_id: &str,
        key: &str,
    ) -> Result<Option<(String, i64)>, Box<dyn std::error::Error + Send + Sync>> {
        let connection = rusqlite::Connection::open(&self.db_path)?;
        let row = connection
            .query_row(
                "SELECT id, chain_id FROM payments WHERE project_id = ? AND idempotency_key = ?",
                rusqlite::params![project_id, key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(row)
    }

    pub async fn install_payment_insert_failure_trigger(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let connection = rusqlite::Connection::open(&self.db_path)?;
        connection.execute(
            "CREATE TRIGGER fail_payment_insert BEFORE INSERT ON payments BEGIN SELECT RAISE(FAIL, 'forced payment insert failure'); END;",
            [],
        )?;
        Ok(())
    }
}

impl Drop for GatewayHarness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.tempdir.path();
    }
}

fn pick_free_port() -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn gateway_suite_lock() -> &'static Arc<tokio::sync::Mutex<()>> {
    static LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
}

async fn status() -> Json<Value> {
    Json(json!({
        "locked": false,
        "initialized": true,
        "active_compartment": null,
        "unlocked_compartments": [],
        "fido2": null
    }))
}

async fn list_wallet_profiles(State(state): State<Arc<StubState>>) -> Json<Value> {
    let profiles = if state.config.wallet_profiles.is_empty() {
        default_wallet_profiles()
    } else {
        state
            .config
            .wallet_profiles
            .iter()
            .map(wallet_profile_json)
            .collect()
    };
    let profiles = profiles
        .into_iter()
        .filter(|profile| {
            let name = profile
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            !state.config.reject_wallet_profiles.contains(name)
        })
        .collect::<Vec<_>>();
    Json(json!({ "profiles": profiles }))
}

async fn list_provider_profiles(State(state): State<Arc<StubState>>) -> Json<Value> {
    let profiles = if state.config.provider_profiles.is_empty() {
        default_provider_profiles()
    } else {
        state
            .config
            .provider_profiles
            .iter()
            .map(provider_profile_json)
            .collect()
    };
    Json(json!({
        "profiles": profiles
    }))
}

async fn export_meta_address(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let wallet = body
        .get("wallet")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state.export_calls.fetch_add(1, Ordering::SeqCst);

    if state.config.reject_export_wallet_profiles.contains(&wallet) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "wallet profile not found" })),
        )
            .into_response();
    }

    Json(json!({
        "wallet": wallet,
        "short_name": body.get("short_name").and_then(Value::as_str).unwrap_or("eth"),
        "scheme_id": 5564,
        "stealth_meta_address": "st:meta:stub",
        "spending_public_key_hex": "11".repeat(32),
        "viewing_public_key_hex": "22".repeat(32),
    }))
    .into_response()
}

async fn generate_address(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    state.generate_calls.fetch_add(1, Ordering::SeqCst);
    Json(json!({
        "short_name": "eth",
        "scheme_id": 5564,
        "stealth_meta_address": body.get("stealth_meta_address").and_then(Value::as_str).unwrap_or("st:meta:stub"),
        "stealth_address": "st:address:stub",
        "ephemeral_public_key_hex": "33".repeat(32),
        "view_tag_hex": "aa",
    }))
    .into_response()
}

async fn create_native_deposit(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let wallet = body
        .get("wallet_profile")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state.native_deposit_calls.fetch_add(1, Ordering::SeqCst);
    if state.config.reject_export_wallet_profiles.contains(&wallet) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "wallet profile not found" })),
        )
            .into_response();
    }

    Json(json!({
        "status": "created",
        "deposit": deposit_json(&wallet, "native", "native-deposit-1"),
    }))
    .into_response()
}

async fn create_erc20_deposit(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let wallet = body
        .get("wallet_profile")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state.erc20_deposit_calls.fetch_add(1, Ordering::SeqCst);
    if state.config.reject_export_wallet_profiles.contains(&wallet) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "wallet profile not found" })),
        )
            .into_response();
    }

    Json(json!({
        "status": "created",
        "deposit": deposit_json(&wallet, "erc20", "erc20-deposit-1"),
    }))
    .into_response()
}

async fn delete_deposit(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let deposit_id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state.delete_deposit_calls.fetch_add(1, Ordering::SeqCst);
    state
        .deleted_deposit_ids
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(deposit_id.clone());

    Json(json!({
        "status": "deleted",
        "deposit": deposit_json("stub-wallet", "native", &deposit_id),
    }))
    .into_response()
}

fn deposit_json(wallet: &str, asset_kind: &str, id: &str) -> Value {
    json!({
        "id": id,
        "status": "pending",
        "asset_kind": asset_kind,
        "wallet_profile": wallet,
        "wallet_compartment_id": 1,
        "provider_compartment_id": 2,
        "wallet": "stub-wallet",
        "short_name": "eth",
        "stealth_meta_address": "st:meta:stub",
        "stealth_address": "st:address:stub",
        "ephemeral_public_key_hex": "33".repeat(32),
        "view_tag_hex": "aa",
        "token_address": null,
        "expected_amount_hex": "0x1",
        "observed_amount_hex": null,
        "observed_native_balance_wei_hex": null,
        "auto_queue_sweep": true,
        "sweep_destination_address": null,
        "min_sweep_amount_hex": null,
        "queue_job_id": null,
        "queue_job_state": null,
        "note": "gateway-payment",
        "created_at_unix": 1,
        "updated_at_unix": 1,
        "last_checked_at_unix": null,
        "broadcast_transaction_hash_hex": null
    })
}

fn default_wallet_profiles() -> Vec<Value> {
    vec![wallet_profile_json(&WalletProfileFixture {
        name: "payments-mainnet".into(),
        wallet: "stub-wallet".into(),
        short_name: "eth".into(),
        provider_profile: "provider-mainnet".into(),
        compartment_id: 1,
        chain_id: Some(1),
        default_destination_address: None,
    })]
}

fn default_provider_profiles() -> Vec<Value> {
    vec![provider_profile_json(&ProviderProfileFixture {
        name: "provider-mainnet".into(),
        rpc_url: "https://rpc.example.invalid".into(),
        auth_token_key: None,
        compartment_id: 2,
        chain_id: 1,
        max_priority_fee_per_gas_hex: None,
        max_fee_per_gas_hex: None,
        native_gas_limit: None,
        erc20_gas_limit: None,
    })]
}

fn wallet_profile_json(profile: &WalletProfileFixture) -> Value {
    json!({
        "name": profile.name,
        "wallet": profile.wallet,
        "short_name": profile.short_name,
        "provider_profile": profile.provider_profile,
        "compartment_id": profile.compartment_id,
        "chain_id": profile.chain_id,
        "default_destination_address": profile.default_destination_address,
    })
}

fn provider_profile_json(profile: &ProviderProfileFixture) -> Value {
    json!({
        "name": profile.name,
        "rpc_url": profile.rpc_url,
        "auth_token_key": profile.auth_token_key,
        "compartment_id": profile.compartment_id,
        "chain_id": profile.chain_id,
        "max_priority_fee_per_gas_hex": profile.max_priority_fee_per_gas_hex,
        "max_fee_per_gas_hex": profile.max_fee_per_gas_hex,
        "native_gas_limit": profile.native_gas_limit,
        "erc20_gas_limit": profile.erc20_gas_limit,
    })
}
