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
mod evm;
mod fido2;
mod lifecycle;
mod maintenance;
mod profiles;
mod queue;
mod secrets;
mod transit;
mod wallets;

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;
use sigillum_api::Validate;
use sigillum_api::response::ErrorResponse;

use crate::AppState;
use crate::service::ServiceResult;

type AppRouter = Router<Arc<AppState>>;

// ── CSP + Security headers ──────────────────────────────────────

/// Base CSP for API responses (no inline scripts needed).
const API_CSP: &str = "default-src 'none'; \
    frame-ancestors 'none'; \
    base-uri 'self'; \
    form-action 'self'";

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

pub(crate) fn err(status: StatusCode, msg: &str) -> Response {
    sec_headers(
        (
            status,
            Json(ErrorResponse {
                error: msg.to_string(),
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
        Err(error) => err(error.status(), error.message()),
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
/// 3. **Validate and destructure** the request body (if any) using `validated()` to ensure
///    the request conforms to the API contract before proceeding
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
    body.0
        .validate()
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
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
    Router::new().route("/", get(serve_ui)).merge(api_routes())
}

fn api_routes() -> AppRouter {
    Router::new()
        .merge(lifecycle_routes())
        .merge(system_routes())
        .merge(compartment_routes())
        .merge(secret_routes())
        .merge(transit_routes())
        .merge(evm_routes())
        .merge(profile_routes())
        .merge(wallet_routes())
        .merge(deposit_routes())
        .merge(queue_routes())
        .merge(fido2_routes())
        .merge(biometric_routes())
}

fn lifecycle_routes() -> AppRouter {
    Router::new()
        .route("/api/status", get(lifecycle::get_status))
        .route("/api/unlock", post(lifecycle::post_unlock))
        .route("/api/lock", post(lifecycle::post_lock))
        .route("/api/session/revoke", post(lifecycle::post_revoke_session))
}

fn biometric_routes() -> AppRouter {
    Router::new()
        .route(
            "/api/biometric/challenge",
            post(biometric::biometric_challenge),
        )
        .route("/api/biometric/unlock", post(biometric::biometric_unlock))
        .route("/api/biometric/enroll", post(biometric::biometric_enroll))
}

fn system_routes() -> AppRouter {
    Router::new()
        .route("/api/diagnostics", get(diagnostics::diagnostics))
        .route("/api/maintenance/run", post(maintenance::run_maintenance))
        .route("/api/audit", get(audit::audit_recent))
        .route("/api/audit/run", post(audit::audit_run))
        .route("/api/setup/reset", post(backup::setup_reset))
        .route("/api/backup/export", post(backup::backup_export))
        .route("/api/backup/restore", post(backup::backup_restore))
}

fn compartment_routes() -> AppRouter {
    Router::new()
        .route("/api/compartment/list", get(compartments::compartment_list))
        .route("/api/compartment/add", post(compartments::compartment_add))
        .route(
            "/api/compartment/remove",
            post(compartments::compartment_remove),
        )
        .route(
            "/api/compartment/init",
            post(compartments::compartment_init),
        )
        .route(
            "/api/compartment/switch",
            post(compartments::compartment_switch),
        )
}

fn secret_routes() -> AppRouter {
    Router::new()
        .route("/api/api-keys", get(secrets::list_api_keys))
        .route("/api/api-keys/get", post(secrets::get_api_key))
        .route("/api/api-keys/set", post(secrets::set_api_key))
        .route("/api/api-keys/delete", post(secrets::delete_api_key))
        .route("/api/secrets", get(secrets::list_secrets))
        .route("/api/secrets/get", post(secrets::get_secret))
        .route("/api/secrets/set", post(secrets::set_secret))
        .route("/api/secrets/delete", post(secrets::delete_secret))
        .route("/api/secrets/resolve-batch", post(secrets::resolve_batch))
        .route("/api/secrets/push", post(secrets::secrets_push))
}

fn transit_routes() -> AppRouter {
    Router::new()
        .route("/api/transit/encrypt", post(transit::transit_encrypt))
        .route("/api/transit/decrypt", post(transit::transit_decrypt))
        .route("/api/transit/hmac", post(transit::transit_hmac))
}

fn evm_routes() -> AppRouter {
    Router::new()
        .route("/api/evm/nonce", post(evm::evm_nonce))
        .route("/api/evm/balance", post(evm::evm_balance))
        .route("/api/evm/erc20-balance", post(evm::evm_erc20_balance))
        .route("/api/evm/broadcast", post(evm::evm_broadcast))
}

fn profile_routes() -> AppRouter {
    Router::new()
        .route(
            "/api/profiles/evm",
            get(profiles::evm_provider_profiles_list),
        )
        .route(
            "/api/profiles/evm/upsert",
            post(profiles::evm_provider_profiles_upsert),
        )
        .route(
            "/api/profiles/evm/delete",
            post(profiles::evm_provider_profiles_delete),
        )
        .route(
            "/api/profiles/eth-stealth",
            get(profiles::eth_stealth_wallet_profiles_list),
        )
        .route(
            "/api/profiles/eth-stealth/upsert",
            post(profiles::eth_stealth_wallet_profiles_upsert),
        )
        .route(
            "/api/profiles/eth-stealth/delete",
            post(profiles::eth_stealth_wallet_profiles_delete),
        )
        .route(
            "/api/profiles/eth-xpub",
            get(profiles::eth_xpub_wallet_profiles_list),
        )
        .route(
            "/api/profiles/eth-xpub/upsert",
            post(profiles::eth_xpub_wallet_profiles_upsert),
        )
        .route(
            "/api/profiles/eth-xpub/delete",
            post(profiles::eth_xpub_wallet_profiles_delete),
        )
}

fn wallet_routes() -> AppRouter {
    Router::new()
        .route(
            "/api/wallets/eth-xpub/export",
            post(wallets::eth_xpub_export),
        )
        .route(
            "/api/wallets/eth-xpub/derive",
            post(wallets::eth_xpub_derive),
        )
        .route(
            "/api/wallets/eth-stealth/export",
            post(wallets::eth_stealth_export),
        )
        .route(
            "/api/wallets/eth-stealth/generate",
            post(wallets::eth_stealth_generate),
        )
        .route(
            "/api/wallets/eth-stealth/check",
            post(wallets::eth_stealth_check),
        )
        .route(
            "/api/wallets/eth-stealth/sign",
            post(wallets::eth_stealth_sign),
        )
        .route(
            "/api/wallets/eth-stealth/sign-transfer",
            post(wallets::eth_stealth_sign_transfer),
        )
        .route(
            "/api/wallets/eth-stealth/sign-erc20-transfer",
            post(wallets::eth_stealth_sign_erc20_transfer),
        )
        .route(
            "/api/wallets/eth-stealth/send-transfer",
            post(evm::eth_stealth_send_transfer),
        )
        .route(
            "/api/wallets/eth-stealth/send-erc20-transfer",
            post(evm::eth_stealth_send_erc20_transfer),
        )
        .route(
            "/api/wallets/eth-stealth/send-with-profile",
            post(profiles::eth_stealth_send_with_profile),
        )
        .route(
            "/api/wallets/eth-stealth/send-erc20-with-profile",
            post(profiles::eth_stealth_send_erc20_with_profile),
        )
}

fn deposit_routes() -> AppRouter {
    Router::new()
        .route(
            "/api/deposits/eth-stealth",
            get(deposits::list_eth_stealth_deposits),
        )
        .route(
            "/api/deposits/eth-stealth/create-native",
            post(deposits::create_eth_stealth_native_deposit),
        )
        .route(
            "/api/deposits/eth-stealth/create-erc20",
            post(deposits::create_eth_stealth_erc20_deposit),
        )
        .route(
            "/api/deposits/eth-stealth/delete",
            post(deposits::delete_eth_stealth_deposit),
        )
        .route(
            "/api/deposits/eth-stealth/refresh",
            post(deposits::refresh_eth_stealth_deposits),
        )
        .route(
            "/api/deposits/eth-stealth/enqueue-sweep",
            post(deposits::enqueue_eth_stealth_deposit_sweep),
        )
}

fn queue_routes() -> AppRouter {
    Router::new()
        .route("/api/queue/jobs", get(queue::list_jobs))
        .route(
            "/api/queue/enqueue/eth-stealth-transfer",
            post(queue::enqueue_eth_stealth_transfer),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-erc20-transfer",
            post(queue::enqueue_eth_stealth_erc20_transfer),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-native-sweep",
            post(queue::enqueue_eth_stealth_native_sweep),
        )
        .route(
            "/api/queue/enqueue/eth-stealth-erc20-sweep",
            post(queue::enqueue_eth_stealth_erc20_sweep),
        )
        .route("/api/queue/process", post(queue::process_jobs))
}

fn fido2_routes() -> AppRouter {
    Router::new()
        .route("/api/fido2/status", get(fido2::fido2_status))
        .route("/api/fido2/detect", get(fido2::fido2_detect))
        .route("/api/fido2/pin/set", post(fido2::fido2_set_pin))
        .route("/api/fido2/list", get(fido2::fido2_list))
        .route("/api/fido2/setup", post(fido2::fido2_setup))
        .route("/api/fido2/register", post(fido2::fido2_register))
        .route("/api/fido2/unlock", post(fido2::fido2_unlock))
        .route("/api/fido2/remove", post(fido2::fido2_remove))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::serve_ui;

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
        assert!(html.contains("data-action=\"resetLocalData\" data-arg0=\"setupResetConfirm\""));
        assert!(html.contains("data-action=\"restoreAuthSnapshot\""));
        assert!(html.contains("data-action=\"resetLocalData\" data-arg0=\"authResetConfirm\""));
        assert!(html.contains("data-action=\"resetLocalData\" data-arg0=\"backupResetConfirm\""));
        assert!(html.contains("data-action=\"togglePoisonWarning\""));
        assert!(csp.contains("script-src 'nonce-"));
        assert!(!csp.contains("script-src-attr 'unsafe-inline'"));
    }
}
