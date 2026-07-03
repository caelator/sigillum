use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc,
    mpsc::{self, Receiver},
};
use std::thread;
use std::time::{Duration, Instant};

use sigillum_daemon::{AppState, DaemonRunOptions};
use tauri::Manager;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri_plugin_window_state::{StateFlags, WindowExt};
use tokio::runtime::Handle;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_READY_HANDLE_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const CONNECT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOCK_ON_QUIT_TIMEOUT: Duration = Duration::from_secs(3);
const TRAY_STATUS_INTERVAL: Duration = Duration::from_secs(2);

struct DaemonControl {
    state: Arc<AppState>,
    handle: Handle,
}

struct DaemonStart {
    errors: Receiver<String>,
    ready: Receiver<(Arc<AppState>, Handle)>,
}

struct TrayHandle {
    _tray: TrayIcon<tauri::Wry>,
}

fn main() {
    if let Err(error) = run_desktop() {
        eprintln!("Sigillum desktop error: {error}");
        std::process::exit(1);
    }
}

fn run_desktop() -> Result<(), String> {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all())
                .build(),
        )
        .on_menu_event(|app, event| match event.id() {
            id if id == "app_quit" => {
                if let Some(control) = app.try_state::<DaemonControl>() {
                    lock_daemon_with_timeout(control.inner(), LOCK_ON_QUIT_TIMEOUT);
                }
                app.exit(0);
            }
            id if id == "reload" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.reload();
                }
            }
            #[cfg(debug_assertions)]
            id if id == "toggle_devtools" => {
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_devtools_open() {
                        window.close_devtools();
                    } else {
                        window.open_devtools();
                    }
                }
            }
            _ => {}
        })
        .setup(setup_desktop)
        .run(tauri::generate_context!())
        .map_err(|error| format!("failed to run Tauri app: {error}"))?;

    Ok(())
}

fn setup_desktop(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = base_dir();
    let port = pick_loopback_port().map_err(setup_error)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let daemon = start_daemon(addr, base_dir).map_err(setup_error)?;

    wait_for_daemon(addr, STARTUP_TIMEOUT, &daemon.errors).map_err(setup_error)?;

    let (state, handle) = daemon
        .ready
        .recv_timeout(DAEMON_READY_HANDLE_TIMEOUT)
        .map_err(|error| {
            setup_error(format!("daemon control handle was not delivered: {error}"))
        })?;
    app.manage(DaemonControl { state, handle });

    let (tray, status_item) = build_tray(app)?;
    app.manage(TrayHandle { _tray: tray });
    start_tray_status_timer(app.handle().clone(), status_item);

    let daemon_url = format!("http://127.0.0.1:{port}/");
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "main",
        tauri::WebviewUrl::External(daemon_url.parse().expect("valid daemon URL")),
    )
    .title("Sigillum")
    .inner_size(1200.0, 820.0)
    .min_inner_size(940.0, 640.0)
    .build()?;

    install_privacy_close_handler(&window);
    window.restore_state(StateFlags::all())?;

    let menu = build_menu(app)?;
    app.set_menu(menu)?;

    Ok(())
}

fn build_menu<R: tauri::Runtime, M: tauri::Manager<R>>(manager: &M) -> tauri::Result<Menu<R>> {
    let about_item = PredefinedMenuItem::about(manager, Some("Sigillum"), None)?;
    let quit_item = MenuItem::with_id(manager, "app_quit", "Quit", true, Some("CmdOrControl+Q"))?;
    let app_menu = Submenu::with_items(manager, "Sigillum", true, &[&about_item, &quit_item])?;

    let edit_menu = Submenu::with_items(
        manager,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::cut(manager, None)?,
            &PredefinedMenuItem::copy(manager, None)?,
            &PredefinedMenuItem::paste(manager, None)?,
            &PredefinedMenuItem::select_all(manager, None)?,
        ],
    )?;

    let reload_item = MenuItem::with_id(manager, "reload", "Reload", true, Some("CmdOrControl+R"))?;

    #[cfg(debug_assertions)]
    let view_menu = {
        let toggle_devtools_item = MenuItem::with_id(
            manager,
            "toggle_devtools",
            "Toggle Developer Tools",
            true,
            Some("CmdOrControl+Shift+I"),
        )?;

        Submenu::with_items(
            manager,
            "View",
            true,
            &[&reload_item, &toggle_devtools_item],
        )?
    };

    #[cfg(not(debug_assertions))]
    let view_menu = Submenu::with_items(manager, "View", true, &[&reload_item])?;

    let window_menu = Submenu::with_items(
        manager,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(manager, None)?,
            &PredefinedMenuItem::fullscreen(manager, None)?,
            &PredefinedMenuItem::close_window(manager, None)?,
        ],
    )?;

    Menu::with_items(manager, &[&app_menu, &edit_menu, &view_menu, &window_menu])
}

fn build_tray(app: &tauri::App) -> tauri::Result<(TrayIcon<tauri::Wry>, MenuItem<tauri::Wry>)> {
    let control = app.state::<DaemonControl>();
    let status_item = MenuItem::with_id(
        app,
        "tray_status",
        daemon_status_label(control.state.is_unlocked()),
        false,
        None::<&str>,
    )?;
    let show_item = MenuItem::with_id(app, "tray_show", "Show Sigillum", true, None::<&str>)?;
    let lock_item = MenuItem::with_id(app, "tray_lock", "Lock now", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&status_item, &show_item, &lock_item, &separator, &quit_item],
    )?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Sigillum")
        .on_menu_event(|app, event| match event.id() {
            id if id == "tray_show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            id if id == "tray_lock" => {
                if let Some(control) = app.try_state::<DaemonControl>() {
                    spawn_lock_now(control.inner());
                }
            }
            id if id == "tray_quit" => {
                if let Some(control) = app.try_state::<DaemonControl>() {
                    lock_daemon_with_timeout(control.inner(), LOCK_ON_QUIT_TIMEOUT);
                }
                app.exit(0);
            }
            _ => {}
        });

    if let Some(icon) = tray_icon(app) {
        builder = builder.icon(icon);
    }

    let tray = builder.build(app)?;
    Ok((tray, status_item))
}

fn tray_icon(app: &tauri::App) -> Option<Image<'_>> {
    app.default_window_icon()
        .cloned()
        .or_else(|| Some(tauri::include_image!("icons/icon.png")))
}

fn start_tray_status_timer(app_handle: tauri::AppHandle, status_item: MenuItem<tauri::Wry>) {
    thread::spawn(move || {
        loop {
            thread::sleep(TRAY_STATUS_INTERVAL);
            let Some(control) = app_handle.try_state::<DaemonControl>() else {
                continue;
            };
            let label = daemon_status_label(control.state.is_unlocked());
            let _ = status_item.set_text(label);
        }
    });
}

fn install_privacy_close_handler(window: &tauri::WebviewWindow) {
    let close_window = window.clone();
    let app_handle = window.app_handle().clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = close_window.hide();
            if let Some(control) = app_handle.try_state::<DaemonControl>() {
                spawn_lock_now(control.inner());
            }
        }
    });
}

fn daemon_status_label(is_unlocked: bool) -> &'static str {
    if is_unlocked {
        "Sigillum: Unlocked"
    } else {
        "Sigillum: Locked"
    }
}

fn spawn_lock_now(control: &DaemonControl) {
    let state = control.state.clone();
    let handle = control.handle.clone();
    std::mem::drop(handle.spawn(async move {
        let _ = state.lock_now().await;
    }));
}

fn lock_daemon_with_timeout(control: &DaemonControl, timeout: Duration) {
    let state = control.state.clone();
    let handle = control.handle.clone();
    let (tx, rx) = mpsc::channel();
    std::mem::drop(handle.spawn(async move {
        let _ = state.lock_now().await;
        let _ = tx.send(());
    }));
    let _ = rx.recv_timeout(timeout);
}

fn setup_error(error: String) -> std::io::Error {
    std::io::Error::other(error)
}

fn base_dir() -> PathBuf {
    std::env::var_os("SIGILLUM_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".sigillum")
        })
}

fn pick_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to bind an ephemeral loopback port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("failed to read loopback listener address: {error}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn start_daemon(addr: SocketAddr, base_dir: PathBuf) -> Result<DaemonStart, String> {
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

fn wait_for_daemon(
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
