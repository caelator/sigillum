//! Shared application state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use sigillum_client::SigillumClient;
use sqlx::SqlitePool;

use crate::auth::ProjectCache;
use crate::config::GatewayConfig;

/// Shared state threaded through all handlers via Axum's `State` extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub daemon: Arc<SigillumClient>,
    pub config: GatewayConfig,
    /// Daemon health flag — set to false after consecutive poll failures.
    pub daemon_healthy: Arc<AtomicBool>,
    /// Consecutive poller failure count.
    pub poll_failures: Arc<AtomicU32>,
    /// In-memory project cache (A2).
    pub project_cache: ProjectCache,
}
