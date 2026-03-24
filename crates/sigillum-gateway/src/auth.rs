//! API key authentication and admin authorization middleware.
//!
//! - **Project auth**: `Authorization: Bearer <project-api-key>` — SHA-256 hashed,
//!   compared in constant time against cached project hashes.
//! - **Admin auth**: `Authorization: Bearer <admin-key>` — required for project creation.
//!
//! The project list is cached in-memory with a configurable TTL (A2) to avoid
//! loading all rows from SQLite on every request.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;

use crate::db;
use crate::db::row::Project;
use crate::error::GatewayError;
use crate::state::AppState;

/// Hash an API key with SHA-256 for storage/lookup.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time comparison of two hex-encoded hashes.
fn ct_hash_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    if a_bytes.len() != b_bytes.len() {
        return false;
    }
    a_bytes.ct_eq(b_bytes).into()
}

/// In-memory project cache with TTL.
#[derive(Clone)]
pub struct ProjectCache {
    inner: Arc<RwLock<CacheInner>>,
    ttl_secs: u64,
}

struct CacheInner {
    projects: Vec<Project>,
    last_refresh: Instant,
}

impl ProjectCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CacheInner {
                projects: Vec::new(),
                last_refresh: Instant::now() - std::time::Duration::from_secs(ttl_secs + 1),
            })),
            ttl_secs,
        }
    }

    /// Get all projects, refreshing from DB if the cache has expired.
    async fn get_projects(&self, pool: &sqlx::SqlitePool) -> Result<Vec<Project>, GatewayError> {
        // Fast path: read lock to check if cache is fresh
        {
            let guard = self.inner.read().await;
            if guard.last_refresh.elapsed().as_secs() < self.ttl_secs {
                return Ok(guard.projects.clone());
            }
        }

        // Slow path: write lock to refresh
        let mut guard = self.inner.write().await;
        // Double-check — another task may have refreshed while we waited for the lock
        if guard.last_refresh.elapsed().as_secs() < self.ttl_secs {
            return Ok(guard.projects.clone());
        }

        let projects = db::list_projects(pool).await?;
        guard.projects = projects.clone();
        guard.last_refresh = Instant::now();
        Ok(projects)
    }

    /// Invalidate the cache (e.g. after creating a new project).
    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        guard.last_refresh = Instant::now() - std::time::Duration::from_secs(self.ttl_secs + 1);
    }
}

/// Axum middleware that validates the Bearer token against project API keys.
///
/// Uses constant-time comparison against cached project hashes (A2).
/// On success, inserts the authenticated `Project` into request extensions.
pub async fn require_api_key(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, GatewayError> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(GatewayError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(GatewayError::Unauthorized)?;

    let incoming_hash = hash_api_key(token);

    // Load from cache (refreshes from DB if TTL expired)
    let projects = state.project_cache.get_projects(&state.db).await?;
    let matched = projects
        .into_iter()
        .find(|p| ct_hash_eq(&incoming_hash, &p.api_key_hash));

    let project = matched.ok_or(GatewayError::Unauthorized)?;
    req.extensions_mut().insert(project);
    Ok(next.run(req).await)
}

/// Axum middleware that validates the admin API key for protected management endpoints.
pub async fn require_admin_key(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, GatewayError> {
    let expected_hash = state
        .config
        .admin_key_hash
        .as_ref()
        .ok_or(GatewayError::Unauthorized)?;

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(GatewayError::Unauthorized)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(GatewayError::Unauthorized)?;

    let incoming_hash = hash_api_key(token);
    if !ct_hash_eq(&incoming_hash, expected_hash) {
        return Err(GatewayError::Unauthorized);
    }

    Ok(next.run(req).await)
}
