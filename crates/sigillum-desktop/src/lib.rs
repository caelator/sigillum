//! Non-UI daemon lifecycle helpers for the Sigillum desktop shell.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc,
    mpsc::{self, Receiver},
};
use std::thread;
use std::time::{Duration, Instant};

use sigillum_daemon::{AppState, DaemonRunOptions};
use tokio::runtime::Handle;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const CONNECT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Shared daemon control state managed by the Tauri app.
pub struct DaemonControl {
    /// The daemon application state.
    pub state: Arc<AppState>,
    /// Runtime handle used to schedule daemon work.
    pub handle: Handle,
}

/// Channels returned while the daemon thread is starting.
pub struct DaemonStart {
    /// Receives daemon startup or runtime errors.
    pub errors: Receiver<String>,
    /// Receives the daemon control handle once ready.
    pub ready: Receiver<(Arc<AppState>, Handle)>,
}

/// Returns the daemon base directory.
pub fn base_dir() -> PathBuf {
    std::env::var_os("SIGILLUM_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".sigillum")
        })
}

/// Picks an immediately available loopback TCP port.
pub fn pick_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to bind an ephemeral loopback port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("failed to read loopback listener address: {error}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// Starts the embedded daemon on a background thread.
pub fn start_daemon(addr: SocketAddr, base_dir: PathBuf) -> Result<DaemonStart, String> {
    let (daemon_error_tx, daemon_error_rx) = mpsc::channel();
    let (daemon_ready_tx, daemon_ready_rx) = mpsc::channel();
    thread::Builder::new()
        .name("sigillum-daemon".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let message = format!("failed to create daemon runtime: {error}");
                    eprintln!("{message}");
                    let _ = daemon_error_tx.send(message);
                    return;
                }
            };

            if let Err(error) = runtime.block_on(sigillum_daemon::run_with_handle(
                addr,
                base_dir,
                DaemonRunOptions::default(),
                move |state, handle| {
                    let _ = daemon_ready_tx.send((state, handle));
                },
            )) {
                let message = error.to_string();
                eprintln!("Sigillum daemon error: {message}");
                let _ = daemon_error_tx.send(message);
            }
        })
        .map_err(|error| format!("failed to spawn daemon thread: {error}"))?;

    Ok(DaemonStart {
        errors: daemon_error_rx,
        ready: daemon_ready_rx,
    })
}

/// Waits until the daemon accepts TCP connections or reports an error.
pub fn wait_for_daemon(
    addr: SocketAddr,
    timeout: Duration,
    daemon_errors: &Receiver<String>,
) -> Result<(), String> {
    let started_at = Instant::now();
    loop {
        if let Ok(error) = daemon_errors.try_recv() {
            return Err(format!("daemon exited before readiness: {error}"));
        }

        if TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok() {
            return Ok(());
        }

        if started_at.elapsed() >= timeout {
            return Err(format!(
                "daemon did not accept TCP connections within {timeout:?}"
            ));
        }

        thread::sleep(CONNECT_POLL_INTERVAL);
    }
}

/// Returns the tray status label for the daemon lock state.
pub fn daemon_status_label(is_unlocked: bool) -> &'static str {
    if is_unlocked {
        "Sigillum: Unlocked"
    } else {
        "Sigillum: Locked"
    }
}

/// Schedules an asynchronous daemon lock request.
pub fn spawn_lock_now(control: &DaemonControl) {
    let state = control.state.clone();
    let handle = control.handle.clone();
    std::mem::drop(handle.spawn(async move {
        let _ = state.lock_now().await;
    }));
}

/// Schedules a daemon lock request and waits for completion up to the timeout.
pub fn lock_daemon_with_timeout(control: &DaemonControl, timeout: Duration) {
    let state = control.state.clone();
    let handle = control.handle.clone();
    let (tx, rx) = mpsc::channel();
    std::mem::drop(handle.spawn(async move {
        let _ = state.lock_now().await;
        let _ = tx.send(());
    }));
    let _ = rx.recv_timeout(timeout);
}

/// Builds the local daemon URL.
pub fn daemon_url(port: u16) -> Result<tauri::Url, String> {
    let url = format!("http://127.0.0.1:{port}/");
    url.parse()
        .map_err(|error| format!("failed to parse daemon URL {url:?}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_daemon_ok_when_listener_accepts() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (_tx, rx) = mpsc::channel();

        assert!(wait_for_daemon(addr, Duration::from_secs(2), &rx).is_ok());
    }

    #[test]
    fn wait_for_daemon_times_out_cleanly() {
        let port = pick_loopback_port().unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let (_tx, rx) = mpsc::channel();

        let error = wait_for_daemon(addr, Duration::from_millis(300), &rx).unwrap_err();

        assert!(error.contains("did not accept TCP connections"));
    }

    #[test]
    fn wait_for_daemon_surfaces_daemon_error() {
        let port = pick_loopback_port().unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let (tx, rx) = mpsc::channel();
        tx.send("boom".to_string()).unwrap();

        let error = wait_for_daemon(addr, Duration::from_millis(100), &rx).unwrap_err();

        assert!(error.contains("daemon exited before readiness: boom"));
    }

    #[test]
    fn daemon_url_round_trips_bound_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let url = daemon_url(port).unwrap();

        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.port(), Some(port));
        assert_eq!(url.path(), "/");
    }

    #[test]
    fn picked_port_is_immediately_bindable() {
        let port = pick_loopback_port().unwrap();

        assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn daemon_status_label_reports_lock_state() {
        assert_eq!(daemon_status_label(true), "Sigillum: Unlocked");
        assert_eq!(daemon_status_label(false), "Sigillum: Locked");
    }
}
