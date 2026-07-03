use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

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
