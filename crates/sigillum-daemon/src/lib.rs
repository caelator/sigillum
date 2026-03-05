//! # Sigillum Daemon
//!
//! HTTP server with multi-compartment vault support. Each compartment is an
//! isolated `FileVault` — the authentication credentials (FIDO2 tap count or
//! passphrase) determine which compartment is accessed.
//!
//! Bind to `localhost:9743` (default) or a custom address.

mod routes;
mod state;
mod ui;

pub use state::AppState;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::{HeaderValue, Method, header};
use axum::Router;
use tower_http::cors::CorsLayer;

/// Build the Axum router with multi-compartment vault state.
pub fn build_router(base_dir: PathBuf) -> (Router, Arc<AppState>) {
    let state = Arc::new(AppState::new(base_dir));

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
pub async fn run(addr: SocketAddr, base_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let (app, _state) = build_router(base_dir);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("sigillum daemon listening on http://{addr}");

    axum::serve(listener, app).await?;
    Ok(())
}
