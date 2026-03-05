//! # Sigillum Daemon
//!
//! HTTP server that holds a `FileVault` in memory. The master key persists
//! across requests, solving the per-process limitation of the CLI.
//!
//! Bind to `localhost:9743` (default) or a custom address.

mod routes;
mod state;
mod ui;

pub use state::AppState;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderValue, Method, header};
use axum::Router;
use tower_http::cors::CorsLayer;

use sigillum_core::{FileVault, VaultConfig};

/// Build the Axum router with shared vault state.
pub fn build_router(config: VaultConfig) -> (Router, Arc<AppState>) {
    let vault = FileVault::new(config);
    let state = Arc::new(AppState::new(vault));

    let cors = CorsLayer::new()
        .allow_origin("http://localhost:9743".parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);

    let app = routes::api_router()
        .layer(cors)
        .with_state(state.clone());

    (app, state)
}

/// Start the daemon and block until shutdown.
pub async fn run(addr: SocketAddr, config: VaultConfig) -> Result<(), Box<dyn std::error::Error>> {
    let (app, _state) = build_router(config);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("sigillum daemon listening on http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
