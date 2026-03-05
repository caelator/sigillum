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

fn err(status: StatusCode, msg: &str) -> Response {
    sec_headers((status, Json(json!({ "error": msg }))).into_response())
}

fn ok_json(val: serde_json::Value) -> Response {
    sec_headers(Json(val).into_response())
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
    compartments: Vec<CompartmentDefBody>,
    passphrase: Option<String>,
}

#[derive(Deserialize)]
struct CompartmentDefBody {
    label: String,
    threshold: usize,
    passphrase_mode: Option<String>,
}

#[derive(Deserialize)]
struct Fido2RegisterBody {
    pin: String,
    label: String,
}

#[derive(Deserialize)]
struct Fido2UnlockBody {
    pins: Vec<String>,
    tap_count: usize,
}

#[derive(Deserialize)]
struct Fido2RemoveBody {
    label: String,
    pin: String,
}

#[derive(Deserialize)]
struct CompartmentAddBody {
    label: String,
    threshold: usize,
    passphrase_mode: Option<String>,
}

#[derive(Deserialize)]
struct CompartmentRemoveBody {
    id: usize,
}

#[derive(Deserialize)]
struct CompartmentInitBody {
    id: usize,
    passphrase: String,
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
        // Compartments
        .route("/api/compartment/list", get(compartment_list))
        .route("/api/compartment/add", post(compartment_add))
        .route("/api/compartment/remove", post(compartment_remove))
        .route("/api/compartment/init", post(compartment_init))
        // Tier 1 — API keys (operate on active compartment)
        .route("/api/api-keys", get(list_api_keys))
        .route("/api/api-keys/get", post(get_api_key))
        .route("/api/api-keys/set", post(set_api_key))
        .route("/api/api-keys/delete", post(delete_api_key))
        // Tier 2 — Encrypted secrets (operate on active compartment)
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
}

// ── Web UI ───────────────────────────────────────────────────────

async fn serve_ui() -> Html<&'static str> {
    Html(crate::ui::INDEX_HTML)
}

// ── Status ───────────────────────────────────────────────────────

async fn get_status(State(state): State<Arc<AppState>>) -> Response {
    let fido_status = state.fido2.status();
    let active_id = state.active_compartment_id();
    let any_exists = state.any_vault_exists();

    let active_info = active_id.and_then(|id| {
        state.with_vault(id, |v| {
            json!({
                "compartment_id": id,
                "unlocked": v.is_unlocked(),
                "api_key_count": v.list_api_keys().len(),
                "secret_count": if v.is_unlocked() { Some(v.list_secrets().len()) } else { None },
            })
        })
    });

    ok_json(json!({
        "any_vault_exists": any_exists,
        "active_compartment": active_info,
        "fido2": fido_status,
    }))
}

// ── Lifecycle ────────────────────────────────────────────────────

async fn post_unlock(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PassphraseBody>,
) -> Response {
    if state.active_compartment_id().is_some() {
        return ok_json(json!({ "status": "already_unlocked" }));
    }

    if body.passphrase.is_empty() {
        return err(StatusCode::BAD_REQUEST, "Passphrase required.");
    }

    // Probe each compartment's wrapped key
    let config = state.fido2.load_config_raw();
    for comp in &config.compartments {
        if comp.passphrase_mode.as_deref() != Some("wrapped") {
            continue;
        }

        let salt_path = state.salt_path(comp.id);
        let wrapped_path = state.wrapped_key_path(comp.id);

        let salt = match std::fs::read(&salt_path) {
            Ok(s) if s.len() == 32 => s,
            _ => continue,
        };

        let wrap_key = derive_key_with_salt(&body.passphrase, &salt);
        if let Some(master_key) = load_wrapped_master_key(&wrap_key, &wrapped_path) {
            state.ensure_vault(comp.id);
            let verified = state.with_vault(comp.id, |v| {
                v.load_master_key(master_key);
                let ok = v.verify_master_key();
                if !ok {
                    v.zeroize_master_key();
                }
                ok
            });

            if verified == Some(true) {
                state.set_active(Some(comp.id));
                return ok_json(json!({
                    "status": "unlocked",
                    "method": "passphrase",
                    "compartment_id": comp.id,
                    "compartment_label": comp.label,
                }));
            }
        }
    }

    err(StatusCode::UNAUTHORIZED, "No compartment matched this passphrase.")
}

async fn post_lock(State(state): State<Arc<AppState>>) -> Response {
    state.lock_all();
    ok_json(json!({
        "status": "locked",
        "message": "All compartments locked. Master keys zeroized."
    }))
}

// ── Compartments ────────────────────────────────────────────────

async fn compartment_list(State(state): State<Arc<AppState>>) -> Response {
    let config = state.fido2.load_config_raw();
    let active = state.active_compartment_id();

    let compartments: Vec<_> = config.compartments.iter().map(|c| {
        let exists = state.with_vault(c.id, |v| v.vault_exists()).unwrap_or(false);
        json!({
            "id": c.id,
            "label": c.label,
            "threshold": c.threshold,
            "passphrase_mode": c.passphrase_mode,
            "vault_exists": exists,
            "is_active": active == Some(c.id),
        })
    }).collect();

    ok_json(json!({ "compartments": compartments }))
}

async fn compartment_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CompartmentAddBody>,
) -> Response {
    if body.label.is_empty() {
        return err(StatusCode::BAD_REQUEST, "label is required");
    }
    if body.threshold == 0 {
        return err(StatusCode::BAD_REQUEST, "threshold must be >= 1");
    }

    let config = state.fido2.load_config_raw();
    let id = config.next_compartment_id();

    let def = sigillum_fido2::config::CompartmentDef {
        id,
        label: body.label.clone(),
        threshold: body.threshold,
        passphrase_mode: body.passphrase_mode.clone(),
    };

    match state.fido2.add_compartment(def) {
        Ok(()) => {
            state.ensure_vault(id);
            ok_json(json!({
                "status": "added",
                "id": id,
                "label": body.label,
                "threshold": body.threshold,
            }))
        }
        Err(e) => err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn compartment_remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CompartmentRemoveBody>,
) -> Response {
    match state.fido2.remove_compartment(body.id) {
        Ok(()) => ok_json(json!({ "status": "removed", "id": body.id })),
        Err(e) => err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

async fn compartment_init(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CompartmentInitBody>,
) -> Response {
    if body.passphrase.len() < 8 {
        return err(StatusCode::BAD_REQUEST, "Passphrase must be at least 8 characters.");
    }

    let config = state.fido2.load_config_raw();
    let comp = match config.compartment_by_id(body.id) {
        Some(c) => c.clone(),
        None => return err(StatusCode::NOT_FOUND, &format!("Compartment {} not found", body.id)),
    };

    state.ensure_vault(body.id);

    let already = state.with_vault(body.id, |v| v.vault_exists()).unwrap_or(false);
    if already {
        return err(StatusCode::CONFLICT, "Compartment vault already initialized.");
    }

    // Generate random master key, wrap with passphrase
    use rand::RngCore;
    let mut master_key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut master_key);

    let (wrap_key, salt) = derive_key_from_passphrase(&body.passphrase);

    // Initialize vault
    let init_result = state.with_vault(body.id, |v| v.initialize(&master_key));
    match init_result {
        Some(Ok(())) => {}
        Some(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Init failed: {e}")),
        None => return err(StatusCode::INTERNAL_SERVER_ERROR, "Vault not found"),
    }

    // Save salt and wrapped key
    let salt_path = state.salt_path(body.id);
    let wrapped_path = state.wrapped_key_path(body.id);
    save_salt(&salt, &salt_path);
    save_wrapped_master_key(&master_key, &wrap_key, &wrapped_path);

    // Update compartment passphrase_mode if needed
    if comp.passphrase_mode.is_none() {
        let mut cfg = state.fido2.load_config_raw();
        if let Some(c) = cfg.compartments.iter_mut().find(|c| c.id == body.id) {
            c.passphrase_mode = Some("wrapped".into());
        }
        let _ = state.fido2.save_config_raw(&cfg);
    }

    // Auto-unlock
    state.unlock_compartment(body.id, master_key);

    // Zeroize local copy
    zeroize::Zeroize::zeroize(&mut master_key);

    ok_json(json!({
        "status": "initialized",
        "compartment_id": body.id,
        "compartment_label": comp.label,
    }))
}

// ── Tier 1: API Keys (active compartment) ──────────────────────

async fn list_api_keys(State(state): State<Arc<AppState>>) -> Response {
    match state.with_active_vault(|v| v.list_api_keys()) {
        Some(keys) => ok_json(json!({ "keys": keys })),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn get_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.with_active_vault(|v| v.get_api_key(&body.key).map(|s| s.expose_secret().to_string())) {
        Some(Some(val)) => ok_json(json!({ "key": body.key, "value": val })),
        Some(None) => err(StatusCode::NOT_FOUND, &format!("API key '{}' not found", body.key)),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn set_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyValue>,
) -> Response {
    let value = match &body.value {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return err(StatusCode::BAD_REQUEST, "value is required"),
    };

    match state.with_active_vault(|v| v.set_api_key(&body.key, &value)) {
        Some(Ok(())) => ok_json(json!({ "status": "ok", "key": body.key, "tier": 1 })),
        Some(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.with_active_vault(|v| v.delete_api_key(&body.key)) {
        Some(Ok(())) => ok_json(json!({ "status": "deleted", "key": body.key })),
        Some(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

// ── Tier 2: Encrypted Secrets (active compartment) ─────────────

async fn list_secrets(State(state): State<Arc<AppState>>) -> Response {
    match state.with_active_vault(|v| {
        if !v.is_unlocked() { return Err("locked"); }
        Ok(v.list_secrets())
    }) {
        Some(Ok(keys)) => ok_json(json!({ "keys": keys })),
        Some(Err(_)) => err(StatusCode::FORBIDDEN, "Vault is locked."),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn get_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.with_active_vault(|v| {
        if !v.is_unlocked() { return Err("locked"); }
        match v.get_secret(&body.key) {
            Some(val) => Ok(val.expose_secret().to_string()),
            None => Err("not_found"),
        }
    }) {
        Some(Ok(val)) => ok_json(json!({ "key": body.key, "value": val })),
        Some(Err("locked")) => err(StatusCode::FORBIDDEN, "Vault is locked."),
        Some(Err(_)) => err(StatusCode::NOT_FOUND, &format!("Secret '{}' not found", body.key)),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn set_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyValue>,
) -> Response {
    let value = match &body.value {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return err(StatusCode::BAD_REQUEST, "value is required"),
    };

    match state.with_active_vault(|v| {
        if !v.is_unlocked() { return Err("locked".to_string()); }
        v.set_secret(&body.key, &value).map_err(|e| e.to_string())
    }) {
        Some(Ok(())) => ok_json(json!({ "status": "ok", "key": body.key, "tier": 2 })),
        Some(Err(ref e)) if e == "locked" => err(StatusCode::FORBIDDEN, "Vault is locked."),
        Some(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

async fn delete_secret(
    State(state): State<Arc<AppState>>,
    Json(body): Json<KeyOnly>,
) -> Response {
    match state.with_active_vault(|v| {
        if !v.is_unlocked() { return Err("locked".to_string()); }
        v.delete_secret(&body.key).map_err(|e| e.to_string())
    }) {
        Some(Ok(())) => ok_json(json!({ "status": "deleted", "key": body.key })),
        Some(Err(ref e)) if e == "locked" => err(StatusCode::FORBIDDEN, "Vault is locked."),
        Some(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
        None => err(StatusCode::FORBIDDEN, "No active compartment."),
    }
}

// ── FIDO2 ────────────────────────────────────────────────────────

async fn fido2_status(State(state): State<Arc<AppState>>) -> Response {
    let s = state.fido2.status();
    ok_json(json!(s))
}

async fn fido2_detect() -> Response {
    let count = sigillum_fido2::hid::detect_devices();
    ok_json(json!({
        "device_present": count > 0,
        "device_count": count,
    }))
}

async fn fido2_list(State(state): State<Arc<AppState>>) -> Response {
    let keys = state.fido2.list_keys();
    ok_json(json!({ "keys": keys }))
}

async fn fido2_setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2SetupBody>,
) -> Response {
    if state.any_vault_exists() {
        return err(StatusCode::CONFLICT, "Vaults already exist. Use /api/fido2/register to add keys.");
    }

    if body.label.is_empty() {
        return err(StatusCode::BAD_REQUEST, "label is required");
    }
    if body.compartments.is_empty() {
        return err(StatusCode::BAD_REQUEST, "at least one compartment required");
    }

    // Create compartment definitions
    for (i, comp) in body.compartments.iter().enumerate() {
        let def = sigillum_fido2::config::CompartmentDef {
            id: i,
            label: comp.label.clone(),
            threshold: comp.threshold,
            passphrase_mode: comp.passphrase_mode.clone(),
        };
        if let Err(e) = state.fido2.add_compartment(def) {
            return err(StatusCode::BAD_REQUEST, &format!("Compartment error: {e}"));
        }
    }

    // Register first FIDO2 key (no existing master keys — will generate them)
    match state.fido2.register_key(&body.pin, &body.label, &[]) {
        Ok(result) => {
            // Initialize each compartment vault with its generated master key
            for (comp_id, mk) in &result.compartment_keys {
                state.ensure_vault(*comp_id);
                let init_result = state.with_vault(*comp_id, |v| v.initialize(mk));
                if let Some(Err(e)) = init_result {
                    return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Failed to init compartment {comp_id}: {e}"));
                }
            }

            // Set up passphrase fallback if provided
            if let Some(ref passphrase) = body.passphrase {
                if passphrase.len() >= 8 {
                    for (comp_id, mk) in &result.compartment_keys {
                        let (wrap_key, salt) = derive_key_from_passphrase(passphrase);
                        save_salt(&salt, &state.salt_path(*comp_id));
                        save_wrapped_master_key(mk, &wrap_key, &state.wrapped_key_path(*comp_id));
                    }
                    // Update compartments to have wrapped passphrase mode
                    let mut cfg = state.fido2.load_config_raw();
                    for c in &mut cfg.compartments {
                        c.passphrase_mode = Some("wrapped".into());
                    }
                    let _ = state.fido2.save_config_raw(&cfg);
                }
            }

            // Auto-unlock first compartment
            if let Some((comp_id, mk)) = result.compartment_keys.first() {
                state.unlock_compartment(*comp_id, **mk);
            }

            ok_json(json!({
                "status": "setup_complete",
                "is_first_key": result.is_first_key,
                "total_keys": result.total_keys,
                "compartments": body.compartments.len(),
                "unlocked": true,
            }))
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("FIDO2 setup failed: {e}")),
    }
}

async fn fido2_register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2RegisterBody>,
) -> Response {
    if !state.any_vault_exists() {
        return err(StatusCode::NOT_FOUND, "No vaults found. Use /api/fido2/setup first.");
    }

    // All compartments must be unlocked to re-encrypt shards
    let master_keys = state.extract_all_master_keys();
    if master_keys.is_empty() {
        return err(StatusCode::FORBIDDEN, "At least one compartment must be unlocked.");
    }

    let mk_refs: Vec<(usize, &[u8; 32])> = master_keys.iter()
        .map(|(id, mk)| (*id, &**mk))
        .collect();

    match state.fido2.register_key(&body.pin, &body.label, &mk_refs) {
        Ok(result) => ok_json(json!({
            "status": "registered",
            "label": body.label,
            "total_keys": result.total_keys,
        })),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Registration failed: {e}")),
    }
}

async fn fido2_unlock(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2UnlockBody>,
) -> Response {
    if state.active_compartment_id().is_some() {
        return ok_json(json!({ "status": "already_unlocked" }));
    }

    if body.pins.is_empty() {
        return err(StatusCode::BAD_REQUEST, "At least one PIN required.");
    }
    if body.tap_count == 0 {
        return err(StatusCode::BAD_REQUEST, "tap_count must be >= 1.");
    }

    match state.fido2.authenticate_compartment(&body.pins, body.tap_count, None) {
        Ok((comp_id, master_key)) => {
            state.ensure_vault(comp_id);
            let verified = state.with_vault(comp_id, |v| {
                v.load_master_key(*master_key);
                let ok = v.verify_master_key();
                if !ok {
                    v.zeroize_master_key();
                }
                ok
            });

            if verified == Some(true) {
                state.set_active(Some(comp_id));
                let config = state.fido2.load_config_raw();
                let label = config.compartment_by_id(comp_id)
                    .map(|c| c.label.clone())
                    .unwrap_or_default();

                ok_json(json!({
                    "status": "unlocked",
                    "method": "fido2",
                    "compartment_id": comp_id,
                    "compartment_label": label,
                }))
            } else {
                err(StatusCode::UNAUTHORIZED, "FIDO2 key does not match compartment vault.")
            }
        }
        Err(e) => err(StatusCode::UNAUTHORIZED, &format!("FIDO2 unlock failed: {e}")),
    }
}

async fn fido2_remove(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Fido2RemoveBody>,
) -> Response {
    let master_keys = state.extract_all_master_keys();
    if master_keys.is_empty() {
        return err(StatusCode::FORBIDDEN, "At least one compartment must be unlocked.");
    }

    let mk_refs: Vec<(usize, &[u8; 32])> = master_keys.iter()
        .map(|(id, mk)| (*id, &**mk))
        .collect();

    match state.fido2.remove_key(&body.label, &mk_refs, &body.pin) {
        Ok(()) => ok_json(json!({ "status": "removed", "label": body.label })),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Removal failed: {e}")),
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
