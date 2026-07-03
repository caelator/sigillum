//! Sigillum Gateway — local-sidecar payment preview and control surface.
//!
//! Bridges websites/services with the Sigillum daemon, providing:
//! - Project/tenant management with API key authentication
//! - Payment lifecycle (create stealth address → monitor → sweep)
//! - Webhook notifications on payment state changes
//! - Embeddable payment widget

mod auth;
mod config;
mod db;
mod error;
mod poller;
mod routes;
mod state;
mod validate;
mod webhooks;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;

use std::net::SocketAddr;

use sigillum_client::SigillumClient;
use tracing_subscriber::EnvFilter;

use crate::auth::ProjectCache;
use crate::config::GatewayConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    let config = GatewayConfig::from_env().unwrap_or_else(|error| {
        eprintln!("Gateway configuration error: {error}");
        std::process::exit(1);
    });

    // P3: Structured JSON logging when GATEWAY_LOG_JSON=1
    if config.log_json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();
    }

    tracing::info!("Connecting to database: {}", config.database_url);
    let db = db::connect(&config.database_url)
        .await
        .unwrap_or_else(|error| {
            tracing::error!("Failed to connect to database: {error}");
            std::process::exit(1);
        });

    let daemon = Arc::new(
        SigillumClient::new(&config.daemon_url).unwrap_or_else(|error| {
            eprintln!("Failed to build daemon client: {error}");
            std::process::exit(1);
        }),
    );
    if let Some(token) = config.daemon_capability_token.as_ref() {
        daemon.set_session_token(token.clone());
        tracing::info!("Using SIGILLUM_DAEMON_CAPABILITY_TOKEN for daemon calls");
    } else if let Some(token) = config.daemon_session_token.as_ref() {
        daemon.set_session_token(token.clone());
        tracing::warn!(
            "Using full daemon session token; configure SIGILLUM_DAEMON_CAPABILITY_TOKEN to reduce gateway scope"
        );
    } else {
        tracing::warn!(
            "No daemon token configured; authenticated gateway operations will fail until SIGILLUM_DAEMON_CAPABILITY_TOKEN or SIGILLUM_DAEMON_SESSION_TOKEN is provided"
        );
    }
    tracing::info!("Daemon URL: {}", config.daemon_url);

    let state = AppState {
        db,
        daemon,
        config: config.clone(),
        daemon_healthy: Arc::new(AtomicBool::new(true)),
        poll_failures: Arc::new(AtomicU32::new(0)),
        project_cache: ProjectCache::new(config.auth_cache_ttl_secs),
    };

    // Start background poller
    poller::spawn(state.clone());
    tracing::info!(
        "Background poller started (interval: {}s)",
        config.poll_interval_secs
    );

    // Build and start HTTP server with graceful shutdown (R1)
    let app = routes::build_router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .unwrap_or_else(|error| {
            tracing::error!("Failed to bind {}: {error}", config.bind_addr);
            std::process::exit(1);
        });
    tracing::info!("Gateway listening on {}", config.bind_addr);

    if config.rate_limit_rps > 0 {
        tracing::info!("Rate limiting: {} req/s per IP", config.rate_limit_rps);
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap_or_else(|error| {
        tracing::error!("Server error: {error}");
        std::process::exit(1);
    });

    tracing::info!("Gateway shut down gracefully");
}

/// Listen for SIGINT/SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.unwrap_or_else(|error| {
            // Without a signal handler, the gateway has no reliable clean shutdown path.
            tracing::error!("Failed to install Ctrl+C handler: {error}");
            std::process::exit(1);
        });
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .unwrap_or_else(|error| {
                // Without a signal handler, the gateway has no reliable clean shutdown path.
                tracing::error!("Failed to install SIGTERM handler: {error}");
                std::process::exit(1);
            })
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT, shutting down..."); }
        _ = terminate => { tracing::info!("Received SIGTERM, shutting down..."); }
    }
}
