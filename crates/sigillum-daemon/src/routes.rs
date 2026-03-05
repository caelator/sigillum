use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::json;

use sigillum_core::{SecretStore, VaultLifecycle};

use crate::AppState;

// ── Security headers ─────────────────────────────────────────────

fn sec_headers(mut resp: Response) -> Response {
    let h = resp.headers_mut();
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
    resp
}

// ── Request bodies ───────────────────────────────────────────────

#[derive(Deserialize)]
struct KeyValue {
    key: String,
    value: Option<String>,
}

#[derive(Deserialize)]
struct KeyOnly {
    key: String,
}

#[derive(Deserialize)]
struct PassphraseBody {
    passphrase: String,
}

// ── Router ───────────────────────────────────────────────────────

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        // Status
        .route("/api/status", get(get_status))
        // Lifecycle
        .route("/api/unlock", post(post_unlock))
        .route("/api/lock", post(post_lock))
        .route("/api/init", post(post_init))
        // Tier 1 — API keys
        .route("/api/api-keys", get(list_api_keys))
        .route("/api/api-keys/get", post(get_api_key))
        .route("/api/api-keys/set", post(set_api_key))
        .route("/api/api-keys/delete", post(delete_api_key))
        // Tier 2 — Encrypted secrets
        .route("/api/secrets", get(list_secrets))
        .route("/api/secrets/get", post(get_secret))
        .route("/api/secrets/set", post(set_secret))
        .route("/api/secrets/delete", post(delete_secret))
}

// ── Status ───────────────────────────────────────────────────────

async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    let vault = &state.vault;
    let exists = vault.vault_exists();
    let unlocked = vault.is_unlocked();
    let api_key_count = vault.list_api_keys().len();
    let secret_count = if unlocked {
        Some(vault.list_secrets().len())
    } else {
        None
    };

    sec_headers(
        Json(json!({
            "vault_exists": exists,
            "unlocked": unlocked,
            "api_key_count": api_key_count,
            "secret_count": secret_count,
        }))
        .into_response(),
    )
}

// ── Lifecycle ────────────────────────────────────────────────────

async fn post_init(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PassphraseBody>,
) -> Response {
    let vault = &state.vault;

    if vault.vault_exists() {
        return sec_headers(
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": "Vault already exists. Delete it first to reinitialize." })),
            )
                .into_response(),
        );
    }

    if body.passphrase.len() < 8 {
        return sec_headers(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Passphrase must be at least 8 characters." })),
            )
                .into_response(),
        );
    }

    let (master_key, salt) = derive_key_from_passphrase(&body.passphrase);

    if let Err(e) = vault.initialize(&master_key) {
        return sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to initialize vault: {e}") })),
            )
                .into_response(),
        );
    }

    // Save salt
    let salt_path = state.salt_path();
    if let Some(dir) = salt_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(salt_path, &salt) {
        return sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to save salt: {e}") })),
            )
                .into_response(),
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(salt_path, std::fs::Permissions::from_mode(0o600));
    }

    // Auto-unlock after init
    vault.load_master_key(master_key);

    sec_headers(
        Json(json!({
            "status": "initialized",
            "message": "Vault created and unlocked. Remember your passphrase."
        }))
        .into_response(),
    )
}

async fn post_unlock(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PassphraseBody>,
) -> Response {
    let vault = &state.vault;

    if vault.is_unlocked() {
        return sec_headers(Json(json!({ "status": "already_unlocked" })).into_response());
    }

    if !vault.vault_exists() {
        return sec_headers(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No vault found. POST /api/init first." })),
            )
                .into_response(),
        );
    }

    let salt = match std::fs::read(state.salt_path()) {
        Ok(s) if s.len() == 32 => s,
        _ => {
            return sec_headers(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Cannot read salt file. Vault may be corrupted." })),
                )
                    .into_response(),
            );
        }
    };

    let master_key = derive_key_with_salt(&body.passphrase, &salt);
    vault.load_master_key(master_key);

    if vault.verify_master_key() {
        sec_headers(Json(json!({ "status": "unlocked" })).into_response())
    } else {
        vault.zeroize_master_key();
        sec_headers(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Wrong passphrase." })),
            )
                .into_response(),
        )
    }
}

async fn post_lock(State(state): State<Arc<AppState>>) -> Response {
    state.vault.zeroize_master_key();
    sec_headers(
        Json(json!({
            "status": "locked",
            "message": "Master key zeroized."
        }))
        .into_response(),
    )
}

// ── Tier 1: API Keys ────────────────────────────────────────────

async fn list_api_keys(State(state): State<Arc<AppState>>) -> Response {
    let keys = state.vault.list_api_keys();
    sec_headers(Json(json!({ "keys": keys })).into_response())
}

async fn get_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.vault.get_api_key(&body.key) {
        Some(val) => sec_headers(
            Json(json!({ "key": body.key, "value": val.expose_secret() })).into_response(),
        ),
        None => sec_headers(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("API key '{}' not found", body.key) })),
            )
                .into_response(),
        ),
    }
}

async fn set_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyValue>,
) -> Response {
    let value = match &body.value {
        Some(v) if !v.is_empty() => v,
        _ => {
            return sec_headers(
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "value is required" })),
                )
                    .into_response(),
            );
        }
    };

    match state.vault.set_api_key(&body.key, value) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "ok", "key": body.key, "tier": 1 })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        ),
    }
}

async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.vault.delete_api_key(&body.key) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "deleted", "key": body.key })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        ),
    }
}

// ── Tier 2: Encrypted Secrets ───────────────────────────────────

async fn list_secrets(State(state): State<Arc<AppState>>) -> Response {
    if !state.vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault is locked." })),
            )
                .into_response(),
        );
    }
    let keys = state.vault.list_secrets();
    sec_headers(Json(json!({ "keys": keys })).into_response())
}

async fn get_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    if !state.vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault is locked." })),
            )
                .into_response(),
        );
    }

    match state.vault.get_secret(&body.key) {
        Some(val) => sec_headers(
            Json(json!({ "key": body.key, "value": val.expose_secret() })).into_response(),
        ),
        None => sec_headers(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Secret '{}' not found", body.key) })),
            )
                .into_response(),
        ),
    }
}

async fn set_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyValue>,
) -> Response {
    if !state.vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault is locked." })),
            )
                .into_response(),
        );
    }

    let value = match &body.value {
        Some(v) if !v.is_empty() => v,
        _ => {
            return sec_headers(
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "value is required" })),
                )
                    .into_response(),
            );
        }
    };

    match state.vault.set_secret(&body.key, value) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "ok", "key": body.key, "tier": 2 })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        ),
    }
}

async fn delete_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    if !state.vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault is locked." })),
            )
                .into_response(),
        );
    }

    match state.vault.delete_secret(&body.key) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "deleted", "key": body.key })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        ),
    }
}

// ── KDF helpers ──────────────────────────────────────────────────

fn derive_key_from_passphrase(passphrase: &str) -> ([u8; 32], [u8; 32]) {
    use rand::rngs::OsRng;
    use rand::RngCore;

    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);
    let key = derive_key_with_salt(passphrase, &salt);
    (key, salt)
}

fn derive_key_with_salt(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::Argon2;

    let mut key = [0u8; 32];
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(65536, 3, 1, Some(32)).unwrap(),
    );
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2id derivation failed");
    key
}
