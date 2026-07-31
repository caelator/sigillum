use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sigillum_desktop::{
    DaemonControl, base_dir, daemon_status_label, daemon_url, lock_daemon_with_timeout,
    pick_loopback_port, spawn_lock_now, start_daemon, wait_for_daemon,
};
use tauri::Manager;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri_plugin_window_state::{StateFlags, WindowExt};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_READY_HANDLE_TIMEOUT: Duration = Duration::from_secs(3);
const LOCK_ON_QUIT_TIMEOUT: Duration = Duration::from_secs(3);
const TRAY_STATUS_INTERVAL: Duration = Duration::from_secs(2);
const STARTUP_LOG_RELATIVE_PATH: &str = "Library/Logs/Sigillum/desktop-startup.log";
const STARTUP_LOG_DISPLAY_PATH: &str = "~/Library/Logs/Sigillum/desktop-startup.log";

struct TrayHandle {
    _tray: TrayIcon<tauri::Wry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SafeStartupFailure {
    code: &'static str,
    detail: &'static str,
}

fn main() {
    match run_desktop() {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(error) => {
            handle_startup_failure(&error);
            std::process::exit(1);
        }
    }
}

fn handle_startup_failure(error: &str) {
    let failure = classify_startup_failure(error);
    let report_written = record_startup_failure(failure);
    let message = startup_failure_message(failure, report_written);

    eprintln!(
        "Sigillum desktop startup error [{}]: {}",
        failure.code, failure.detail
    );
    if !show_startup_failure_alert(&message) {
        eprintln!("{message}");
    }
}

fn classify_startup_failure(error: &str) -> SafeStartupFailure {
    if error.contains("failed to bind an ephemeral loopback port")
        || error.contains("failed to read loopback listener address")
    {
        SafeStartupFailure {
            code: "loopback-port",
            detail: "Sigillum could not reserve a local-only network port.",
        }
    } else if error.contains("failed to spawn daemon thread")
        || error.contains("failed to create daemon runtime")
    {
        SafeStartupFailure {
            code: "local-service-runtime",
            detail: "Sigillum could not start its local service runtime.",
        }
    } else if error.contains("daemon did not accept TCP connections") {
        SafeStartupFailure {
            code: "local-service-timeout",
            detail: "Sigillum's local service did not become ready in time.",
        }
    } else if error.contains("daemon exited before readiness") {
        SafeStartupFailure {
            code: "local-service-startup",
            detail: "Sigillum's local service stopped before it was ready.",
        }
    } else if error.contains("daemon control handle was not delivered") {
        SafeStartupFailure {
            code: "local-service-control",
            detail: "Sigillum's local service started without its control channel.",
        }
    } else {
        SafeStartupFailure {
            code: "desktop-runtime",
            detail: "Sigillum could not initialize its desktop window.",
        }
    }
}

fn startup_failure_message(failure: SafeStartupFailure, report_written: bool) -> String {
    let report_guidance = if report_written {
        format!("A safe startup report was written to:\n{STARTUP_LOG_DISPLAY_PATH}")
    } else {
        "Sigillum could not write its safe startup report.".to_string()
    };

    format!(
        "{}\n\nDiagnostic code: {}\n\nTry opening Sigillum again. If it still fails, \
         open Terminal and run:\n  sigillum doctor\n\n{}\n\nThe dialog and report omit raw \
         startup errors so wallet secrets, session tokens, and sensitive configuration are \
         not exposed.",
        failure.detail, failure.code, report_guidance
    )
}

fn startup_log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(STARTUP_LOG_RELATIVE_PATH))
}

fn record_startup_failure(failure: SafeStartupFailure) -> bool {
    let Some(path) = startup_log_path() else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };

    if fs::create_dir_all(parent).is_err() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).is_err() {
            return false;
        }
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }

    let Ok(mut file) = options.open(&path) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .is_err()
        {
            return false;
        }
    }

    let unix_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    writeln!(
        file,
        "unix_time={unix_time} code={} detail={} next_action=run_sigillum_doctor",
        failure.code, failure.detail
    )
    .is_ok()
}

#[cfg(target_os = "macos")]
fn show_startup_failure_alert(message: &str) -> bool {
    use objc2::MainThreadMarker;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    autoreleasepool(|_| {
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        let title = NSString::from_str("Sigillum couldn’t start");
        let information = NSString::from_str(message);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&title);
        alert.setInformativeText(&information);
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.runModal();
    });

    true
}

#[cfg(not(target_os = "macos"))]
fn show_startup_failure_alert(_message: &str) -> bool {
    false
}

fn run_desktop() -> Result<i32, String> {
    let app = tauri::Builder::default()
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
        .setup(|app| {
            if let Err(error) = setup_desktop(app) {
                // Tauri panics if the setup hook returns `Err`. Surface the failure
                // while AppKit is still alive. Exit requests issued before the
                // event loop starts are dropped, so terminate only after the
                // operator dismisses the modal.
                handle_startup_failure(&error.to_string());
                std::process::exit(1);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|error| format!("failed to build Tauri app: {error}"))?;

    Ok(app.run_return(|_app_handle, _event| {}))
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

    let daemon_url = daemon_url(port).map_err(setup_error)?;
    let window =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(daemon_url))
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

fn setup_error(error: String) -> std::io::Error {
    std::io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_message_never_includes_raw_error() {
        let raw_error =
            "daemon exited before readiness: seed=abandon session_token=super-secret-token";

        let failure = classify_startup_failure(raw_error);
        let message = startup_failure_message(failure, true);

        assert_eq!(failure.code, "local-service-startup");
        assert!(!message.contains("abandon"));
        assert!(!message.contains("super-secret-token"));
        assert!(message.contains("sigillum doctor"));
        assert!(message.contains(STARTUP_LOG_DISPLAY_PATH));
    }

    #[test]
    fn unknown_startup_failure_gets_safe_generic_detail() {
        let raw_error = "webview failed with credential=hunter2";

        let failure = classify_startup_failure(raw_error);
        let message = startup_failure_message(failure, false);

        assert_eq!(failure.code, "desktop-runtime");
        assert!(!message.contains("hunter2"));
        assert!(message.contains("could not write its safe startup report"));
    }
}
