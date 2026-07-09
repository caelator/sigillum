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
mod token_registry;
mod ui;

pub use state::AppState;

#[derive(Debug, thiserror::Error)]
pub enum DaemonInitError {
    #[error("failed to build daemon HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("invalid CORS origin header: {0}")]
    CorsOrigin(#[from] axum::http::header::InvalidHeaderValue),
}

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method, header};
use axum::middleware;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[derive(Clone, Copy, Debug, Default)]
pub struct DaemonRunOptions {
    pub force_daemon_lock: bool,
}

/// Build the Axum router with multi-compartment vault state.
pub fn build_router(
    base_dir: PathBuf,
    listen_port: u16,
) -> Result<(Router, Arc<AppState>), DaemonInitError> {
    if let Err(error) = prepare_base_dir(&base_dir) {
        tracing::warn!(
            path = %base_dir.display(),
            error = %error,
            "failed to prepare daemon base directory"
        );
    }

    let state = Arc::new(AppState::new(base_dir)?);
    let service = service::SigillumService::new(state.clone());
    match service.recover_runtime_state() {
        Ok(_) => state.mark_startup_ready(),
        Err(error) => {
            tracing::warn!(error = %error, "failed to reconcile runtime state during startup");
            state.mark_startup_failed(error.to_string());
        }
    }

    let origin: HeaderValue = format!("http://localhost:{listen_port}").parse()?;

    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true);

    let app = routes::api_router()
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::startup_gate,
        ))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // Snapshot import/export needs larger JSON payloads
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    Ok((app, state))
}

/// Start the daemon and block until shutdown.
/// On SIGINT/SIGTERM, all master keys are zeroized before exit.
pub async fn run(addr: SocketAddr, base_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    run_with_options(addr, base_dir, DaemonRunOptions::default()).await
}

pub async fn run_with_options(
    addr: SocketAddr,
    base_dir: PathBuf,
    options: DaemonRunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    run_inner(addr, base_dir, options, |_state, _handle| {}).await
}

pub async fn run_with_handle<F>(
    addr: SocketAddr,
    base_dir: PathBuf,
    options: DaemonRunOptions,
    on_ready: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce(Arc<AppState>, tokio::runtime::Handle) + Send + 'static,
{
    run_inner(addr, base_dir, options, on_ready).await
}

async fn run_inner<F>(
    addr: SocketAddr,
    base_dir: PathBuf,
    options: DaemonRunOptions,
    on_ready: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce(Arc<AppState>, tokio::runtime::Handle) + Send + 'static,
{
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sigillum_daemon=info,tower_http=info".parse().unwrap()),
        )
        .init();

    prepare_base_dir(&base_dir)?;
    let _daemon_lock = DaemonLock::acquire(&base_dir, options.force_daemon_lock)?;
    let (app, state) = build_router(base_dir, addr.port())?;
    spawn_idle_lock_task(state.clone());

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("sigillum daemon listening on http://{addr}");
    on_ready(state.clone(), tokio::runtime::Handle::current());

    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            {
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
            let _ = shutdown_state.begin_locking();
            let _guard = shutdown_state.operation_guard().await;
            shutdown_state.lock_all();
        })
        .await?;
    Ok(())
}

fn spawn_idle_lock_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if !state.idle_lock_due() || !state.begin_locking() {
                continue;
            }

            let policy = state.runtime_policy();
            let drain = Duration::from_secs(policy.idle_lock_drain_secs);
            let force_after = if policy.idle_lock_force_after_secs == 0 {
                None
            } else {
                Some(Duration::from_secs(policy.idle_lock_force_after_secs))
            };

            let guard = match tokio::time::timeout(drain, state.operation_guard()).await {
                Ok(guard) => Some(guard),
                Err(_) => {
                    tracing::warn!(
                        drain_secs = policy.idle_lock_drain_secs,
                        "idle_lock_drain_exceeded; waiting for guarded operations"
                    );
                    if let Some(force_after) = force_after {
                        match tokio::time::timeout(force_after, state.operation_guard()).await {
                            Ok(guard) => Some(guard),
                            Err(_) => {
                                tracing::error!(
                                    force_after_secs = policy.idle_lock_force_after_secs,
                                    "idle_lock_force_after_exceeded; force-zeroizing unlocked state"
                                );
                                None
                            }
                        }
                    } else {
                        Some(state.operation_guard().await)
                    }
                }
            };

            if guard.is_none() || state.idle_lock_due_after_drain() {
                state.lock_all();
            } else {
                state.finish_locking();
            }
            drop(guard);
        }
    });
}

fn prepare_base_dir(base_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(base_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(base_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Debug)]
struct DaemonLock {
    pid_path: PathBuf,
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    lock_path: PathBuf,
}

impl DaemonLock {
    fn acquire(base_dir: &Path, force: bool) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let lock_path = base_dir.join("daemon.lock");
            let pid_path = base_dir.join("daemon.lock.pid");
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)?;
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    let pid = std::fs::read_to_string(&pid_path)
                        .unwrap_or_else(|_| "unknown".into())
                        .trim()
                        .to_string();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("daemon lock already held by pid {pid}"),
                    ));
                }
                return Err(error);
            }

            if force {
                tracing::warn!("daemon_lock_force_acquired");
            }
            std::fs::write(&pid_path, std::process::id().to_string())?;
            Ok(Self { pid_path, file })
        }

        #[cfg(not(unix))]
        {
            let lock_path = base_dir.join("daemon.lock");
            let pid_path = base_dir.join("daemon.lock.pid");
            if force {
                let _ = std::fs::remove_file(&lock_path);
                tracing::warn!("daemon_lock_force_acquired");
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => {
                    std::fs::write(&pid_path, std::process::id().to_string())?;
                    Ok(Self {
                        lock_path,
                        pid_path,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid = std::fs::read_to_string(&pid_path)
                        .unwrap_or_else(|_| "unknown".into())
                        .trim()
                        .to_string();
                    Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("daemon lock already held by pid {pid}"),
                    ))
                }
                Err(error) => Err(error),
            }
        }
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
        let _ = std::fs::remove_file(&self.pid_path);
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonLock, prepare_base_dir};

    #[test]
    fn prepare_base_dir_restricts_unix_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("sigillum");

        prepare_base_dir(&base).unwrap();

        assert!(base.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&base).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);

            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
            prepare_base_dir(&base).unwrap();
            let repaired = std::fs::metadata(&base).unwrap().permissions().mode() & 0o777;
            assert_eq!(repaired, 0o700);
        }
    }

    #[test]
    fn daemon_lock_refuses_second_holder() {
        let dir = tempfile::tempdir().unwrap();
        prepare_base_dir(dir.path()).unwrap();

        let _first = DaemonLock::acquire(dir.path(), false).unwrap();
        let error = DaemonLock::acquire(dir.path(), false).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("daemon lock already held"));
    }
}
