use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
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

#[derive(Deserialize)]
struct Fido2SetupBody {
    pin: String,
    label: String,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct Fido2RegisterBody {
    pin: String,
    label: String,
}

#[derive(Deserialize)]
struct Fido2UnlockBody {
    pins: Vec<String>,
}

#[derive(Deserialize)]
struct Fido2RemoveBody {
    label: String,
    pin: String,
}

#[derive(Deserialize)]
struct Fido2QuorumBody {
    threshold: usize,
}

// ── Router ───────────────────────────────────────────────────────

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        // Web UI
        .route("/", get(serve_ui))
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
        // FIDO2
        .route("/api/fido2/status", get(fido2_status))
        .route("/api/fido2/detect", get(fido2_detect))
        .route("/api/fido2/list", get(fido2_list))
        .route("/api/fido2/setup", post(fido2_setup))
        .route("/api/fido2/register", post(fido2_register))
        .route("/api/fido2/unlock", post(fido2_unlock))
        .route("/api/fido2/remove", post(fido2_remove))
        .route("/api/fido2/set-quorum", post(fido2_set_quorum))
}

// ── Web UI ───────────────────────────────────────────────────────

async fn serve_ui() -> Html<&'static str> {
    Html(crate::ui::INDEX_HTML)
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

    let fido_status = state.fido2.status();

    sec_headers(
        Json(json!({
            "vault_exists": exists,
            "unlocked": unlocked,
            "api_key_count": api_key_count,
            "secret_count": secret_count,
            "fido2": {
                "enabled": fido_status.enabled,
                "key_count": fido_status.key_count,
                "quorum_threshold": fido_status.quorum_threshold,
                "unlock_method": fido_status.unlock_method,
            }
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

    // Check if this is a "wrapped" passphrase mode
    let fido_config = state.fido2.load_config_raw();
    let is_wrapped = fido_config.passphrase_mode.as_deref() == Some("wrapped");

    if is_wrapped {
        let salt = match std::fs::read(state.salt_path()) {
            Ok(s) if s.len() == 32 => s,
            _ => {
                return sec_headers(
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "Cannot read salt file." })),
                    )
                        .into_response(),
                );
            }
        };
        let wrap_key = derive_key_with_salt(&body.passphrase, &salt);
        match load_wrapped_master_key(&wrap_key, state.wrapped_key_path()) {
            Some(master_key) => {
                vault.load_master_key(master_key);
                if vault.verify_master_key() {
                    return sec_headers(
                        Json(json!({ "status": "unlocked", "method": "passphrase_wrapped" }))
                            .into_response(),
                    );
                }
                vault.zeroize_master_key();
                return sec_headers(
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "Wrong passphrase." })),
                    )
                        .into_response(),
                );
            }
            None => {
                return sec_headers(
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "Wrong passphrase or corrupted wrapped key." })),
                    )
                        .into_response(),
                );
            }
        }
    }

    // Direct mode (legacy)
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
        sec_headers(Json(json!({ "status": "unlocked", "method": "passphrase" })).into_response())
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

// ── FIDO2 ────────────────────────────────────────────────────────

async fn fido2_status(State(state): State<Arc<AppState>>) -> Response {
    let s = state.fido2.status();
    sec_headers(Json(json!(s)).into_response())
}

async fn fido2_detect() -> Response {
    let count = sigillum_fido2::hid::detect_devices();
    sec_headers(
        Json(json!({
            "device_present": count > 0,
            "device_count": count,
        }))
        .into_response(),
    )
}

async fn fido2_list(State(state): State<Arc<AppState>>) -> Response {
    let keys = state.fido2.list_keys();
    sec_headers(Json(json!({ "keys": keys })).into_response())
}

async fn fido2_setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2SetupBody>,
) -> Response {
    let vault = &state.vault;

    if vault.vault_exists() {
        return sec_headers(
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": "Vault already exists. Use /api/fido2/register to add keys." })),
            )
                .into_response(),
        );
    }

    if body.label.is_empty() {
        return sec_headers(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "label is required" })),
            )
                .into_response(),
        );
    }

    // Register first FIDO2 key
    match state.fido2.register_key(&body.pin, &body.label, None) {
        Ok(result) => {
            // Initialize vault with generated master key
            if let Err(e) = vault.initialize(&result.master_key) {
                return sec_headers(
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("Failed to initialize vault: {e}") })),
                    )
                        .into_response(),
                );
            }

            // Auto-unlock
            vault.load_master_key(*result.master_key);

            // Optionally set passphrase fallback
            if let Some(ref passphrase) = body.passphrase {
                if passphrase.len() >= 8 {
                    let (wrap_key, salt) = derive_key_from_passphrase(passphrase);
                    save_salt(&salt, state.salt_path());
                    save_wrapped_master_key(&result.master_key, &wrap_key, state.wrapped_key_path());

                    let mut config = state.fido2.load_config_raw();
                    config.unlock_method = "both".into();
                    config.passphrase_mode = Some("wrapped".into());
                    let _ = state.fido2.save_config_raw(&config);
                }
            }

            sec_headers(
                Json(json!({
                    "status": "setup_complete",
                    "is_first_key": result.is_first_key,
                    "total_keys": result.total_keys,
                    "unlocked": true,
                }))
                .into_response(),
            )
        }
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("FIDO2 setup failed: {e}") })),
            )
                .into_response(),
        ),
    }
}

async fn fido2_register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2RegisterBody>,
) -> Response {
    let vault = &state.vault;

    if !vault.vault_exists() {
        return sec_headers(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No vault found. Use /api/fido2/setup first." })),
            )
                .into_response(),
        );
    }

    if !vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault must be unlocked to register a key." })),
            )
                .into_response(),
        );
    }

    let existing_mk = vault.extract_master_key();
    let mk_ref = existing_mk.as_ref().map(|k| &**k);

    match state.fido2.register_key(&body.pin, &body.label, mk_ref) {
        Ok(result) => sec_headers(
            Json(json!({
                "status": "registered",
                "label": body.label,
                "total_keys": result.total_keys,
            }))
            .into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Registration failed: {e}") })),
            )
                .into_response(),
        ),
    }
}

async fn fido2_unlock(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2UnlockBody>,
) -> Response {
    let vault = &state.vault;

    if vault.is_unlocked() {
        return sec_headers(Json(json!({ "status": "already_unlocked" })).into_response());
    }

    if !vault.vault_exists() {
        return sec_headers(
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "No vault found." })),
            )
                .into_response(),
        );
    }

    if body.pins.is_empty() {
        return sec_headers(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "At least one PIN required." })),
            )
                .into_response(),
        );
    }

    match state.fido2.authenticate_quorum(&body.pins, None) {
        Ok(master_key) => {
            vault.load_master_key(*master_key);
            if vault.verify_master_key() {
                sec_headers(
                    Json(json!({ "status": "unlocked", "method": "fido2" })).into_response(),
                )
            } else {
                vault.zeroize_master_key();
                sec_headers(
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "FIDO2 key does not match vault." })),
                    )
                        .into_response(),
                )
            }
        }
        Err(e) => sec_headers(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": format!("FIDO2 unlock failed: {e}") })),
            )
                .into_response(),
        ),
    }
}

async fn fido2_remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2RemoveBody>,
) -> Response {
    let vault = &state.vault;

    if !vault.is_unlocked() {
        return sec_headers(
            (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Vault must be unlocked." })),
            )
                .into_response(),
        );
    }

    let master_key = match vault.extract_master_key() {
        Some(mk) => mk,
        None => {
            return sec_headers(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Cannot extract master key." })),
                )
                    .into_response(),
            );
        }
    };

    match state.fido2.remove_key(&body.label, &master_key, &body.pin) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "removed", "label": body.label })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Removal failed: {e}") })),
            )
                .into_response(),
        ),
    }
}

async fn fido2_set_quorum(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2QuorumBody>,
) -> Response {
    match state.fido2.set_quorum(body.threshold) {
        Ok(()) => sec_headers(
            Json(json!({ "status": "ok", "threshold": body.threshold })).into_response(),
        ),
        Err(e) => sec_headers(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("{e}") })),
            )
                .into_response(),
        ),
    }
}

// ── KDF + wrapped key helpers ───────────────────────────────────

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

fn save_salt(salt: &[u8; 32], path: &std::path::Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, salt);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn save_wrapped_master_key(master_key: &[u8; 32], wrap_key: &[u8; 32], path: &std::path::Path) {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    use rand::rngs::OsRng;
    use rand::RngCore;

    let cipher = Aes256Gcm::new_from_slice(wrap_key).expect("wrap key length");
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, master_key.as_ref())
        .expect("wrap encryption");

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, &output);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn load_wrapped_master_key(wrap_key: &[u8; 32], path: &std::path::Path) -> Option<[u8; 32]> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = std::fs::read(path).ok()?;
    if data.len() < 12 {
        return None;
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(wrap_key).ok()?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .ok()?;
    if plaintext.len() < 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext[..32]);
    Some(key)
}
