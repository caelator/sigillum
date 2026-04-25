//! # Sigillum Daemon
//!
//! HTTP server with multi-compartment vault support. Each compartment is an
//! isolated `FileVault` — the authentication credentials (FIDO2 tap count or
//! passphrase) determine which compartment is accessed.
//!
//! Bind to `localhost:9743` (default) or a custom address.
//!
//! ## Architecture: Daemon ↔ Service ↔ State
//!
//! The daemon is organized in three layers:
//! - **Daemon** (this module): HTTP lifecycle management, signal handling, graceful shutdown.
//!   On SIGINT/SIGTERM, all master keys are zeroized before exit.
//! - **Service** (`service` module): Business logic that bridges compartment lookup,
//!   authentication, and vault operations. Handles operation journals, audit logging,
//!   and runtime policy enforcement.
//! - **State** (`state` module): Persistent application state (`AppState`) holding
//!   the multi-compartment vault, policy cache, and I/O directories.
//!
//! The service layer (not the daemon) owns recovery logic and policy initialization.
//! The daemon's job is to serve HTTP and manage process lifecycle cleanly.

mod api;
mod audit;
mod audit_db;
mod audit_log;
mod deposits;
mod inventory;
mod json_store;
mod operations;
mod policy;
mod profiles;
mod queue_store;
mod routes;
mod service;
mod state;
mod ui;

pub use state::AppState;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method, header};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Build the Axum router with multi-compartment vault state.
pub fn build_router(base_dir: PathBuf, listen_port: u16) -> (Router, Arc<AppState>) {
    let state = Arc::new(AppState::new(base_dir));
    let service = service::SigillumService::new(state.clone());
    if let Err(error) = service.recover_runtime_state() {
        tracing::warn!(error = %error, "failed to reconcile runtime state during startup");
    }

    let origin: HeaderValue = format!("http://localhost:{listen_port}")
        .parse()
        .expect("valid CORS origin from listen port");

    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);

    let app = routes::api_router()
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // Snapshot import/export needs larger JSON payloads
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    (app, state)
}

/// Start the daemon and block until shutdown.
/// On SIGINT/SIGTERM, all master keys are zeroized before exit.
pub async fn run(addr: SocketAddr, base_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sigillum_daemon=info,tower_http=info".parse().unwrap()),
        )
        .init();

    let (app, state) = build_router(base_dir, addr.port());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("sigillum daemon listening on http://{addr}");

    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            {
                let mut sigterm =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("failed to install SIGTERM handler");
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = sigterm.recv() => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = ctrl_c.await;
            }
            tracing::info!("shutting down — zeroizing all master keys");
            shutdown_state.lock_all();
        })
        .await?;
    Ok(())
}
