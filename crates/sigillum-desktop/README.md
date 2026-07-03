# Sigillum Desktop

`sigillum-desktop` is a Tauri v2 shell for the local Sigillum daemon UI.

At launch it claims a free ephemeral `127.0.0.1` port, starts
`sigillum-daemon` in-process on a background thread, waits for TCP readiness,
then opens the daemon UI in the runtime-created `main` webview window.

The single-instance plugin is registered first. Secondary launches are stopped
by the plugin and ask the existing `main` window to unminimize, show, and focus.

The native app menu includes app quit/about items, working Edit clipboard
shortcuts for webview input fields, Window controls, Reload, and debug-only
developer tools toggling.

Window state is persisted with `tauri-plugin-window-state`; first launch uses
the 1200x820 default size and the window cannot shrink below 940x640.
