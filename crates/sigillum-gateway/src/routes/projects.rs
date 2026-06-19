//! Project management endpoints.

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::hash_api_key;
use crate::db;
use crate::error::GatewayError;
use crate::state::AppState;
use crate::validate;

async fn ensure_wallet_profile_exists(
    state: &AppState,
    wallet_profile: &str,
) -> Result<(), GatewayError> {
    let wallet_exists = state
        .daemon
        .list_eth_stealth_wallet_profiles()
        .await?
        .into_iter()
        .any(|profile| profile.name == wallet_profile);

    if wallet_exists {
        Ok(())
    } else {
        Err(GatewayError::BadRequest(format!(
            "wallet_profile '{wallet_profile}' was not found in the daemon"
        )))
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    /// Human-readable project name (must be unique).
    pub name: String,
    /// Name of the Sigillum stealth wallet profile to use.
    pub wallet_profile: String,
    /// Optional webhook URL for payment events.
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectScopesRequest {
    pub scopes: Vec<String>,
}

fn validate_project_scopes(scopes: &[String]) -> Result<(), GatewayError> {
    if scopes.is_empty() {
        return Err(GatewayError::BadRequest("scopes must not be empty".into()));
    }
    for scope in scopes {
        if !db::DEFAULT_PROJECT_SCOPES
            .iter()
            .any(|known| scope == known)
        {
            return Err(GatewayError::BadRequest(format!(
                "unknown project scope '{scope}'"
            )));
        }
    }
    Ok(())
}

/// `POST /api/v1/projects` — register a new project (admin auth required).
///
/// Returns the generated API key (shown only once).
pub async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<Json<Value>, GatewayError> {
    if body.name.is_empty() {
        return Err(GatewayError::BadRequest("name is required".into()));
    }
    if body.wallet_profile.is_empty() {
        return Err(GatewayError::BadRequest(
            "wallet_profile is required".into(),
        ));
    }

    // S3: SSRF protection — validate webhook URL before storing
    if let Some(ref url) = body.webhook_url {
        validate::validate_webhook_url(url)
            .map_err(|e| GatewayError::BadRequest(format!("invalid webhook_url: {e}")))?;
    }

    ensure_wallet_profile_exists(&state, &body.wallet_profile).await?;

    let id = Uuid::new_v4().to_string();
    let api_key = format!("sgw_{}", hex::encode(rand::random::<[u8; 24]>()));
    let api_key_hash = hash_api_key(&api_key);

    // Generate a per-project webhook secret
    let webhook_secret = hex::encode(rand::random::<[u8; 32]>());

    db::insert_project(
        &state.db,
        &id,
        &body.name,
        &api_key_hash,
        &body.wallet_profile,
        body.webhook_url.as_deref(),
        Some(&webhook_secret),
    )
    .await
    .map_err(|e| match e {
        ref error if db::is_unique_constraint(error) => {
            GatewayError::Conflict(format!("project '{}' already exists", body.name))
        }
        other => other,
    })?;

    // Invalidate auth cache so new project is immediately usable
    state.project_cache.invalidate().await;

    Ok(Json(json!({
        "id": id,
        "name": body.name,
        "scopes": db::default_project_scopes(),
        "api_key": api_key,
        "webhook_secret": webhook_secret,
        "message": "Save the api_key — it cannot be retrieved again."
    })))
}

/// `GET /api/v1/projects/:id` — get project details (requires auth).
///
/// Scoped to the authenticated project — a project can only view itself.
pub async fn get_project(
    State(state): State<AppState>,
    axum::Extension(project): axum::Extension<db::row::Project>,
    Path(id): Path<String>,
) -> Result<Json<Value>, GatewayError> {
    // Ownership check: project can only view itself
    if project.id != id {
        return Err(GatewayError::NotFound(format!("project {id} not found")));
    }

    let project = db::find_project_by_id(&state.db, &id)
        .await?
        .ok_or_else(|| GatewayError::NotFound(format!("project {id} not found")))?;

    Ok(Json(json!({
        "id": project.id,
        "name": project.name,
        "wallet_profile": project.wallet_profile,
        "scopes": project.scopes,
        "webhook_url": project.webhook_url,
        "created_at": project.created_at,
    })))
}

pub async fn update_project_scopes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectScopesRequest>,
) -> Result<Json<Value>, GatewayError> {
    validate_project_scopes(&body.scopes)?;
    let project = db::update_project_scopes(&state.db, &id, &body.scopes)
        .await?
        .ok_or_else(|| GatewayError::NotFound(format!("project {id} not found")))?;
    state.project_cache.invalidate().await;

    Ok(Json(json!({
        "id": project.id,
        "name": project.name,
        "wallet_profile": project.wallet_profile,
        "scopes": project.scopes,
        "updated_at": project.updated_at,
    })))
}
