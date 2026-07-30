//! HTTP route definitions and response helpers for the Sigillum daemon API.
//!
//! Each sub-module contains thin route handlers that extract Axum state and
//! headers, validate the request body, and delegate to [`crate::service::SigillumService`].
//! Route handlers never contain business logic — they own only the HTTP ↔ service
//! translation layer.
//!
//! ## Security
//!
//! Every response passes through [`sec_headers`] which injects CSP, X-Frame-Options,
//! X-Content-Type-Options, and Cache-Control headers. The embedded UI gets a
//! per-request CSP nonce via [`serve_ui`], and the zero-build UI routes
//! interactions through delegated `data-action="..."` handlers so the UI CSP
//! can keep script attributes disabled.
//!
//! ## Router decomposition
//!
//! Routes are grouped by domain into builder functions (`lifecycle_routes`,
//! `secret_routes`, `wallet_routes`, etc.) and merged into a single [`api_router`].
//! This keeps each group self-contained while allowing the top-level router to
//! compose them declaratively.

mod audit;
mod backup;
mod biometric;
mod compartments;
mod deposits;
mod diagnostics;
mod events;
mod evm;
mod fido2;
mod generate;
mod inventory;
mod lifecycle;
mod list_query;
mod maintenance;
mod operations;
mod profiles;
mod queue;
mod secrets;
mod transit;
mod wallets;

use std::sync::Arc;

use axum::extract::{FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use sigillum_api::Validate;
use sigillum_api::response::ErrorResponse;
use sigillum_api::route_paths;

use crate::AppState;
use crate::service::ServiceResult;

type AppRouter = Router<Arc<AppState>>;

// ── CSP + Security headers ──────────────────────────────────────

/// Base CSP for API responses (no inline scripts needed).
const API_CSP: &str = "default-src 'none'; \
    frame-ancestors 'none'; \
    base-uri 'self'; \
    form-action 'self'";

pub(crate) const BACKGROUND_REQUEST_HEADER: &str = "x-sigillum-background";

fn common_security_headers(h: &mut axum::http::header::HeaderMap) {
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "cache-control",
        HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
}

pub(crate) fn sec_headers(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    common_security_headers(h);
    h.insert("content-security-policy", HeaderValue::from_static(API_CSP));
    resp
}

pub(crate) fn err(status: StatusCode, code: &'static str, msg: &str) -> Response {
    sec_headers(
        (
            status,
            Json(ErrorResponse {
                code: code.to_string(),
                error: msg.to_string(),
                action: None,
                fields: None,
            }),
        )
            .into_response(),
    )
}

/// 400 `validation_failed` envelope carrying the per-field breakdown when
/// the DTO reported one.
pub(crate) fn err_validation(
    msg: &str,
    fields: Vec<sigillum_api::response::FieldError>,
) -> Response {
    sec_headers(
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: sigillum_api::error_codes::VALIDATION_FAILED.to_string(),
                error: msg.to_string(),
                action: None,
                fields: if fields.is_empty() {
                    None
                } else {
                    Some(fields)
                },
            }),
        )
            .into_response(),
    )
}

pub(crate) fn ok_json(val: serde_json::Value) -> Response {
    sec_headers(Json(val).into_response())
}

pub(crate) fn service_response<T>(result: ServiceResult<T>) -> Response
where
    T: Serialize,
{
    match result {
        Ok(payload) => ok_json(json!(payload)),
        Err(error) => sec_headers(
            (
                error.status(),
                Json(ErrorResponse {
                    code: error.code().to_string(),
                    error: error.message().to_string(),
                    action: error.action().map(str::to_owned),
                    fields: None,
                }),
            )
                .into_response(),
        ),
    }
}

/// Extract the bearer token from the Authorization header.
/// Returns `Some(token)` if present and valid, or `None` if missing/invalid.
pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned)
}

/// Extract a JSON request body and validate it against the API contract.
pub(crate) struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let body = Json::<T>::from_request(req, state)
            .await
            .map_err(|rejection| rejection.into_response())?;
        validated(body).map(Self)
    }
}

/// ## Route Handler Pattern
///
/// All route handlers in this module follow a consistent, composable pattern for
/// invoking service layer operations and converting results to HTTP responses.
/// This ensures uniform error handling, consistent authentication validation, and
/// proper security header injection across the entire API surface.
///
/// The canonical pattern is:
///
/// 1. **Extract shared state** from Axum's `State` extractor to access the application
///    context (compartments, vault registry, audit log, session manager, etc.)
///
/// 2. **Extract the bearer token** from the `Authorization` header using `bearer_token(&headers)`
///
/// 3. **Extract, validate, and destructure** the request body (if any) using the
///    `ValidatedJson` extractor so the request conforms to the API contract before proceeding
///
/// 4. **Delegate to the service layer** via `SigillumService::new(state).method(token, ...)`
///    The service layer handles:
///    - Authentication/authorization via `require_session()`
///    - Vault access control and state validation
///    - Domain logic (crypto, compartment operations, etc.)
///    - Audit logging
///    - All error conversion via `From<VaultError>` and `From<std::io::Error>` impls
///
/// 5. **Map the ServiceResult** into an HTTP Response using `service_response()` which:
///    - Converts `Ok(T)` to 200 with JSON payload
///    - Converts `Err(ServiceError)` to the mapped HTTP status code with error details
///    - Injects security headers (CSP, X-Frame-Options, Cache-Control, etc.)
///
/// This pattern eliminates boilerplate while maintaining security and error handling
/// consistency. Service methods should never call `service_response()` directly;
/// route handlers always own the response wrapping.
/// Validate a request body and extract it, or return a 400 error Response.
#[allow(clippy::result_large_err)]
pub(crate) fn validated<T: Validate>(body: Json<T>) -> Result<T, Response> {
    body.0.validate_fields().map_err(|failure| {
        let message = failure.message().to_string();
        err_validation(&message, failure.into_fields())
    })?;
    Ok(body.0)
}

// ── Router ──────────────────────────────────────────────────────

/// Serve the embedded UI with a per-response CSP nonce.
///
/// Generates a fresh random nonce for each page load, renders the embedded UI
/// with a matching `<script nonce="...">` tag, and sets the UI CSP header
/// without allowing inline script attributes.
async fn serve_ui() -> Response {
    use rand::RngCore;
    use rand::rngs::OsRng;

    let mut nonce_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);

    let html = crate::ui::render_index_html(&nonce);

    let csp = format!(
        "default-src 'self'; \
         script-src 'nonce-{nonce}'; \
         style-src 'unsafe-inline'; \
         connect-src 'self'; \
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'none'; \
         form-action 'self'"
    );

    let mut resp = Html(html).into_response();
    let h = resp.headers_mut();
    common_security_headers(h);
    if let Ok(csp_value) = HeaderValue::from_str(&csp) {
        h.insert("content-security-policy", csp_value);
    }
    resp
}

pub fn api_router() -> AppRouter {
    Router::new()
        .route(route_paths::API_HEALTH, get(lifecycle::get_health))
        .route(route_paths::UI_ROOT, get(serve_ui))
        .merge(api_routes())
}

pub(crate) async fn startup_gate(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if req.uri().path() == route_paths::API_HEALTH || state.startup_ready() {
        return next.run(req).await;
    }
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        sigillum_api::error_codes::UNAVAILABLE,
        "Startup recovery is not ready.",
    )
}

/// Touch session activity only for successful, user-initiated HTTP requests.
/// Background polling authenticates normally but cannot keep the vault open.
pub(crate) async fn activity_touch(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let token = bearer_token(req.headers());
    let is_background = req
        .headers()
        .get(BACKGROUND_REQUEST_HEADER)
        .is_some_and(|value| value.as_bytes() == b"1");
    let response = next.run(req).await;
    if !is_background && response.status().is_success() {
        if let Some(token) = token {
            state.touch_session_activity(&token);
        }
    }
    response
}

fn api_routes() -> AppRouter {
    Router::new()
        .merge(lifecycle_routes())
        .merge(system_routes())
        .merge(compartment_routes())
        .merge(secret_routes())
        .merge(generate_routes())
        .merge(transit_routes())
        .merge(evm_routes())
        .merge(profile_routes())
        .merge(wallet_routes())
        .merge(inventory_routes())
        .merge(operation_routes())
        .merge(deposit_routes())
        .merge(queue_routes())
        .merge(fido2_routes())
        .merge(biometric_routes())
}

fn lifecycle_routes() -> AppRouter {
    Router::new()
        .route("/api/status", get(lifecycle::get_status))
        .route("/api/events", get(events::get_events))
        .route("/api/unlock", post(lifecycle::post_unlock))
        .route("/api/lock", post(lifecycle::post_lock))
        .route(
            route_paths::API_SESSION_REVOKE,
            post(lifecycle::post_revoke_session),
        )
        .route(
            route_paths::API_AUTH_CAPABILITY,
            post(lifecycle::post_capability_session),
        )
}

fn biometric_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_BIOMETRIC_CHALLENGE,
            post(biometric::biometric_challenge),
        )
        .route(
            route_paths::API_BIOMETRIC_UNLOCK,
            post(biometric::biometric_unlock),
        )
        .route(
            route_paths::API_BIOMETRIC_ENROLL,
            post(biometric::biometric_enroll),
        )
}

fn system_routes() -> AppRouter {
    Router::new()
        .route(route_paths::API_DIAGNOSTICS, get(diagnostics::diagnostics))
        .route(
            route_paths::API_SELFCHECK_RUN,
            post(diagnostics::selfcheck_run),
        )
        .route(
            route_paths::API_MAINTENANCE_RUN,
            post(maintenance::run_maintenance),
        )
        .route(route_paths::API_AUDIT, get(audit::audit_recent))
        .route(route_paths::API_AUDIT_VERIFY, get(audit::audit_verify))
        .route(route_paths::API_AUDIT_RUN, post(audit::audit_run))
        .route(route_paths::API_SETUP_RESET, post(backup::setup_reset))
        .route(route_paths::API_BACKUP_EXPORT, post(backup::backup_export))
        .route(
            route_paths::API_BACKUP_RESTORE,
            post(backup::backup_restore),
        )
}

fn operation_routes() -> AppRouter {
    Router::new()
        .route("/api/operations", get(operations::list_operations))
        .route("/api/operations/{id}", get(operations::get_operation))
        .route(
            "/api/operations/{id}/cancel",
            post(operations::cancel_operation),
        )
}

fn compartment_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_COMPARTMENT_LIST,
            get(compartments::compartment_list),
        )
        .route(
            route_paths::API_COMPARTMENT_ADD,
            post(compartments::compartment_add),
        )
        .route(
            route_paths::API_COMPARTMENT_REMOVE,
            post(compartments::compartment_remove),
        )
        .route(
            route_paths::API_COMPARTMENT_INIT,
            post(compartments::compartment_init),
        )
        .route(
            route_paths::API_COMPARTMENT_SWITCH,
            post(compartments::compartment_switch),
        )
}

fn secret_routes() -> AppRouter {
    Router::new()
        .route(route_paths::API_API_KEYS, get(secrets::list_api_keys))
        .route(route_paths::API_API_KEYS_GET, post(secrets::get_api_key))
        .route(route_paths::API_API_KEYS_SET, post(secrets::set_api_key))
        .route(
            route_paths::API_API_KEYS_DELETE,
            post(secrets::delete_api_key),
        )
        .route(route_paths::API_SECRETS, get(secrets::list_secrets))
        .route(route_paths::API_SECRETS_GET, post(secrets::get_secret))
        .route(route_paths::API_SECRETS_SET, post(secrets::set_secret))
        .route(
            route_paths::API_SECRETS_DELETE,
            post(secrets::delete_secret),
        )
        .route(
            route_paths::API_SECRETS_RESOLVE_BATCH,
            post(secrets::resolve_batch),
        )
        .route(route_paths::API_SECRETS_PUSH, post(secrets::secrets_push))
}

fn generate_routes() -> AppRouter {
    Router::new().route(
        route_paths::API_GENERATE_STORE,
        post(generate::generate_store),
    )
}

fn transit_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_TRANSIT_ENCRYPT,
            post(transit::transit_encrypt),
        )
        .route(
            route_paths::API_TRANSIT_DECRYPT,
            post(transit::transit_decrypt),
        )
        .route(route_paths::API_TRANSIT_HMAC, post(transit::transit_hmac))
}

fn evm_routes() -> AppRouter {
    Router::new()
        .route(route_paths::API_EVM_NONCE, post(evm::evm_nonce))
        .route(route_paths::API_EVM_BALANCE, post(evm::evm_balance))
        .route(
            route_paths::API_EVM_ERC20_BALANCE,
            post(evm::evm_erc20_balance),
        )
        .route(route_paths::API_EVM_BROADCAST, post(evm::evm_broadcast))
        .route(
            route_paths::API_EVM_FEES_ESTIMATE,
            post(evm::evm_estimate_fees),
        )
}

fn profile_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_PROFILES_EVM,
            get(profiles::evm_provider_profiles_list),
        )
        .route(
            route_paths::API_PROFILES_EVM_UPSERT,
            post(profiles::evm_provider_profiles_upsert),
        )
        .route(
            route_paths::API_PROFILES_EVM_DELETE,
            post(profiles::evm_provider_profiles_delete),
        )
        .route(
            route_paths::API_PROFILES_ETH_STEALTH,
            get(profiles::eth_stealth_wallet_profiles_list),
        )
        .route(
            route_paths::API_PROFILES_ETH_STEALTH_UPSERT,
            post(profiles::eth_stealth_wallet_profiles_upsert),
        )
        .route(
            route_paths::API_PROFILES_ETH_STEALTH_DELETE,
            post(profiles::eth_stealth_wallet_profiles_delete),
        )
        .route(
            route_paths::API_PROFILES_ETH_XPUB,
            get(profiles::eth_xpub_wallet_profiles_list),
        )
        .route(
            route_paths::API_PROFILES_ETH_XPUB_UPSERT,
            post(profiles::eth_xpub_wallet_profiles_upsert),
        )
        .route(
            route_paths::API_PROFILES_ETH_XPUB_DELETE,
            post(profiles::eth_xpub_wallet_profiles_delete),
        )
        .route(
            route_paths::API_PROFILES_ETH_SEED,
            get(profiles::eth_seed_wallet_profiles_list),
        )
        .route(
            route_paths::API_PROFILES_ETH_SEED_UPSERT,
            post(profiles::eth_seed_wallet_profiles_upsert),
        )
        .route(
            route_paths::API_PROFILES_ETH_SEED_CREATE,
            post(profiles::eth_seed_wallet_profiles_create),
        )
        .route(
            route_paths::API_PROFILES_ETH_SEED_DELETE,
            post(profiles::eth_seed_wallet_profiles_delete),
        )
}

fn wallet_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_WALLETS_ETH_XPUB_EXPORT,
            post(wallets::eth_xpub_export),
        )
        .route(
            route_paths::API_WALLETS_ETH_XPUB_DERIVE,
            post(wallets::eth_xpub_derive),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_EXPORT,
            post(wallets::eth_stealth_export),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_GENERATE,
            post(wallets::eth_stealth_generate),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_CHECK,
            post(wallets::eth_stealth_check),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SIGN,
            post(wallets::eth_stealth_sign),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SIGN_TRANSFER,
            post(wallets::eth_stealth_sign_transfer),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SIGN_ERC20_TRANSFER,
            post(wallets::eth_stealth_sign_erc20_transfer),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SEND_TRANSFER,
            post(evm::eth_stealth_send_transfer),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SEND_ERC20_TRANSFER,
            post(evm::eth_stealth_send_erc20_transfer),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SEND_WITH_PROFILE,
            post(profiles::eth_stealth_send_with_profile),
        )
        .route(
            route_paths::API_WALLETS_ETH_STEALTH_SEND_ERC20_WITH_PROFILE,
            post(profiles::eth_stealth_send_erc20_with_profile),
        )
}

fn inventory_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_INVENTORY_WALLETS,
            get(inventory::list_wallet_inventory),
        )
        // Chain registry. CANONICAL: `/api/chains*` (used by the client crate,
        // CLI, and console). The `/api/inventory/chains*` trio below is a
        // legacy alias kept for compatibility — deprecated, scheduled for
        // removal at the next major version (docs/stability.md).
        .route(route_paths::API_CHAINS, get(inventory::list_chain_profiles))
        .route(
            route_paths::API_CHAINS_UPSERT,
            post(inventory::upsert_chain_profile),
        )
        .route(
            route_paths::API_CHAINS_DELETE,
            post(inventory::delete_chain_profile),
        )
        .route(
            route_paths::API_INVENTORY_CHAINS,
            get(inventory::list_chain_profiles),
        )
        .route(
            route_paths::API_INVENTORY_CHAINS_UPSERT,
            post(inventory::upsert_chain_profile),
        )
        .route(
            route_paths::API_INVENTORY_CHAINS_DELETE,
            post(inventory::delete_chain_profile),
        )
        .route(
            route_paths::API_INVENTORY_SCAN_EVM,
            post(inventory::scan_wallet_inventory_evm),
        )
        .route(
            "/api/inventory/addresses/delete",
            post(inventory::delete_wallet_inventory_addresses),
        )
        .route(
            "/api/inventory/nft-metadata/opt-ins",
            get(inventory::list_nft_metadata_optins),
        )
        .route(
            route_paths::API_INVENTORY_NFT_METADATA_OPT_INS_UPSERT,
            post(inventory::upsert_nft_metadata_optin),
        )
        .route(
            route_paths::API_INVENTORY_NFT_METADATA_OPT_INS_DELETE,
            post(inventory::delete_nft_metadata_optin),
        )
        .route(
            route_paths::API_INVENTORY_NFT_METADATA_SETTINGS,
            post(inventory::update_nft_metadata_settings),
        )
        .route(
            route_paths::API_INVENTORY_NFT_METADATA_FETCH,
            post(inventory::fetch_nft_metadata),
        )
        .route(
            route_paths::API_INVENTORY_WATCH_ADDRESSES,
            get(inventory::list_watch_address_book),
        )
        .route(
            route_paths::API_INVENTORY_WATCH_ADDRESSES_UPSERT,
            post(inventory::upsert_watch_address_book_entry),
        )
        .route(
            route_paths::API_INVENTORY_WATCH_ADDRESSES_DELETE,
            post(inventory::delete_watch_address_book_entry),
        )
        .route(
            route_paths::API_INVENTORY_TOKEN_REGISTRY,
            get(inventory::list_token_registry),
        )
        .route(
            route_paths::API_INVENTORY_TOKEN_REGISTRY_IMPORT,
            post(inventory::import_token_registry),
        )
        .route(
            route_paths::API_INVENTORY_TOKEN_REGISTRY_DELETE,
            post(inventory::delete_token_registry_list),
        )
        .route(
            route_paths::API_DISCOVERY_JOBS,
            get(inventory::list_discovery_jobs),
        )
        .route(
            route_paths::API_DISCOVERY_JOBS_CANCEL,
            post(inventory::cancel_discovery_job),
        )
        .route(
            route_paths::API_DISCOVERY_JOBS_RESUME,
            post(inventory::resume_discovery_job),
        )
        .route(
            route_paths::API_RISK_FINDINGS,
            get(inventory::list_risk_findings),
        )
        .route(
            route_paths::API_RISK_CATALOG,
            get(inventory::list_risk_catalog),
        )
        .route(
            route_paths::API_RISK_CATALOG_UPSERT,
            post(inventory::upsert_risk_catalog_entry),
        )
        .route(
            route_paths::API_RISK_CATALOG_DELETE,
            post(inventory::delete_risk_catalog_entry),
        )
        .route(
            route_paths::API_PLANS_CONSOLIDATION,
            get(inventory::list_consolidation_plans),
        )
        .route(
            route_paths::API_PLANS_CONSOLIDATION_GENERATE,
            post(inventory::generate_consolidation_plan),
        )
        .route(
            route_paths::API_PLANS_CONSOLIDATION_APPROVE,
            post(inventory::approve_consolidation_plan),
        )
        .route(
            route_paths::API_PLANS_CONSOLIDATION_SIMULATE,
            post(inventory::simulate_consolidation_plan),
        )
        .route(
            route_paths::API_PLANS_CONSOLIDATION_EXPORT,
            post(inventory::export_consolidation_plan),
        )
        .route(
            route_paths::API_PLANS_ENQUEUE_STEP,
            post(inventory::enqueue_consolidation_plan_step),
        )
        .route(
            route_paths::API_PLANS_ENQUEUE_PLAN,
            post(inventory::enqueue_consolidation_plan),
        )
        .route(
            route_paths::API_TREASURY_OVERVIEW,
            get(inventory::treasury_overview),
        )
        .route(
            route_paths::API_RECEIVING_OVERVIEW,
            get(inventory::receiving_overview),
        )
        .route(
            route_paths::API_RECEIVING_REFRESH_BALANCES,
            post(inventory::refresh_receiving_balances),
        )
        .route(
            route_paths::API_RECEIVING_DEPOSITS_TAG,
            post(deposits::tag_eth_stealth_deposit),
        )
        .route(
            route_paths::API_TREASURY_POLICY,
            get(inventory::get_treasury_policy),
        )
        .route(
            route_paths::API_TREASURY_POLICY_UPDATE,
            post(inventory::update_treasury_policy),
        )
        .route(
            route_paths::API_TREASURY_RECEIVE_ADDRESSES,
            get(inventory::list_treasury_receive_allocations),
        )
        .route(
            route_paths::API_TREASURY_RECEIVE_ADDRESSES_ALLOCATE,
            post(inventory::allocate_treasury_receive_address),
        )
        .route(
            route_paths::API_TREASURY_RECEIVE_ADDRESSES_ROTATE,
            post(inventory::rotate_treasury_receive_address),
        )
        .route(
            "/api/treasury/receive-addresses/purge",
            post(inventory::purge_treasury_receive_address),
        )
        .route(
            "/api/treasury/parties",
            get(inventory::list_treasury_parties).post(inventory::create_treasury_party),
        )
        .route(
            route_paths::API_TREASURY_PARTIES_UPDATE,
            post(inventory::update_treasury_party),
        )
        .route(
            route_paths::API_TREASURY_PARTIES_DELETE,
            post(inventory::delete_treasury_party),
        )
}

fn deposit_routes() -> AppRouter {
    Router::new()
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH,
            get(deposits::list_eth_stealth_deposits),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_CREATE_NATIVE,
            post(deposits::create_eth_stealth_native_deposit),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_CREATE_ERC20,
            post(deposits::create_eth_stealth_erc20_deposit),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_SCAN_ANNOUNCEMENTS,
            post(deposits::scan_eth_stealth_announcements),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_DELETE,
            post(deposits::delete_eth_stealth_deposit),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_REFRESH,
            post(deposits::refresh_eth_stealth_deposits),
        )
        .route(
            route_paths::API_DEPOSITS_ETH_STEALTH_ENQUEUE_SWEEP,
            post(deposits::enqueue_eth_stealth_deposit_sweep),
        )
}

fn queue_routes() -> AppRouter {
    Router::new()
        .route(route_paths::API_QUEUE_JOBS, get(queue::list_jobs))
        .route(
            route_paths::API_QUEUE_ENQUEUE_ETH_STEALTH_TRANSFER,
            post(queue::enqueue_eth_stealth_transfer),
        )
        .route(
            route_paths::API_QUEUE_ENQUEUE_ETH_STEALTH_ERC20_TRANSFER,
            post(queue::enqueue_eth_stealth_erc20_transfer),
        )
        .route(
            route_paths::API_QUEUE_ENQUEUE_ETH_STEALTH_NATIVE_SWEEP,
            post(queue::enqueue_eth_stealth_native_sweep),
        )
        .route(
            route_paths::API_QUEUE_ENQUEUE_ETH_STEALTH_ERC20_SWEEP,
            post(queue::enqueue_eth_stealth_erc20_sweep),
        )
        .route(route_paths::API_QUEUE_PAUSE, post(queue::pause_execution))
        .route(route_paths::API_QUEUE_RESUME, post(queue::resume_execution))
        .route(route_paths::API_QUEUE_PROCESS, post(queue::process_jobs))
}

fn fido2_routes() -> AppRouter {
    Router::new()
        .route(route_paths::API_FIDO2_STATUS, get(fido2::fido2_status))
        .route(route_paths::API_FIDO2_DETECT, get(fido2::fido2_detect))
        .route(route_paths::API_FIDO2_PIN_SET, post(fido2::fido2_set_pin))
        .route(route_paths::API_FIDO2_LIST, get(fido2::fido2_list))
        .route(route_paths::API_FIDO2_SETUP, post(fido2::fido2_setup))
        .route(route_paths::API_FIDO2_REGISTER, post(fido2::fido2_register))
        .route(route_paths::API_FIDO2_UNLOCK, post(fido2::fido2_unlock))
        .route(route_paths::API_FIDO2_REMOVE, post(fido2::fido2_remove))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use sigillum_fido2::config::CompartmentMeta;
    use tempfile::TempDir;
    use tower::util::ServiceExt;

    use super::{BACKGROUND_REQUEST_HEADER, activity_touch, bearer_token, serve_ui};
    use crate::AppState;

    #[tokio::test]
    async fn serve_ui_csp_matches_delegated_handlers_present_in_html() {
        let resp = serve_ui().await;
        let csp = resp
            .headers()
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .expect("ui response should include CSP")
            .to_string();

        let body = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("ui body should be readable");
        let html = String::from_utf8(body.to_vec()).expect("ui body should be utf-8");

        assert!(html.contains("data-action=\"wizPreset\" data-arg0=\"secure\""));
        assert!(html.contains("data-action=\"wizSetNewPin\""));
        assert!(html.contains("data-action=\"wizRegisterAdditionalKey\""));
        assert!(html.contains("data-action=\"wizSetAdditionalKeyPin\""));
        assert!(html.contains("data-action=\"fido2SetNewPin\""));
        assert!(html.contains("data-action=\"restoreSetupSnapshot\""));
        // The typed-confirmation dialog collects the reset phrase; the HTML
        // only carries the three delegating buttons.
        assert_eq!(html.matches("data-action=\"resetLocalData\"").count(), 3);
        assert!(html.contains("data-action=\"restoreAuthSnapshot\""));
        assert!(html.contains("data-action=\"togglePoisonWarning\""));
        assert!(csp.contains("script-src 'nonce-"));
        assert!(!csp.contains("script-src-attr 'unsafe-inline'"));
    }

    async fn authenticated_ok(
        State(state): State<Arc<AppState>>,
        headers: HeaderMap,
    ) -> StatusCode {
        if bearer_token(&headers)
            .as_deref()
            .is_some_and(|token| state.verify_token(token))
        {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    }

    #[tokio::test]
    async fn authenticated_background_poll_does_not_touch_session_activity() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("state should initialize"));
        state.unlock_compartment(
            0,
            [1_u8; 32],
            CompartmentMeta {
                id: 0,
                label: "daily".into(),
                threshold: 1,
                passphrase_mode: None,
            },
        );
        let session = state.create_session(Some(0));
        state.backdate_session_activity(&session, Duration::from_secs(60));

        let app = Router::new()
            .fallback(get(authenticated_ok))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                activity_touch,
            ))
            .with_state(state.clone());
        let background = Request::builder()
            .uri("/")
            .header("authorization", format!("Bearer {session}"))
            .header(BACKGROUND_REQUEST_HEADER, "1")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(background).await.unwrap().status(),
            StatusCode::OK
        );
        assert!(
            state.session_activity_elapsed(&session).unwrap() >= Duration::from_secs(59),
            "successful authentication alone must not count as operator activity"
        );

        let interactive = Request::builder()
            .uri("/")
            .header("authorization", format!("Bearer {session}"))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(interactive).await.unwrap().status(),
            StatusCode::OK
        );
        assert!(
            state.session_activity_elapsed(&session).unwrap() < Duration::from_secs(1),
            "a successful interactive request must refresh operator activity"
        );
    }

    async fn error_body(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("error body should be readable");
        serde_json::from_slice(&bytes).expect("error body should be json")
    }

    #[tokio::test]
    async fn service_response_envelope_carries_code() {
        use crate::service::ServiceError;

        let resp = super::service_response::<serde_json::Value>(Err(ServiceError::vault_locked(
            "Vault is locked.",
        )));
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
        let body = error_body(resp).await;
        assert_eq!(body["code"], "vault_locked");
        assert_eq!(body["error"], "Vault is locked.");
        assert!(body.get("action").is_none());
        assert!(body.get("fields").is_none());
    }

    #[tokio::test]
    async fn service_response_envelope_carries_action_for_policy_violation() {
        use crate::service::ServiceError;

        let resp = super::service_response::<serde_json::Value>(Err(
            ServiceError::policy_violation("cross_party_linkage"),
        ));
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN);
        let body = error_body(resp).await;
        assert_eq!(body["code"], "policy_violation");
        assert_eq!(body["error"], "policy_violation");
        assert_eq!(body["action"], "cross_party_linkage");
    }

    #[tokio::test]
    async fn service_response_envelope_disambiguates_overloaded_statuses() {
        use crate::service::ServiceError;

        let cases: [(ServiceError, u16, &str); 7] = [
            (
                ServiceError::execution_gate_denied("x"),
                403,
                "execution_gate_denied",
            ),
            (
                ServiceError::capability_scope_denied("x"),
                403,
                "capability_scope_denied",
            ),
            (ServiceError::not_found("x"), 404, "not_found"),
            (ServiceError::not_initialized("x"), 404, "not_initialized"),
            (ServiceError::unlock_throttled("x"), 429, "unlock_throttled"),
            (ServiceError::too_many_requests("x"), 429, "rate_limited"),
            (ServiceError::conflict("x"), 409, "conflict"),
        ];
        for (error, status, code) in cases {
            let resp = super::service_response::<serde_json::Value>(Err(error));
            assert_eq!(resp.status().as_u16(), status, "status for {code}");
            let body = error_body(resp).await;
            assert_eq!(body["code"], code);
        }
    }
}
