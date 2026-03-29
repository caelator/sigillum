//! Embedded single-page web UI for Sigillum vault management.
//!
//! The entire interface — HTML, CSS, and JavaScript — is compiled into the
//! daemon binary as three `const &str` segments joined at runtime by
//! [`render_index_html`] with a per-request CSP nonce.
//!
//! ## Architecture
//!
//! The UI is intentionally zero-dependency: no build step, no bundler, no
//! framework. This guarantees the daemon remains a single static binary
//! that works air-gapped. The trade-off is a large source file, mitigated
//! by clear sectioning.
//!
//! ### CSS design system
//!
//! All colours, radii, and fonts are declared as CSS custom properties on
//! `:root`. Every visual component (cards, badges, pills, buttons, entity
//! lists) references these tokens, ensuring a single place to change the
//! palette or spacing scale.
//!
//! ### JavaScript architecture
//!
//! - **`api(method, path, body)`** — central fetch wrapper that attaches the
//!   session token and auto-clears it on 401.
//! - **`refresh()`** — visibility-aware polling controller that prevents
//!   overlapping refreshes, tracks sync status, and delegates real work to
//!   `runRefreshCycle()`.
//! - **`runRefreshCycle()`** — master state-sync: calls `/api/status`, toggles
//!   card visibility, and fans out to `load*()` functions in parallel.
//! - **`render*(list)`** — pure render functions that produce HTML from typed
//!   arrays and inject it into the DOM.
//! - **Setup wizard** — a multi-step flow (`wizStep0` → `wizStepDone`) that
//!   guides first-time users through preset selection, compartment definition,
//!   and FIDO2 or passphrase initialization.
//!
//! ### Security considerations
//!
//! - The main script tag is injected with a per-request `nonce` for CSP compliance.
//! - Interactive controls use delegated `data-action="..."` handlers so the
//!   daemon UI does not need to allow inline script attributes in CSP.
//! - All user-visible strings pass through `esc()` / `escAttr()` to prevent XSS.
//! - Session tokens use `sessionStorage` (not `localStorage`) so they do not
//!   persist across browser sessions.
//! - Revealed secret values auto-hide after 30 seconds.

/// Render the complete HTML page with a CSP nonce injected into the script tag.
pub(crate) fn render_index_html(nonce: &str) -> String {
    format!(
        "{INDEX_HTML_SHELL_BEFORE_SCRIPT}<script nonce=\"{nonce}\">{INDEX_HTML_SCRIPT}</script>{INDEX_HTML_SHELL_AFTER_SCRIPT}",
    )
}

const INDEX_HTML_SHELL_BEFORE_SCRIPT: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Sigillum Vault</title>
<style>
  :root {
    --bg: #0a0a0f;
    --surface: #12121a;
    --border: #1e1e2e;
    --text: #e0e0e8;
    --text-dim: #6e6e8a;
    --accent: #7c6ff0;
    --accent-hover: #9488f5;
    --danger: #e5484d;
    --success: #46a758;
    --warning: #f5a623;
    --radius: 8px;
    --font: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
    --mono: 'SF Mono', 'Cascadia Code', 'JetBrains Mono', monospace;
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: var(--font);
    background: var(--bg);
    color: var(--text);
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  header {
    width: 100%;
    padding: 24px 32px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .logo {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.05em;
  }
  .logo span { color: var(--accent); }
  .header-right { display: flex; align-items: center; gap: 12px; }
  .refresh-meta {
    padding: 6px 12px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    border: 1px solid var(--border);
    color: var(--text-dim);
    background: rgba(18,18,26,0.9);
  }
  .refresh-meta[data-state="busy"] {
    color: var(--warning);
    border-color: rgba(245,166,35,0.35);
  }
  .refresh-meta[data-state="live"] {
    color: var(--success);
    border-color: rgba(70,167,88,0.3);
  }
  .refresh-meta[data-state="paused"] {
    color: #a9a9bf;
    border-color: rgba(110,110,138,0.28);
  }
  .refresh-meta[data-state="error"] {
    color: var(--danger);
    border-color: rgba(229,72,77,0.32);
  }
  .compartment-badge {
    padding: 6px 14px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 600;
    background: rgba(124,111,240,0.15);
    color: var(--accent);
    letter-spacing: 0.04em;
  }
  .status-badge {
    padding: 6px 14px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .status-locked { background: rgba(229,72,77,0.15); color: var(--danger); }
  .status-unlocked { background: rgba(70,167,88,0.15); color: var(--success); }
  .status-no-vault { background: rgba(245,166,35,0.15); color: var(--warning); }
  main {
    width: 100%;
    max-width: 1080px;
    padding: 32px 24px;
    flex: 1;
  }
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 24px;
    margin-bottom: 20px;
    scroll-margin-top: 110px;
  }
  .card h2 {
    font-size: 14px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin-bottom: 16px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: 12px;
  }
  .stat {
    text-align: center;
    padding: 16px;
    background: var(--bg);
    border-radius: var(--radius);
  }
  .stat .value {
    font-size: 28px;
    font-weight: 700;
    font-family: var(--mono);
  }
  .stat .label {
    font-size: 11px;
    color: var(--text-dim);
    margin-top: 4px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .form-row {
    display: flex;
    gap: 10px;
    margin-bottom: 12px;
  }
  input[type="text"], input[type="password"], input[type="number"], select {
    flex: 1;
    padding: 10px 14px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-family: var(--mono);
    font-size: 14px;
    outline: none;
    transition: border-color 0.15s;
  }
  input:focus, select:focus { border-color: var(--accent); }
  button {
    padding: 10px 20px;
    border: none;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }
  .btn-primary { background: var(--accent); color: #fff; }
  .btn-primary:hover { background: var(--accent-hover); }
  .btn-danger { background: rgba(229,72,77,0.15); color: var(--danger); }
  .btn-danger:hover { background: rgba(229,72,77,0.25); }
  .btn-ghost {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .btn-ghost:hover { border-color: var(--text-dim); color: var(--text); }
  .btn-success { background: rgba(70,167,88,0.15); color: var(--success); }
  .btn-success:hover { background: rgba(70,167,88,0.25); }
  .key-list { list-style: none; }
  .key-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    font-family: var(--mono);
    font-size: 13px;
  }
  .key-list li:last-child { border-bottom: none; }
  .key-actions { display: flex; gap: 6px; }
  .key-actions button {
    padding: 4px 10px;
    font-size: 11px;
  }
  .toast {
    position: fixed;
    bottom: 24px;
    right: 24px;
    padding: 12px 20px;
    border-radius: var(--radius);
    font-size: 13px;
    font-weight: 500;
    z-index: 100;
    animation: fadeIn 0.2s ease;
  }
  .toast-success { background: rgba(70,167,88,0.9); color: #fff; }
  .toast-error { background: rgba(229,72,77,0.9); color: #fff; }
  @keyframes fadeIn { from { opacity: 0; transform: translateY(8px); } }
  .hidden { display: none; }
  .secret-value {
    font-family: var(--mono);
    font-size: 13px;
    padding: 8px 12px;
    background: var(--bg);
    border-radius: var(--radius);
    margin-top: 8px;
    word-break: break-all;
  }
  footer {
    padding: 16px;
    text-align: center;
    font-size: 11px;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
    width: 100%;
  }
  footer a { color: var(--accent); text-decoration: none; }
  .wizard-step { display: none; }
  .wizard-step.active { display: block; }
  .wizard-step p {
    color: var(--text-dim);
    font-size: 13px;
    margin-bottom: 14px;
    line-height: 1.6;
  }
  .method-btn {
    display: block;
    width: 100%;
    padding: 16px 20px;
    margin-bottom: 10px;
    text-align: left;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    cursor: pointer;
    transition: border-color 0.15s;
  }
  .method-btn:hover { border-color: var(--accent); }
  .method-btn .method-title {
    font-weight: 600;
    font-size: 14px;
    margin-bottom: 4px;
  }
  .method-btn .method-desc {
    font-size: 12px;
    color: var(--text-dim);
  }
  .method-btn.recommended { border-color: var(--accent); }
  .method-btn.recommended .method-title::after {
    content: ' (recommended)';
    color: var(--accent);
    font-weight: 400;
    font-size: 12px;
  }
  .wiz-comp-row {
    padding: 8px 14px;
    margin-bottom: 6px;
    background: var(--bg);
    border-radius: var(--radius);
    font-size: 13px;
    display: flex;
    justify-content: space-between;
  }
  .wiz-comp-label { color: var(--text); font-weight: 500; }
  .wiz-comp-threshold { color: var(--accent); font-family: var(--mono); }
  .input-narrow { flex: none; width: 100px; }
  .input-compact { flex: none; width: 120px; }
  .input-mid { flex: none; width: 150px; }
  .input-wide { flex: none; width: 180px; }
  .input-wider { flex: none; width: 190px; }
  .text-meta { color: var(--text-dim); font-size: 13px; }
  .text-meta-sm { color: var(--text-dim); font-size: 12px; }
  .poison-warning {
    padding: 10px 14px;
    background: rgba(229,72,77,0.08);
    border: 1px solid rgba(229,72,77,0.2);
    border-radius: var(--radius);
    font-size: 12px;
    color: var(--danger);
    margin-top: 6px;
  }
  .wiz-center {
    text-align: center;
    padding: 24px 0;
  }
  .pin-modal {
    max-width: 340px;
    margin: 0 16px;
  }
  .pin-modal h2 {
    font-size: 14px;
    margin-bottom: 12px;
  }
  .pulse { animation: pulse 1.5s infinite; }
  @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }
  .info-box {
    padding: 12px 16px;
    background: rgba(124,111,240,0.08);
    border: 1px solid rgba(124,111,240,0.2);
    border-radius: var(--radius);
    font-size: 13px;
    color: var(--text-dim);
    margin-bottom: 14px;
  }
  .unlock-tabs {
    display: flex;
    gap: 8px;
    margin-bottom: 14px;
  }
  .unlock-tab {
    padding: 8px 16px;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text-dim);
    transition: all 0.15s;
  }
  .unlock-tab.active {
    border-color: var(--accent);
    color: var(--accent);
    background: rgba(124,111,240,0.08);
  }
  .comp-switcher {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .comp-switcher button {
    padding: 8px 16px;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text-dim);
    transition: all 0.15s;
  }
  .comp-switcher button.active {
    border-color: var(--accent);
    color: var(--accent);
    background: rgba(124,111,240,0.08);
  }
  .comp-switcher button:hover { border-color: var(--accent); }
  .section-title {
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin-bottom: 10px;
  }
  .section-divider {
    height: 1px;
    background: var(--border);
    margin: 20px 0;
  }
  .helper-text {
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.6;
    margin-bottom: 12px;
  }
  .entity-list { list-style: none; }
  .entity-list li {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
    padding: 14px;
    border-bottom: 1px solid var(--border);
  }
  .entity-list li:last-child { border-bottom: none; }
  .entity-main {
    min-width: 0;
    flex: 1;
  }
  .entity-title {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 600;
    word-break: break-all;
  }
  .entity-meta {
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.6;
    margin-top: 6px;
    word-break: break-all;
  }
  .entity-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
    flex-shrink: 0;
  }
  .entity-actions button {
    padding: 6px 10px;
    font-size: 11px;
  }
  .checkbox-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .checkbox-row input { flex: none; }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .form-row-center { display: flex; gap: 10px; margin-bottom: 12px; align-items: center; }
  .mono-line {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text);
  }
  .result-box {
    padding: 12px 14px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    font-size: 12px;
    line-height: 1.6;
    margin-bottom: 14px;
  }
  .pill {
    display: inline-block;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-family: var(--mono);
  }
  .pill-good {
    background: rgba(70,167,88,0.15);
    color: var(--success);
  }
  .pill-warn {
    background: rgba(245,166,35,0.15);
    color: var(--warning);
  }
  .pill-danger {
    background: rgba(229,72,77,0.15);
    color: var(--danger);
  }
  .pill-neutral {
    background: rgba(110,110,138,0.18);
    color: #a9a9bf;
  }
  .section-nav {
    position: sticky;
    top: 16px;
    z-index: 5;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 20px;
    padding: 12px;
    background: rgba(18,18,26,0.92);
    border: 1px solid var(--border);
    border-radius: calc(var(--radius) + 4px);
    backdrop-filter: blur(12px);
  }
  .section-nav a {
    padding: 8px 12px;
    background: rgba(10,10,15,0.95);
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-dim);
    text-decoration: none;
    font-size: 12px;
    font-weight: 600;
    transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .section-nav a:hover,
  .section-nav a:focus {
    border-color: var(--accent);
    color: var(--accent);
    background: rgba(124,111,240,0.08);
    outline: none;
  }
  .section-nav a.active {
    border-color: var(--accent);
    color: #f4f3ff;
    background: rgba(124,111,240,0.18);
  }
  /* 2026 UI refresh: guided local-control layout layered over the existing
     zero-build markup so behavior stays daemon-compatible while the page
     becomes much clearer for first-time operators. */
  :root {
    --bg: #07111a;
    --surface: #0f1824;
    --surface-strong: #111f2f;
    --surface-soft: rgba(255,255,255,0.035);
    --border: rgba(140, 181, 214, 0.14);
    --border-strong: rgba(140, 181, 214, 0.26);
    --text: #e8eff8;
    --text-dim: #95a6bb;
    --accent: #4fd1c5;
    --accent-hover: #74dfd5;
    --accent-warm: #ffbf69;
    --danger: #ff7b7b;
    --success: #47d9a0;
    --warning: #ffbf69;
    --radius: 18px;
    --font: "Avenir Next", "Segoe UI", "Helvetica Neue", system-ui, sans-serif;
    --mono: "IBM Plex Mono", "SF Mono", "JetBrains Mono", monospace;
  }
  body {
    background:
      radial-gradient(circle at top left, rgba(79, 209, 197, 0.12), transparent 34%),
      radial-gradient(circle at top right, rgba(255, 191, 105, 0.12), transparent 28%),
      linear-gradient(180deg, #07111a 0%, #09131d 46%, #08101a 100%);
    color: var(--text);
    align-items: stretch;
  }
  header {
    max-width: 1320px;
    margin: 0 auto;
    padding: 28px 28px 14px;
    border-bottom: none;
    gap: 18px;
  }
  .brand-lockup {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }
  .brand-mark {
    width: 44px;
    height: 44px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    font-size: 18px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: #061119;
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-warm) 100%);
    box-shadow: 0 16px 32px rgba(0, 0, 0, 0.22);
  }
  .logo {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .logo span { color: var(--accent); }
  .logo-sub {
    margin-top: 5px;
    max-width: 760px;
    font-size: 14px;
    line-height: 1.55;
    color: var(--text-dim);
  }
  .header-right {
    gap: 10px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .refresh-meta,
  .compartment-badge,
  .status-badge {
    min-height: 38px;
    display: inline-flex;
    align-items: center;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: rgba(8, 14, 22, 0.78);
    backdrop-filter: blur(16px);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.04);
  }
  .refresh-meta {
    padding: 6px 14px;
    font-size: 11px;
    letter-spacing: 0.08em;
  }
  .compartment-badge {
    padding: 6px 14px;
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .status-badge {
    padding: 6px 16px;
    font-size: 11px;
    letter-spacing: 0.1em;
  }
  main {
    max-width: 1320px;
    margin: 0 auto;
    padding: 6px 28px 56px;
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 20px;
  }
  footer {
    max-width: 1320px;
    margin: 0 auto;
    width: 100%;
    padding: 0 28px 38px;
    border-top: none;
    text-align: left;
    color: var(--text-dim);
  }
  #sectionNav,
  #statusCard,
  #guideCard,
  #nextStepCard,
  #authCard,
  #setupCard,
  #profilesCard,
  #xpubCard,
  #depositsCard,
  #queueCard,
  #maintenanceCard,
  #backupCard,
  #auditCard,
  #diagCard {
    grid-column: 1 / -1;
  }
  #compartmentCard,
  #pushCard,
  #apiKeysCard,
  #secretsCard,
  #fido2Card {
    grid-column: span 6;
  }
  .card {
    position: relative;
    overflow: hidden;
    padding: 26px;
    margin-bottom: 0;
    border: 1px solid var(--border);
    border-radius: 24px;
    background:
      linear-gradient(180deg, rgba(255,255,255,0.025) 0%, rgba(255,255,255,0.01) 100%),
      linear-gradient(180deg, rgba(7,11,17,0.2) 0%, rgba(7,11,17,0) 50%),
      var(--surface);
    box-shadow:
      0 24px 60px rgba(0, 0, 0, 0.22),
      inset 0 1px 0 rgba(255,255,255,0.04);
    scroll-margin-top: 112px;
  }
  .card::before {
    content: "";
    position: absolute;
    inset: 0 auto auto 0;
    width: 100%;
    height: 1px;
    background: linear-gradient(90deg, rgba(79,209,197,0.28), rgba(255,191,105,0.18), transparent);
  }
  .section-hidden {
    display: none !important;
  }
  .section-nav {
    position: sticky;
    top: 16px;
    z-index: 5;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 10px;
    margin-bottom: 0;
    padding: 12px;
    background: rgba(9, 16, 26, 0.86);
    border: 1px solid var(--border);
    border-radius: 20px;
    backdrop-filter: blur(16px);
    box-shadow: 0 18px 40px rgba(0, 0, 0, 0.2);
  }
  .workspace-tab {
    width: 100%;
    padding: 12px 14px;
    border: 1px solid transparent;
    border-radius: 16px;
    background: rgba(255,255,255,0.03);
    color: var(--text-dim);
    text-align: left;
  }
  .workspace-tab strong,
  .workspace-tab span {
    display: block;
  }
  .workspace-tab strong {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.03em;
    color: var(--text);
  }
  .workspace-tab span {
    margin-top: 4px;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-dim);
  }
  .workspace-tab:hover,
  .workspace-tab:focus-visible {
    border-color: var(--border-strong);
    background: rgba(255,255,255,0.06);
    outline: none;
  }
  .workspace-tab.active {
    border-color: rgba(79, 209, 197, 0.36);
    background:
      linear-gradient(180deg, rgba(79,209,197,0.15) 0%, rgba(255,191,105,0.08) 100%),
      rgba(255,255,255,0.04);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.05);
  }
  .guide-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 14px;
    margin-top: 16px;
  }
  .guide-block {
    padding: 16px 18px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background: var(--surface-soft);
  }
  .guide-block-title {
    margin-bottom: 8px;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .guide-block p {
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-dim);
  }
  .guide-list {
    margin: 0;
    padding-left: 18px;
    color: var(--text-dim);
  }
  .guide-list li {
    margin-top: 8px;
    line-height: 1.55;
  }
  .card h2 {
    margin-bottom: 10px;
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    text-transform: none;
    color: var(--text);
  }
  .eyebrow {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .hero-shell {
    display: grid;
    grid-template-columns: minmax(0, 1.5fr) minmax(310px, 0.9fr);
    gap: 26px;
    align-items: stretch;
  }
  .hero-copy h1 {
    font-size: clamp(32px, 4vw, 50px);
    line-height: 0.98;
    letter-spacing: -0.04em;
    margin: 0;
  }
  .hero-summary {
    max-width: 760px;
    margin-top: 14px;
    font-size: 16px;
    line-height: 1.65;
    color: var(--text-dim);
  }
  .hero-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 20px;
  }
  .hero-context {
    margin-top: 22px;
    display: grid;
    gap: 10px;
  }
  .hero-context .context-row {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 12px 14px;
    border-radius: 16px;
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.04);
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.5;
  }
  .hero-context .context-row strong {
    color: var(--text);
    font-weight: 600;
  }
  .hero-mode {
    padding: 18px;
    border-radius: 18px;
    background:
      radial-gradient(circle at top right, rgba(79,209,197,0.12), transparent 46%),
      rgba(255,255,255,0.03);
    border: 1px solid var(--border);
  }
  .hero-mode-kicker {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .hero-mode-value {
    margin-top: 8px;
    font-size: 24px;
    line-height: 1.1;
    font-weight: 700;
  }
  .hero-mode-detail {
    margin-top: 10px;
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.55;
  }
  .hero-stats {
    margin-top: 14px;
    gap: 12px;
  }
  .stat {
    min-height: 112px;
    padding: 18px 16px;
    border-radius: 18px;
    border: 1px solid rgba(255,255,255,0.045);
    background:
      linear-gradient(180deg, rgba(255,255,255,0.03) 0%, rgba(255,255,255,0.015) 100%),
      rgba(6, 11, 17, 0.32);
  }
  .stat .value {
    font-size: 32px;
    letter-spacing: -0.04em;
  }
  .stat .label {
    margin-top: 8px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .section-nav {
    top: 12px;
    padding: 12px;
    gap: 10px;
    border: 1px solid var(--border);
    background: rgba(8, 13, 20, 0.74);
    backdrop-filter: blur(18px);
    box-shadow: 0 18px 40px rgba(0,0,0,0.16);
  }
  .section-nav a {
    padding: 9px 14px;
    border-radius: 999px;
    color: var(--text-dim);
    background: rgba(255,255,255,0.025);
    border-color: rgba(255,255,255,0.05);
  }
  .section-nav a:hover,
  .section-nav a:focus {
    color: var(--text);
    border-color: rgba(79,209,197,0.35);
    background: rgba(79,209,197,0.12);
  }
  .section-nav a.active {
    color: #02141a;
    border-color: transparent;
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-warm) 130%);
  }
  .form-row,
  .form-row-center {
    flex-wrap: wrap;
    gap: 12px;
  }
  input[type="text"], input[type="password"], input[type="number"], select {
    min-height: 52px;
    padding: 12px 16px;
    border-radius: 16px;
    border: 1px solid rgba(148, 173, 196, 0.14);
    background: rgba(7, 13, 21, 0.72);
    color: var(--text);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.03);
  }
  input::placeholder { color: #76879d; }
  input:focus, select:focus {
    border-color: rgba(79,209,197,0.42);
    box-shadow: 0 0 0 3px rgba(79,209,197,0.12);
  }
  button {
    min-height: 52px;
    padding: 12px 18px;
    border-radius: 16px;
    font-size: 14px;
    font-weight: 700;
    letter-spacing: -0.01em;
    box-shadow: 0 10px 24px rgba(0,0,0,0.12);
  }
  .btn-primary {
    color: #04141b;
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-warm) 140%);
  }
  .btn-primary:hover { background: linear-gradient(135deg, var(--accent-hover) 0%, #ffd083 140%); }
  .btn-ghost {
    color: var(--text);
    border: 1px solid rgba(148, 173, 196, 0.16);
    background: rgba(255,255,255,0.025);
  }
  .btn-ghost:hover {
    border-color: rgba(79,209,197,0.32);
    color: var(--text);
    background: rgba(79,209,197,0.08);
  }
  .btn-danger {
    border: 1px solid rgba(255,123,123,0.18);
    background: rgba(255,123,123,0.09);
    color: var(--danger);
  }
  .btn-success {
    border: 1px solid rgba(71,217,160,0.2);
    background: rgba(71,217,160,0.1);
    color: var(--success);
  }
  .helper-text,
  .text-meta,
  .text-meta-sm,
  .wizard-step p {
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.65;
  }
  .info-box,
  .result-box {
    border-radius: 18px;
    border: 1px solid var(--border);
    background: rgba(255,255,255,0.03);
  }
  .wizard-header {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(280px, 0.8fr);
    gap: 22px;
    align-items: start;
    margin-bottom: 22px;
  }
  .wizard-summary {
    padding: 18px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background:
      radial-gradient(circle at top right, rgba(255,191,105,0.12), transparent 44%),
      rgba(255,255,255,0.03);
  }
  .progress-pill {
    display: inline-flex;
    align-items: center;
    padding: 7px 12px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: #05141b;
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-warm) 140%);
  }
  .wizard-stage-title {
    margin-top: 12px;
    font-size: 24px;
    line-height: 1.1;
    font-weight: 700;
  }
  .wizard-stage-summary {
    margin-top: 10px;
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.6;
  }
  .summary-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 14px;
  }
  .checklist-list {
    margin-top: 16px;
    display: grid;
    gap: 10px;
  }
  .checklist-item {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 12px 14px;
    border-radius: 16px;
    border: 1px solid rgba(255,255,255,0.05);
    background: rgba(7,13,21,0.48);
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.55;
  }
  .checklist-mark {
    width: 22px;
    height: 22px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 800;
    color: #04141b;
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-warm) 140%);
  }
  .method-btn {
    margin-bottom: 12px;
    padding: 18px 20px;
    border-radius: 18px;
    border: 1px solid rgba(255,255,255,0.05);
    background:
      linear-gradient(180deg, rgba(255,255,255,0.03) 0%, rgba(255,255,255,0.015) 100%),
      rgba(7, 13, 21, 0.54);
  }
  .method-btn:hover { border-color: rgba(79,209,197,0.34); }
  .method-btn .method-title {
    font-size: 18px;
    font-weight: 700;
    margin-bottom: 6px;
  }
  .method-btn .method-desc {
    font-size: 14px;
    color: var(--text-dim);
    line-height: 1.55;
  }
  .method-btn.recommended {
    border-color: rgba(79,209,197,0.34);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.04);
  }
  .method-btn.recommended .method-title::after {
    color: var(--accent);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    font-weight: 700;
  }
  .wiz-comp-row {
    padding: 12px 14px;
    margin-bottom: 10px;
    border-radius: 16px;
    border: 1px solid rgba(255,255,255,0.05);
    background: rgba(7,13,21,0.52);
    font-size: 14px;
  }
  .section-title {
    margin-bottom: 12px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--accent);
  }
  .section-divider {
    margin: 24px 0;
    background: linear-gradient(90deg, rgba(79,209,197,0.24), rgba(255,255,255,0.05), transparent);
  }
  .key-list,
  .entity-list {
    display: grid;
    gap: 10px;
  }
  .key-list li,
  .entity-list li {
    padding: 15px 16px;
    border: 1px solid rgba(255,255,255,0.05);
    border-radius: 18px;
    background: rgba(7,13,21,0.48);
  }
  .key-list li:last-child,
  .entity-list li:last-child { border-bottom: 1px solid rgba(255,255,255,0.05); }
  .toast {
    right: 22px;
    bottom: 22px;
    padding: 12px 16px;
    border-radius: 16px;
    border: 1px solid rgba(255,255,255,0.06);
    box-shadow: 0 18px 32px rgba(0,0,0,0.26);
  }
  .pin-modal {
    background: var(--surface);
  }
  .next-step-grid {
    margin-top: 18px;
    display: grid;
    gap: 10px;
  }
  .next-step-item {
    padding: 14px 16px;
    border-radius: 18px;
    border: 1px solid rgba(255,255,255,0.05);
    background: rgba(7,13,21,0.48);
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.6;
  }
  .next-step-item strong {
    display: block;
    margin-bottom: 4px;
    color: var(--text);
    font-weight: 700;
  }
  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 18px;
  }
  .card-note {
    margin-top: 12px;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.6;
  }
  @media (max-width: 1100px) {
    main {
      grid-template-columns: repeat(6, minmax(0, 1fr));
    }
    #compartmentCard,
    #pushCard,
    #apiKeysCard,
    #secretsCard,
    #fido2Card {
      grid-column: span 6;
    }
    .hero-shell,
    .wizard-header {
      grid-template-columns: 1fr;
    }
  }
  @media (max-width: 760px) {
    header {
      padding: 22px 18px 10px;
      flex-direction: column;
      align-items: stretch;
    }
    main {
      padding: 6px 18px 46px;
      grid-template-columns: 1fr;
    }
    #sectionNav,
    #statusCard,
    #authCard,
    #setupCard,
    #compartmentCard,
    #pushCard,
    #apiKeysCard,
    #secretsCard,
    #profilesCard,
    #depositsCard,
    #queueCard,
    #maintenanceCard,
    #fido2Card,
    #backupCard,
    #auditCard,
    #diagCard {
      grid-column: 1 / -1;
    }
    .hero-copy h1 {
      font-size: 32px;
    }
    footer {
      padding: 0 18px 30px;
    }
  }
  :root {
    color-scheme: dark;
    --bg: #0c1014;
    --bg-deep: #07090c;
    --surface: rgba(20, 25, 31, 0.9);
    --surface-strong: rgba(25, 31, 38, 0.96);
    --surface-soft: rgba(255, 255, 255, 0.028);
    --surface-quiet: rgba(255, 255, 255, 0.04);
    --border: rgba(244, 248, 252, 0.08);
    --border-strong: rgba(244, 248, 252, 0.14);
    --text: #eef3f7;
    --text-dim: #a4b0bb;
    --text-soft: #6f7b86;
    --accent: #d8e3ed;
    --accent-hover: #eef4fa;
    --accent-strong: #f8fbfe;
    --accent-soft: rgba(216, 227, 237, 0.12);
    --accent-alt: #8da8c1;
    --accent-alt-soft: rgba(141, 168, 193, 0.12);
    --danger: #ff8d7f;
    --success: #7fc5a3;
    --warning: #d7b079;
    --radius: 24px;
    --radius-sm: 16px;
    --radius-lg: 32px;
    --font: "Avenir Next", "Segoe UI Variable Display", "Segoe UI", "Helvetica Neue", sans-serif;
    --font-body: "Avenir Next", "Segoe UI Variable Text", "Segoe UI", "Helvetica Neue", sans-serif;
    --mono: "JetBrains Mono", "SFMono-Regular", "SF Mono", "Cascadia Code", monospace;
  }
  body {
    font-family: var(--font-body);
    background:
      radial-gradient(circle at top center, rgba(255, 255, 255, 0.065), transparent 30%),
      radial-gradient(circle at 18% 18%, rgba(141, 168, 193, 0.08), transparent 20%),
      linear-gradient(180deg, #11161c 0%, #090c10 100%);
    color: var(--text);
    position: relative;
    overflow-x: hidden;
  }
  body::before {
    content: "";
    position: fixed;
    inset: 0;
    pointer-events: none;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.035), transparent 22%),
      linear-gradient(90deg, transparent 0%, rgba(255, 255, 255, 0.015) 50%, transparent 100%);
    opacity: 0.2;
  }
  body::after {
    content: "";
    position: fixed;
    inset: 0;
    pointer-events: none;
    background:
      radial-gradient(circle at 80% 14%, rgba(216, 227, 237, 0.08), transparent 18%),
      radial-gradient(circle at 60% 82%, rgba(141, 168, 193, 0.06), transparent 18%);
    mix-blend-mode: normal;
    opacity: 0.26;
  }
  header {
    position: sticky;
    top: 0;
    z-index: 50;
    padding: 16px clamp(18px, 3vw, 34px);
    gap: 18px;
    background: rgba(9, 12, 16, 0.72);
    backdrop-filter: blur(14px);
    border-bottom: 1px solid rgba(244, 248, 252, 0.06);
    box-shadow: 0 12px 24px rgba(0, 0, 0, 0.14);
  }
  .brand-lockup {
    display: flex;
    align-items: center;
    gap: 16px;
    min-width: 0;
  }
  .brand-mark {
    width: 46px;
    height: 46px;
    display: grid;
    place-items: center;
    border-radius: 14px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.07), transparent),
      rgba(18, 23, 29, 0.94);
    border: 1px solid rgba(244, 248, 252, 0.08);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.06),
      0 16px 28px rgba(0, 0, 0, 0.28);
    font-family: var(--font);
    font-size: 17px;
    font-weight: 700;
    color: var(--accent);
    letter-spacing: -0.03em;
  }
  .logo {
    font-family: var(--font);
    font-size: clamp(20px, 2vw, 28px);
    font-weight: 700;
    letter-spacing: -0.05em;
    color: var(--text);
  }
  .logo span {
    color: var(--accent-strong);
  }
  .logo-sub {
    max-width: 640px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.55;
    margin-top: 4px;
  }
  .header-right {
    justify-content: flex-end;
    gap: 10px;
    flex-wrap: wrap;
  }
  .refresh-meta,
  .compartment-badge,
  .status-badge {
    min-height: 44px;
    padding: 0 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: rgba(18, 23, 29, 0.82);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.03);
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  .refresh-meta {
    color: var(--text-dim);
  }
  .refresh-meta[data-state="busy"] {
    color: var(--warning);
    border-color: rgba(255, 179, 71, 0.3);
    background: rgba(255, 179, 71, 0.1);
  }
  .refresh-meta[data-state="live"] {
    color: var(--success);
    border-color: rgba(50, 213, 154, 0.28);
    background: rgba(50, 213, 154, 0.1);
  }
  .refresh-meta[data-state="paused"] {
    color: var(--text-soft);
    border-color: rgba(157, 184, 214, 0.18);
  }
  .refresh-meta[data-state="error"] {
    color: var(--danger);
    border-color: rgba(255, 120, 120, 0.3);
    background: rgba(255, 120, 120, 0.1);
  }
  .compartment-badge {
    color: var(--accent-strong);
    background: rgba(88, 166, 255, 0.12);
    border-color: rgba(88, 166, 255, 0.26);
  }
  .status-locked {
    background: rgba(255, 120, 120, 0.1);
    color: var(--danger);
    border-color: rgba(255, 120, 120, 0.24);
  }
  .status-unlocked {
    background: rgba(50, 213, 154, 0.1);
    color: var(--success);
    border-color: rgba(50, 213, 154, 0.24);
  }
  .status-no-vault {
    background: rgba(255, 179, 71, 0.1);
    color: var(--warning);
    border-color: rgba(255, 179, 71, 0.24);
  }
  main {
    width: min(1160px, calc(100% - 36px));
    max-width: none;
    padding: 24px 0 54px;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 18px;
    align-items: start;
  }
  main.has-nav {
    grid-template-columns: minmax(0, 1fr);
  }
  main.has-nav > nav {
    grid-column: 1;
    grid-row: auto;
    align-self: auto;
    position: relative;
    top: auto;
  }
  main.has-nav > .card {
    grid-column: 1;
  }
  .section-nav {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background: rgba(12, 16, 21, 0.6);
    box-shadow:
      0 18px 32px rgba(0, 0, 0, 0.16),
      inset 0 1px 0 rgba(255, 255, 255, 0.03);
    backdrop-filter: blur(12px);
  }
  .section-nav::before {
    content: none;
  }
  .section-nav a {
    display: flex;
    align-items: center;
    min-height: 38px;
    padding: 0 14px;
    border-radius: 12px;
    text-decoration: none;
    color: var(--text-dim);
    font-family: var(--font-body);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    border: 1px solid transparent;
    background: transparent;
    transition:
      background 180ms ease,
      border-color 180ms ease,
      color 180ms ease,
      transform 180ms ease;
  }
  .section-nav a:hover,
  .section-nav a:focus-visible {
    color: var(--text);
    border-color: rgba(244, 248, 252, 0.08);
    background: rgba(255, 255, 255, 0.03);
    transform: none;
    outline: none;
  }
  .section-nav a.active {
    color: #081018;
    border-color: rgba(255, 255, 255, 0.08);
    background: linear-gradient(180deg, #eef4fa 0%, #c9d6e2 100%);
    box-shadow: 0 10px 20px rgba(0, 0, 0, 0.18);
  }
  .card {
    position: relative;
    margin-bottom: 0;
    padding: clamp(20px, 2.2vw, 26px);
    border-radius: 26px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.03), transparent 18%),
      linear-gradient(180deg, rgba(28, 34, 41, 0.94), rgba(16, 20, 25, 0.96));
    box-shadow:
      0 22px 48px rgba(0, 0, 0, 0.24),
      inset 0 1px 0 rgba(255, 255, 255, 0.04),
      inset 0 -18px 28px rgba(0, 0, 0, 0.12);
    overflow: hidden;
  }
  .card::before {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 24%);
    opacity: 0.5;
  }
  .card > * {
    position: relative;
    z-index: 1;
  }
  #statusCard {
    background:
      linear-gradient(180deg, rgba(31, 37, 44, 0.96), rgba(17, 21, 26, 0.98));
    border-color: rgba(244, 248, 252, 0.08);
  }
  #statusCard::after,
  #nextStepCard::after {
    content: none;
  }
  #authCard,
  #setupCard {
    border-color: rgba(244, 248, 252, 0.08);
  }
  #nextStepCard {
    border-color: rgba(244, 248, 252, 0.08);
    background: linear-gradient(180deg, rgba(24, 29, 35, 0.96), rgba(14, 18, 22, 0.96));
  }
  .eyebrow,
  .section-title {
    display: inline-flex;
    align-items: center;
    min-height: 0;
    padding: 0;
    border-radius: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    margin-bottom: 14px;
  }
  .card h2 {
    font-family: var(--font);
    font-size: clamp(23px, 2.3vw, 31px);
    font-weight: 700;
    letter-spacing: -0.045em;
    line-height: 1.08;
    color: var(--text);
    margin-bottom: 12px;
    text-transform: none;
  }
  .helper-text,
  .text-meta,
  .text-meta-sm,
  .card-note {
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.65;
  }
  .card-note {
    margin-top: 16px;
  }
  .section-divider {
    height: 1px;
    margin: 24px 0 20px;
    background: linear-gradient(90deg, rgba(244, 248, 252, 0.1), transparent 88%);
  }
  .hero-shell {
    display: grid;
    gap: 18px;
    grid-template-columns: minmax(0, 1.3fr) minmax(300px, 0.9fr);
    align-items: start;
  }
  .hero-copy {
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    min-width: 0;
  }
  .hero-copy h1 {
    max-width: 12.5ch;
    font-family: var(--font);
    font-size: clamp(34px, 4.8vw, 50px);
    font-weight: 700;
    line-height: 1;
    letter-spacing: -0.055em;
    margin-bottom: 14px;
    text-transform: none;
    color: var(--text);
  }
  .hero-summary {
    max-width: 58ch;
    font-size: clamp(15px, 1.35vw, 17px);
    line-height: 1.68;
    color: var(--text-dim);
  }
  .hero-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin: 24px 0 0;
  }
  .hero-context {
    display: grid;
    gap: 14px;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    margin-top: 22px;
  }
  .context-row {
    display: grid;
    gap: 6px;
    min-height: 100%;
    padding: 2px 0 2px 14px;
    border-radius: 0;
    border: none;
    border-left: 1px solid rgba(244, 248, 252, 0.08);
    background: transparent;
  }
  .context-row strong {
    color: var(--text);
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .context-row span {
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.55;
  }
  .hero-mode,
  .wizard-summary {
    display: grid;
    gap: 12px;
    padding: 18px;
    border-radius: 22px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.03), transparent 100%),
      rgba(15, 19, 24, 0.7);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.03),
      0 16px 30px rgba(0, 0, 0, 0.16);
  }
  .hero-mode-kicker,
  .progress-pill,
  .wizard-stage-title,
  .wizard-stage-summary {
    font-family: var(--mono);
  }
  .hero-mode-kicker,
  .progress-pill {
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }
  .hero-mode-value {
    font-family: var(--font);
    font-size: clamp(24px, 2.3vw, 32px);
    font-weight: 700;
    line-height: 1.02;
    letter-spacing: -0.05em;
    color: var(--text);
  }
  .hero-mode-detail {
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.65;
  }
  .hero-stats,
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 12px;
  }
  .stat {
    padding: 14px 16px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent),
      rgba(255, 255, 255, 0.016);
    text-align: left;
  }
  .stat .value {
    font-family: var(--mono);
    font-size: clamp(24px, 2.6vw, 32px);
    font-weight: 700;
    letter-spacing: -0.05em;
    color: var(--text);
  }
  .stat .label {
    margin-top: 6px;
    color: var(--text-dim);
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }
  .wizard-header {
    display: grid;
    grid-template-columns: minmax(0, 1.15fr) minmax(280px, 0.9fr);
    gap: 24px;
    align-items: start;
    margin-bottom: 26px;
  }
  .wizard-stage-title {
    color: var(--text);
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .wizard-stage-summary {
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.7;
    text-transform: none;
    letter-spacing: 0.02em;
  }
  .summary-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .checklist-list,
  .next-step-grid {
    display: grid;
    gap: 12px;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  }
  .checklist-item,
  .next-step-item {
    display: grid;
    gap: 10px;
    padding: 16px 18px;
    border-radius: 18px;
    border: 1px solid rgba(157, 184, 214, 0.14);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.03), transparent),
      rgba(255, 255, 255, 0.02);
  }
  .checklist-mark {
    width: fit-content;
    min-width: 34px;
    height: 34px;
    display: inline-grid;
    place-items: center;
    padding: 0 10px;
    border-radius: 999px;
    background: rgba(88, 166, 255, 0.14);
    color: var(--accent-strong);
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }
  .next-step-item strong {
    color: var(--text);
    font-family: var(--mono);
    font-size: 12px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .next-step-item span,
  .checklist-item div:last-child {
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1.6;
  }
  .card-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 22px;
  }
  .form-row,
  .form-row-center {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
  }
  .form-row-center {
    justify-content: space-between;
  }
  input[type="text"],
  input[type="password"],
  input[type="number"],
  input[type="file"],
  select {
    min-height: 48px;
    padding: 12px 16px;
    border-radius: 16px;
    border: 1px solid var(--border);
    background: rgba(10, 13, 17, 0.86);
    color: var(--text);
    font-family: var(--font-body);
    font-size: 14px;
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.04),
      inset 0 -10px 18px rgba(0, 0, 0, 0.12),
      0 0 0 0 rgba(216, 227, 237, 0);
    transition:
      border-color 180ms ease,
      box-shadow 180ms ease,
      background 180ms ease,
      transform 180ms ease;
  }
  input::placeholder {
    color: rgba(164, 176, 187, 0.72);
  }
  input:focus,
  select:focus {
    outline: none;
    border-color: rgba(216, 227, 237, 0.28);
    background: rgba(12, 16, 20, 0.94);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.05),
      inset 0 -10px 18px rgba(0, 0, 0, 0.14),
      0 0 0 3px rgba(216, 227, 237, 0.08);
  }
  input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  button {
    min-height: 48px;
    padding: 0 18px;
    border-radius: 14px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text);
    font-family: var(--font-body);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
    text-transform: none;
    transition:
      transform 180ms ease,
      background 180ms ease,
      border-color 180ms ease,
      color 180ms ease,
      box-shadow 180ms ease;
  }
  button:hover {
    transform: translateY(-1px);
  }
  button:focus-visible {
    outline: none;
    border-color: rgba(216, 227, 237, 0.26);
    box-shadow: 0 0 0 3px rgba(216, 227, 237, 0.1);
  }
  .btn-primary {
    background: linear-gradient(180deg, var(--accent-hover) 0%, #cbd7e2 100%);
    color: #081018;
    border-color: rgba(255, 255, 255, 0.14);
    box-shadow:
      0 14px 30px rgba(0, 0, 0, 0.2),
      inset 0 1px 0 rgba(255, 255, 255, 0.5);
  }
  .btn-primary:hover {
    background: linear-gradient(180deg, #f4f8fc 0%, #d7e1ea 100%);
  }
  .btn-ghost {
    background: rgba(255, 255, 255, 0.028);
    border-color: var(--border);
    color: var(--text);
  }
  .btn-ghost:hover {
    background: rgba(255, 255, 255, 0.05);
    border-color: var(--border-strong);
  }
  .btn-danger {
    background: rgba(255, 141, 127, 0.08);
    border-color: rgba(255, 141, 127, 0.18);
    color: #ffd4d4;
  }
  .btn-danger:hover {
    background: rgba(255, 141, 127, 0.14);
  }
  .btn-success {
    background: rgba(127, 197, 163, 0.1);
    border-color: rgba(127, 197, 163, 0.18);
    color: #d9f2e6;
  }
  .btn-success:hover {
    background: rgba(127, 197, 163, 0.16);
  }
  .unlock-tabs,
  .comp-switcher {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-bottom: 16px;
  }
  .unlock-tab,
  .comp-switcher button {
    min-height: 42px;
    padding: 0 14px;
    border-radius: 14px;
    border: 1px solid var(--border);
    background: rgba(255, 255, 255, 0.028);
    color: var(--text-dim);
    font-family: var(--font-body);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    text-transform: none;
  }
  .unlock-tab.active,
  .comp-switcher button.active {
    color: #081018;
    border-color: rgba(255, 255, 255, 0.08);
    background: linear-gradient(180deg, var(--accent-hover) 0%, #c9d6e2 100%);
  }
  .info-box,
  .result-box,
  .poison-warning {
    padding: 14px 16px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.025), transparent),
      rgba(255, 255, 255, 0.018);
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.65;
  }
  .info-box {
    border-color: rgba(216, 227, 237, 0.12);
    background:
      linear-gradient(180deg, rgba(216, 227, 237, 0.05), transparent),
      rgba(216, 227, 237, 0.035);
  }
  .result-box {
    margin-top: 16px;
    border-color: rgba(127, 197, 163, 0.16);
    background:
      linear-gradient(180deg, rgba(127, 197, 163, 0.05), transparent),
      rgba(127, 197, 163, 0.035);
  }
  .poison-warning {
    border-color: rgba(255, 141, 127, 0.2);
    background:
      linear-gradient(180deg, rgba(255, 141, 127, 0.06), transparent),
      rgba(255, 141, 127, 0.04);
    color: #ffcccc;
  }
  .method-btn {
    padding: 18px 20px;
    margin-bottom: 12px;
    border-radius: 22px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.025), transparent),
      rgba(255, 255, 255, 0.018);
    transition:
      transform 180ms ease,
      border-color 180ms ease,
      background 180ms ease,
      box-shadow 180ms ease;
  }
  .method-btn:hover {
    transform: translateY(-1px);
    border-color: var(--border-strong);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.035), transparent),
      rgba(255, 255, 255, 0.024);
  }
  .method-btn.recommended {
    border-color: rgba(216, 227, 237, 0.14);
    background:
      linear-gradient(180deg, rgba(216, 227, 237, 0.05), transparent),
      rgba(255, 255, 255, 0.024);
  }
  .method-btn .method-title,
  .entity-title,
  .wiz-comp-label {
    color: var(--text);
    font-family: var(--font);
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.03em;
  }
  .method-btn .method-title::after {
    font-family: var(--mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .method-btn .method-desc,
  .entity-meta {
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1.7;
  }
  .wiz-comp-row {
    padding: 16px 18px;
    margin-bottom: 10px;
    border-radius: 18px;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.03), transparent),
      rgba(255, 255, 255, 0.02);
    border: 1px solid rgba(157, 184, 214, 0.14);
    align-items: center;
  }
  .wiz-comp-threshold {
    color: var(--accent-strong);
    font-family: var(--mono);
    font-size: 12px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .entity-list,
  .key-list {
    list-style: none;
    border-radius: 22px;
    border: 1px solid var(--border);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent),
      rgba(255, 255, 255, 0.018);
    overflow: hidden;
  }
  .entity-list li,
  .key-list li {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
    padding: 18px;
    border-bottom: 1px solid rgba(244, 248, 252, 0.07);
  }
  .entity-list li:last-child,
  .key-list li:last-child {
    border-bottom: none;
  }
  .key-list li > span:first-child,
  .entity-title {
    min-width: 0;
    color: var(--text);
    font-family: var(--mono);
    font-size: 13px;
    line-height: 1.55;
    word-break: break-word;
  }
  .entity-main {
    min-width: 0;
    flex: 1;
  }
  .entity-actions,
  .key-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }
  .entity-actions button,
  .key-actions button {
    min-height: 40px;
    padding: 0 14px;
    font-size: 11px;
  }
  .secret-value {
    margin-top: 12px;
    padding: 12px 14px;
    border-radius: 16px;
    border: 1px solid rgba(88, 166, 255, 0.18);
    background: rgba(4, 8, 14, 0.82);
    color: var(--accent-strong);
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.7;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    min-height: 26px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid transparent;
    font-family: var(--mono);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    vertical-align: middle;
  }
  .pill-neutral {
    background: rgba(157, 184, 214, 0.1);
    color: var(--text-dim);
    border-color: rgba(157, 184, 214, 0.16);
  }
  .pill-good {
    background: rgba(50, 213, 154, 0.1);
    color: var(--success);
    border-color: rgba(50, 213, 154, 0.18);
  }
  .pill-warn {
    background: rgba(255, 179, 71, 0.1);
    color: var(--warning);
    border-color: rgba(255, 179, 71, 0.18);
  }
  .pill-danger {
    background: rgba(255, 120, 120, 0.1);
    color: var(--danger);
    border-color: rgba(255, 120, 120, 0.18);
  }
  .checkbox-row,
  .checkbox-label {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    min-height: 48px;
    border-radius: 16px;
    border: 1px solid var(--border);
    background: rgba(255, 255, 255, 0.018);
    color: var(--text-dim);
    font-family: var(--font-body);
    font-size: 13px;
    font-weight: 500;
    letter-spacing: -0.01em;
    text-transform: none;
  }
  .pin-modal {
    width: min(420px, calc(100vw - 28px));
  }
  .pin-modal h2 {
    font-family: var(--font);
    font-size: 24px;
    line-height: 1.1;
    letter-spacing: -0.04em;
    color: var(--text);
  }
  .toast {
    bottom: 24px;
    right: 24px;
    min-width: 260px;
    max-width: min(420px, calc(100vw - 32px));
    padding: 14px 18px;
    border-radius: 18px;
    border: 1px solid var(--border);
    background: rgba(12, 16, 20, 0.94);
    box-shadow: 0 24px 48px rgba(0, 0, 0, 0.34);
    font-family: var(--font-body);
    font-size: 13px;
    line-height: 1.55;
    letter-spacing: -0.01em;
  }
  .toast-success {
    color: #d9fff0;
    border-color: rgba(127, 197, 163, 0.22);
    background:
      linear-gradient(180deg, rgba(127, 197, 163, 0.1), transparent),
      rgba(12, 16, 20, 0.96);
  }
  .toast-error {
    color: #ffe2e2;
    border-color: rgba(255, 141, 127, 0.22);
    background:
      linear-gradient(180deg, rgba(255, 141, 127, 0.1), transparent),
      rgba(12, 16, 20, 0.96);
  }
  footer {
    position: relative;
    padding: 8px 18px 34px;
    color: var(--text-soft);
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    border-top: none;
    z-index: 1;
  }
  footer a {
    color: var(--accent-strong);
  }
  @media (max-width: 1180px) {
    main.has-nav {
      grid-template-columns: minmax(0, 1fr);
    }
    main.has-nav > nav,
    main.has-nav > .card {
      grid-column: 1;
    }
    main.has-nav > nav {
      position: relative;
      top: auto;
      grid-row: auto;
      overflow-x: auto;
      flex-direction: row;
      flex-wrap: wrap;
      padding: 0;
    }
    .section-nav a {
      white-space: nowrap;
    }
    .hero-shell,
    .wizard-header {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (max-width: 760px) {
    header {
      align-items: flex-start;
    }
    main {
      width: min(100%, calc(100% - 24px));
      gap: 16px;
    }
    .card {
      padding: 18px;
      border-radius: 24px;
    }
    .hero-copy h1 {
      font-size: clamp(30px, 10vw, 42px);
    }
    .hero-summary,
    .helper-text,
    .text-meta,
    .card-note {
      font-size: 13px;
    }
    .hero-context,
    .checklist-list,
    .guide-grid,
    .next-step-grid,
    .hero-stats,
    .stats {
      grid-template-columns: minmax(0, 1fr);
    }
    .entity-list li,
    .key-list li,
    .form-row,
    .form-row-center {
      flex-direction: column;
      align-items: stretch;
    }
    .entity-actions,
    .key-actions {
      width: 100%;
      justify-content: stretch;
    }
    .entity-actions button,
    .key-actions button,
    button {
      width: 100%;
    }
    .checkbox-row,
    .checkbox-label {
      width: 100%;
      justify-content: flex-start;
    }
    .header-right {
      width: 100%;
      justify-content: flex-start;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    * {
      animation: none !important;
      transition-duration: 0ms !important;
      scroll-behavior: auto !important;
    }
    button:hover,
    .workspace-tab:hover,
    .workspace-tab:focus-visible {
      transform: none;
    }
  }
</style>
</head>
<body>

<header>
  <div class="brand-lockup">
    <div class="brand-mark">S</div>
    <div>
      <div class="logo"><span>Sigillum</span> Local Control</div>
      <div class="logo-sub">Local vault operations for access, secrets, profiles, deposits, queue work, recovery, and audit on this machine.</div>
    </div>
  </div>
  <div class="header-right">
    <div id="refreshMeta" class="refresh-meta" data-state="busy">syncing...</div>
    <div id="compartmentBadge" class="compartment-badge hidden"></div>
    <div id="statusBadge" class="status-badge status-locked">checking...</div>
  </div>
</header>

<main>
  <nav id="sectionNav" class="section-nav hidden" aria-label="Visible sections"></nav>

  <!-- Status Card -->
  <div class="card" id="statusCard" data-nav-label="Overview" data-workspace-section="overview">
    <div class="hero-shell">
      <div class="hero-copy">
        <div class="eyebrow" id="statusEyebrow">Local vault daemon</div>
        <h1 id="statusTitle">Bring your vault online</h1>
        <p class="hero-summary" id="statusSummary">Create a vault, unlock it only when needed, and run every sensitive workflow locally on this machine.</p>
        <div class="hero-actions">
          <button class="btn-primary" id="heroPrimaryBtn" data-action="heroPrimaryAction">Open setup</button>
          <button class="btn-ghost" id="heroSecondaryBtn" data-action="heroSecondaryAction">Compare setup options</button>
        </div>
      </div>
      <div>
        <div class="hero-mode">
          <div class="hero-mode-kicker">Current state</div>
          <div class="hero-mode-value" id="heroModeValue">Checking daemon state…</div>
          <div class="hero-mode-detail" id="heroModeDetail">Status, next actions, and scope will appear here as soon as the daemon responds.</div>
        </div>
        <div class="stats hero-stats">
          <div class="stat">
            <div class="value" id="apiKeyCount">-</div>
            <div class="label">API Keys</div>
          </div>
          <div class="stat">
            <div class="value" id="secretCount">-</div>
            <div class="label">Secrets</div>
          </div>
          <div class="stat">
            <div class="value" id="compartmentCount">-</div>
            <div class="label">Compartments</div>
          </div>
        </div>
      </div>
    </div>
    <div class="hero-context" id="statusContext"></div>
  </div>

  <!-- Unlock / Lock Card -->
  <div class="card" id="authCard" data-nav-label="Access" data-workspace-section="access">
    <div class="eyebrow">Access</div>
    <h2 id="authTitle">Unlock this local session</h2>
    <p class="helper-text" id="authLead">Unlock with the passphrase or hardware-key threshold you configured during setup. Hardware-key touches are enough when the authenticator allows it; enter a current PIN only for keys that require one. The resulting session token stays only in this browser tab.</p>
    <!-- Compartment switcher (only visible when multiple unlocked) -->
    <div id="compSwitcher" class="comp-switcher hidden"></div>
    <div id="unlockTabs" class="unlock-tabs hidden">
      <button type="button" class="unlock-tab active" data-action="switchUnlockTab" data-arg0="passphrase">Passphrase</button>
      <button type="button" class="unlock-tab" data-action="switchUnlockTab" data-arg0="fido2">Hardware Key</button>
    </div>
    <!-- Deniable passphrase unlock: just a password field, no hints -->
    <div id="unlockPassphrase">
      <div class="form-row">
        <input type="password" id="passphrase" placeholder="Enter vault passphrase">
        <button class="btn-primary" data-action="unlock">Unlock vault</button>
      </div>
    </div>
    <!-- Deniable FIDO2 unlock: optional PIN + number, no compartment hints -->
    <div id="unlockFido2" class="hidden">
      <div class="form-row">
        <input type="password" id="fido2Pin" placeholder="Current hardware-key PIN (optional)">
        <input type="number" id="fido2TapCount" min="1" value="1" class="input-narrow" placeholder="Keys">
        <button class="btn-primary" data-action="fido2Unlock">Unlock with hardware key</button>
      </div>
    </div>
    <div id="authGuidance" class="info-box">Unlocking reveals the protected workspace only in this browser tab. The daemon keeps your local data on disk even while the session is locked.</div>
    <div id="authRecovery">
      <div class="section-divider"></div>
      <div class="section-title">Recover this machine</div>
      <p class="helper-text">If setup failed or this browser session got stranded, restore a snapshot or wipe only this machine’s local Sigillum data and start over.</p>
      <div class="form-row">
        <input type="file" id="authRestoreFile" style="flex:1;">
        <input type="password" id="authRestorePass" placeholder="Restore passphrase" style="flex:1;">
        <button class="btn-danger" data-action="restoreAuthSnapshot">Restore Snapshot</button>
      </div>
      <div class="form-row">
        <input type="text" id="authResetConfirm" placeholder="Type RESET LOCAL SIGILLUM DATA to start over">
        <button class="btn-danger" data-action="resetLocalData" data-arg0="authResetConfirm">Reset local data</button>
      </div>
      <p class="card-note">Restoring replaces local on-disk state. Resetting erases the local Sigillum data directory on this machine and returns you to first-run setup.</p>
    </div>
    <div id="lockForm" class="hidden">
      <div class="form-row" style="margin-bottom:0;">
        <button class="btn-ghost" data-action="logoutSession">Log Out Session</button>
        <button class="btn-danger" data-action="lock">Lock All Compartments</button>
      </div>
    </div>
  </div>

  <!-- Setup Wizard -->
  <div class="card hidden" id="setupCard" data-nav-label="Setup" data-workspace-section="access">
    <div class="wizard-header">
      <div>
        <div class="eyebrow">Setup assistant</div>
        <h2>Create a vault that matches how you work</h2>
        <p class="helper-text">Choose a starting protection model, review what Sigillum will create, and finish with a guided hardware-key or passphrase flow. You can add more keys and compartments later.</p>
      </div>
      <div class="wizard-summary">
        <div class="progress-pill" id="wizStagePill">Step 1 of 3</div>
        <div class="wizard-stage-title" id="wizStageTitle">Choose a protection model</div>
        <div class="wizard-stage-summary" id="wizStageSummary">Pick the setup pattern that matches how many compartments and hardware keys you want to manage.</div>
        <div class="info-box" id="wizDeviceHint" style="margin-top:14px;">Checking for attached FIDO2 devices on this machine…</div>
        <div class="summary-actions">
          <button class="btn-ghost" data-action="wizDetectDevice">Check hardware key again</button>
        </div>
        <div id="wizChecklist" class="checklist-list"></div>
      </div>
    </div>

    <!-- Step 0: Choose preset -->
    <div class="wizard-step active" id="wizStep0">
      <p>Start with a recommended plan. The names below describe how access is split, not how secrets are stored.</p>
      <button class="method-btn recommended" data-action="wizPreset" data-arg0="secure">
        <div class="method-title">Daily + Secure</div>
        <div class="method-desc">Best for most people. One key unlocks everyday work, while the higher-trust secure lane requires two. You can add more keys later.</div>
      </button>
      <button class="method-btn" data-action="wizPreset" data-arg0="simple">
        <div class="method-title">Single Compartment</div>
        <div class="method-desc">Best for local testing or one operator who wants the fastest setup. Lowest ceremony now, but no separate high-trust lane until you add one later.</div>
      </button>
      <button class="method-btn" data-action="wizPreset" data-arg0="legacy">
        <div class="method-title">Legacy / Estate Planning</div>
        <div class="method-desc">Best for long-term recovery or shared custody. More setup ceremony now, but clearer hot, cold, and legacy access layers later.</div>
      </button>
      <button class="method-btn" data-action="wizPreset" data-arg0="custom">
        <div class="method-title">Custom</div>
        <div class="method-desc">Best when you already know your compartment model. You trade setup speed for exact threshold control.</div>
      </button>
      <button class="method-btn" data-action="wizPreset" data-arg0="passphrase">
        <div class="method-title">Passphrase Only</div>
        <div class="method-desc">Best when this machine will not use hardware keys. Easiest to start, but weaker than hardware-key protection.</div>
      </button>
    </div>

    <!-- Step 2a: Passphrase compartment setup -->
    <div class="wizard-step" id="wizStepPassphrase">
      <p>Create your first compartment and protect it with a strong passphrase. You can add more compartments and hardware keys later.</p>
      <div class="form-row">
        <input type="text" id="wizPLabel" placeholder="Compartment name (e.g. default)" value="default">
      </div>
      <div class="form-row">
        <input type="password" id="wizPassphrase" placeholder="Create a passphrase (min 8 chars)">
      </div>
      <div class="form-row">
        <input type="password" id="wizPassphraseConfirm" placeholder="Confirm passphrase">
      </div>
      <div class="form-row">
        <button class="btn-ghost" data-action="wizBackToPresets">Back</button>
        <button class="btn-primary" data-action="wizInitPassphrase">Create vault</button>
      </div>
    </div>

    <!-- Step 2b: FIDO2 compartment confirm (presets already selected) -->
    <div class="wizard-step" id="wizStepCompartments">
      <p>Review the access layers Sigillum will create. Thresholds refer to how many distinct hardware keys are needed to unlock each compartment.</p>
      <div id="wizCompList"></div>
      <div class="form-row" style="margin-top:12px;">
        <button class="btn-ghost" data-action="wizBackToPresets">Back</button>
        <button class="btn-primary" data-action="wizProceedFido2">Continue to key registration</button>
      </div>
    </div>

    <!-- Step 2c: Custom compartments -->
    <div class="wizard-step" id="wizStepCustomComps">
      <p>Add each compartment you want Sigillum to create. Use thresholds to express which workflows should require more than one hardware key.</p>
      <div id="wizCustomCompList"></div>
      <div class="form-row">
        <input type="text" id="wizCustomLabel" placeholder="Label">
        <input type="number" id="wizCustomThreshold" placeholder="Threshold" min="1" class="input-narrow">
        <button class="btn-ghost" data-action="wizAddCustomComp">Add compartment</button>
      </div>
      <div class="form-row" style="margin-top:12px;">
        <button class="btn-ghost" data-action="wizBackToPresets">Back</button>
        <button class="btn-primary" id="wizCustomContinue" data-action="wizProceedFido2" disabled>Continue to key registration</button>
      </div>
    </div>

    <!-- Step 3: FIDO2 PIN + label -->
    <div class="wizard-step" id="wizStepFido2Pin">
      <p>Register the first hardware key for this vault. If the key works with touch alone, you can leave the PIN field empty. If the authenticator already requires a current PIN, enter it here before starting registration.</p>
      <div class="form-row">
        <input type="password" id="wizFido2Pin" placeholder="Current FIDO2 PIN if this key requires one">
      </div>
      <div class="form-row">
        <input type="text" id="wizFido2Label" placeholder="Key label (e.g. yubikey-primary)">
      </div>
      <div class="form-row">
        <button class="btn-ghost" data-action="wizBackFromFido2Pin">Back</button>
        <button class="btn-primary" data-action="wizRegisterKey">Register primary hardware key</button>
      </div>
      <div class="info-box" style="margin-top:14px;">
        Brand-new hardware keys often ship without a FIDO2 PIN. You can keep using touch-only hardware keys, or set a PIN on the inserted key here if you want that extra gate.
      </div>
      <div class="form-row" style="margin-top:12px;">
        <input type="password" id="wizNewFido2Pin" placeholder="New PIN for this key (min 4 chars)">
        <input type="password" id="wizNewFido2PinConfirm" placeholder="Confirm new PIN">
        <button class="btn-ghost" data-action="wizSetNewPin">Set key PIN</button>
      </div>
      <p>Optional but recommended: set one fallback passphrase that can locally unwrap every compartment if your hardware keys are unavailable.</p>
      <div class="form-row">
        <input type="password" id="wizFallbackPass" placeholder="Fallback passphrase (optional)">
      </div>
    </div>

    <div class="wizard-step" id="wizStepAdditionalKeys">
      <p id="wizAdditionalKeysLead">This plan includes higher-threshold compartments, so Sigillum needs more than one enrolled hardware key before every access layer is usable.</p>
      <div class="info-box" id="wizAdditionalKeyStatus">1 of 2 required hardware keys enrolled so far.</div>
      <div class="form-row" style="margin-top:12px;">
        <input type="password" id="wizAdditionalKeyPin" placeholder="Current PIN for the inserted backup key (optional)">
        <input type="text" id="wizAdditionalKeyLabel" placeholder="Backup key label (e.g. yubikey-backup)">
        <button class="btn-primary" data-action="wizRegisterAdditionalKey">Register next hardware key</button>
      </div>
      <div class="info-box" style="margin-top:14px;">
        Brand-new backup key? Touch-only works too. Set its first FIDO2 PIN here only if you want that key to require one.
      </div>
      <div class="form-row" style="margin-top:12px;">
        <input type="password" id="wizAdditionalNewPin" placeholder="New PIN for inserted backup key">
        <input type="password" id="wizAdditionalNewPinConfirm" placeholder="Confirm new PIN">
        <button class="btn-ghost" data-action="wizSetAdditionalKeyPin">Set backup key PIN</button>
      </div>
      <p id="wizAdditionalKeysNote">If you finish with fewer keys than the highest threshold, the lower-threshold compartments will work now, but the stronger access layers will stay unavailable until you enroll more keys later.</p>
      <div class="form-row">
        <button class="btn-ghost" data-action="wizFinishForNow">Finish for now</button>
      </div>
    </div>

    <!-- Step 4: Touch -->
    <div class="wizard-step" id="wizStepTouch">
      <div class="info-box pulse" style="text-align:center;font-size:16px;padding:24px;">
        Touch your FIDO2 key now...
      </div>
    </div>

    <!-- Step 5: Done -->
    <div class="wizard-step" id="wizStepDone">
      <div class="wiz-center">
        <div style="font-size:32px;margin-bottom:12px;color:var(--success);">&#10003;</div>
        <div style="font-size:16px;font-weight:600;" id="wizDoneMsg">Setup Complete</div>
        <p style="margin-top:8px;" id="wizDoneDetail">Your vault is ready.</p>
      </div>
    </div>

    <div class="section-divider"></div>
    <div class="section-title">Recover or start over</div>
    <p class="helper-text">If setup gets interrupted, you can restore a previously exported snapshot or erase this machine’s local Sigillum data and restart first-run setup.</p>
    <div class="form-row">
      <input type="file" id="setupRestoreFile" style="flex:1;">
      <input type="password" id="setupRestorePass" placeholder="Restore passphrase" style="flex:1;">
      <button class="btn-danger" data-action="restoreSetupSnapshot">Restore Snapshot</button>
    </div>
    <div class="form-row">
      <input type="text" id="setupResetConfirm" placeholder="Type RESET LOCAL SIGILLUM DATA to start over">
      <button class="btn-danger" data-action="resetLocalData" data-arg0="setupResetConfirm">Reset local setup</button>
    </div>
    <p class="card-note">Resetting removes the local Sigillum data directory on this machine. Restoring replaces it with the contents of the selected encrypted snapshot.</p>
  </div>

  <div class="card hidden" id="nextStepCard" data-workspace-section="overview">
    <div class="eyebrow">Recommended next move</div>
    <h2 id="nextStepTitle">Finish the essentials before deeper operator work</h2>
    <p class="helper-text" id="nextStepSummary">Sigillum will keep this guidance updated as the workspace becomes more complete.</p>
    <div class="next-step-grid" id="nextStepList"></div>
    <div class="card-actions">
      <button class="btn-primary" id="nextStepPrimaryBtn" data-action="nextStepPrimaryAction">Open next step</button>
      <button class="btn-ghost" id="nextStepSecondaryBtn" data-action="nextStepSecondaryAction">Open supporting section</button>
    </div>
    <div class="card-note" id="nextStepNote">This card suggests the next highest-leverage action while leaving the rest of the operator surface available below.</div>
  </div>

  <div class="card hidden" id="guideCard" data-workspace-section="overview">
    <div class="eyebrow">Operator guide</div>
    <h2>What Sigillum does today</h2>
    <p class="helper-text">
      Sigillum is currently a local vault plus an Ethereum operator console. Use the workspace modes above as distinct jobs, not as a single scrolling checklist.
    </p>
    <div class="guide-grid">
      <div class="guide-block">
        <div class="guide-block-title">1. Access</div>
        <p>Set up or unlock the local vault, then verify your recovery path before you do anything sensitive.</p>
      </div>
      <div class="guide-block">
        <div class="guide-block-title">2. Vault</div>
        <p>Store connection keys and encrypted secrets, organize compartments, and move values between compartments when needed.</p>
      </div>
      <div class="guide-block">
        <div class="guide-block-title">3. Wallets</div>
        <p>Save provider profiles, then choose a wallet family: stealth wallets for tracked deposits and queue work, or xpub receive wallets for public receive-tree export and address previews.</p>
      </div>
      <div class="guide-block">
        <div class="guide-block-title">4. Operations</div>
        <p>Run tracked deposits, queue processing, and maintenance. These operator workflows are currently implemented for stealth wallets.</p>
      </div>
      <div class="guide-block">
        <div class="guide-block-title">5. Recovery</div>
        <p>Manage hardware keys, export encrypted snapshots, inspect the audit trail, and verify daemon health from one place.</p>
      </div>
    </div>
    <div class="section-divider"></div>
    <div class="section-title">Shipped now vs later roadmap</div>
    <div class="guide-grid">
      <div class="guide-block">
        <div class="guide-block-title">Available now</div>
        <ul class="guide-list">
          <li>Local passphrase and FIDO2 vault workflows</li>
          <li>Flat connection-key and encrypted-secret storage</li>
          <li>EVM provider profiles and stealth wallet profiles</li>
          <li>Initial xpub receive-branch export and public address preview</li>
          <li>Stealth deposits, queue processing, maintenance, snapshots, audit, and diagnostics</li>
        </ul>
      </div>
      <div class="guide-block">
        <div class="guide-block-title">Not implemented yet</div>
        <ul class="guide-list">
          <li>Xpub discovery, hidden sponsor or treasury branches, and xpub sweeping automation</li>
          <li>Hierarchical secret or wallet tree browsing</li>
          <li>Remote multi-host control and aggregated remote audit workflows</li>
        </ul>
      </div>
    </div>
  </div>

  <!-- Compartment Management Card (only when unlocked) -->
  <div class="card hidden" id="compartmentCard" data-nav-label="Compartments" data-workspace-section="vault">
    <div class="eyebrow">Vault</div>
    <h2>Unlocked compartments</h2>
    <p class="helper-text">These are the compartments currently available in this browser session. Switching changes which compartment new operations target.</p>
    <div id="compartmentList"></div>
  </div>

  <!-- Push-Down Card (only when all compartments unlocked) -->
  <div class="card hidden" id="pushCard" data-nav-label="Push" data-workspace-section="vault">
    <div class="eyebrow">Vault</div>
    <h2>Move a secret between compartments</h2>
    <p class="text-meta-sm" style="margin-bottom:12px;">
      Copy a secret from one compartment to another. The copy is indistinguishable from manual entry.
    </p>
    <div class="form-row">
      <select id="pushFrom" class="input-wide"></select>
      <select id="pushTo" class="input-wide"></select>
    </div>
    <div class="form-row">
      <input type="text" id="pushKey" placeholder="Secret key name">
      <input type="text" id="pushNewKey" placeholder="New name (optional)">
    </div>
    <div class="form-row">
      <select id="pushTier" class="input-wide">
        <option value="2">Tier 2 (encrypted)</option>
        <option value="1">Tier 1 (API key)</option>
      </select>
      <button class="btn-primary" data-action="pushSecret">Push</button>
    </div>
  </div>

  <!-- API Keys Card -->
  <div class="card" id="apiKeysCard" data-nav-label="API Keys" data-workspace-section="vault">
    <div class="eyebrow">Vault</div>
    <h2>Connection keys</h2>
    <p class="helper-text">Store RPC tokens and similar operational keys that Sigillum needs to use directly during daemon workflows.</p>
    <div class="form-row">
      <input type="text" id="apiKeyName" placeholder="Key name">
      <input type="password" id="apiKeyValue" placeholder="Value">
      <button class="btn-primary" data-action="setApiKey">Store API key</button>
    </div>
    <ul class="key-list" id="apiKeyList"></ul>
  </div>

  <!-- Secrets Card -->
  <div class="card" id="secretsCard" data-nav-label="Secrets" data-workspace-section="vault">
    <div class="eyebrow">Vault</div>
    <h2>Encrypted secrets</h2>
    <p class="helper-text">Store values that should stay encrypted at rest and require an unlocked compartment to view or modify.</p>
    <div id="secretsLocked" class="hidden text-meta">
      Vault is locked. Unlock to manage secrets.
    </div>
    <div id="secretsUnlocked">
      <div class="form-row">
        <input type="text" id="secretName" placeholder="Secret name">
        <input type="password" id="secretValue" placeholder="Value">
        <button class="btn-primary" data-action="setSecret">Store secret</button>
      </div>
      <ul class="key-list" id="secretList"></ul>
    </div>
  </div>

  <!-- Profiles Card -->
  <div class="card hidden" id="profilesCard" data-nav-label="Profiles" data-workspace-section="wallets">
    <div class="eyebrow">Wallets</div>
    <h2>Providers and stealth wallets</h2>
    <p class="helper-text">
      Start by saving reusable provider settings, then bind stealth wallet labels to those providers.
      Stealth wallets power tracked deposits, sweeps, queue jobs, and maintenance today.
    </p>

    <div class="section-title">EVM Provider Profiles</div>
    <div class="form-row">
      <input type="text" id="providerName" placeholder="Profile name">
      <input type="text" id="providerRpcUrl" placeholder="RPC URL">
      <input type="number" id="providerChainId" placeholder="Chain ID" value="1" class="input-compact">
    </div>
    <div class="form-row">
      <input type="text" id="providerAuthTokenKey" placeholder="Auth token key (optional)">
      <input type="number" id="providerCompartmentId" placeholder="Compartment (optional)" class="input-wider">
    </div>
    <div class="form-row">
      <input type="text" id="providerMaxPriorityFee" placeholder="Max priority fee hex (optional)">
      <input type="text" id="providerMaxFee" placeholder="Max fee hex (optional)">
    </div>
    <div class="form-row">
      <input type="number" id="providerNativeGasLimit" placeholder="Native gas limit (optional)">
      <input type="number" id="providerErc20GasLimit" placeholder="ERC-20 gas limit (optional)">
      <button class="btn-primary" data-action="upsertProviderProfile">Save Provider</button>
    </div>
    <div id="providerProfileList"></div>

    <div class="section-divider"></div>

    <div class="section-title">Stealth Wallet Profiles</div>
    <div class="form-row">
      <input type="text" id="walletProfileName" placeholder="Profile name">
      <input type="text" id="walletLabel" placeholder="Wallet label">
      <input type="text" id="walletShortName" placeholder="Short name (optional)">
    </div>
    <div class="form-row">
      <select id="walletProviderProfile"></select>
      <input type="number" id="walletCompartmentId" placeholder="Compartment (optional)" class="input-wider">
      <input type="number" id="walletChainId" placeholder="Chain ID (optional)" class="input-mid">
    </div>
    <div class="form-row">
      <input type="text" id="walletDefaultDestination" placeholder="Default destination address (optional)">
      <button class="btn-primary" data-action="upsertWalletProfile">Save Wallet</button>
    </div>
    <div id="walletProfileList"></div>
  </div>

  <div class="card hidden" id="xpubCard" data-nav-label="Xpub Wallets" data-workspace-section="wallets">
    <div class="eyebrow">Wallets</div>
    <h2>Xpub receive wallets</h2>
    <p class="helper-text">
      This initial xpub slice exports a public receive branch from a saved profile and previews derived receive addresses. It does not yet implement discovery, treasury branches, or sweeping automation.
    </p>

    <div class="section-title">Xpub Receive Profiles</div>
    <div class="form-row">
      <input type="text" id="xpubProfileName" placeholder="Profile name">
      <input type="number" id="xpubProjectAccount" placeholder="Project account" value="0" class="input-mid">
      <select id="xpubProviderProfile"></select>
    </div>
    <div class="form-row">
      <input type="number" id="xpubCompartmentId" placeholder="Compartment (optional)" class="input-wider">
      <input type="number" id="xpubChainId" placeholder="Chain ID (optional)" class="input-mid">
      <input type="text" id="xpubDefaultDestination" placeholder="Default destination address (optional)">
    </div>
    <div class="form-row">
      <button class="btn-primary" data-action="upsertXpubWalletProfile">Save Xpub Profile</button>
    </div>
    <div id="xpubWalletProfileList"></div>

    <div class="section-divider"></div>

    <div class="section-title">Export Receive Branch</div>
    <p class="helper-text">Choose a saved xpub wallet profile to export its public receive branch. The exported xpub can derive receive addresses without touching private key material.</p>
    <div class="form-row">
      <select id="xpubPreviewProfile"></select>
      <button class="btn-primary" data-action="exportSelectedXpubWallet">Export Xpub</button>
    </div>
    <div id="xpubExportResult" class="result-box hidden"></div>

    <div class="section-divider"></div>

    <div class="section-title">Preview Receive Addresses</div>
    <p class="helper-text">Paste or reuse an exported receive-branch xpub, then derive a receive address at a specific index.</p>
    <div class="form-row">
      <input type="text" id="xpubReceiveXpub" placeholder="Receive-branch xpub">
      <input type="number" id="xpubPreviewIndex" placeholder="Index" value="0" class="input-mid">
      <button class="btn-ghost" data-action="previewXpubReceiveAddress">Derive Address</button>
    </div>
    <div id="xpubPreviewResult" class="result-box hidden"></div>
  </div>

  <!-- Deposits Card -->
  <div class="card hidden" id="depositsCard" data-nav-label="Deposits" data-workspace-section="operations">
    <div class="eyebrow">Operations</div>
    <h2>Deposits</h2>
    <p class="helper-text">
      Generate tracked deposit addresses from wallet profiles, refresh detected balances, and enqueue
      sweeps without leaving the local daemon.
    </p>

    <div class="section-title">Create Native Deposit</div>
    <div class="form-row">
      <select id="depositNativeWalletProfile"></select>
      <input type="text" id="depositNativeExpected" placeholder="Expected value hex (optional)">
      <input type="text" id="depositNativeMinSweep" placeholder="Min sweep value hex (optional)">
    </div>
    <div class="form-row">
      <input type="text" id="depositNativeDestination" placeholder="Sweep destination (optional)">
      <input type="text" id="depositNativeNote" placeholder="Note (optional)">
      <label class="checkbox-row">
        <input type="checkbox" id="depositNativeAutoQueue" checked>
        Auto queue sweep
      </label>
      <button class="btn-primary" data-action="createNativeDeposit">Create Native</button>
    </div>

    <div class="section-divider"></div>

    <div class="section-title">Create ERC-20 Deposit</div>
    <div class="form-row">
      <select id="depositErc20WalletProfile"></select>
      <input type="text" id="depositErc20TokenAddress" placeholder="Token address">
      <input type="text" id="depositErc20Expected" placeholder="Expected amount hex (optional)">
    </div>
    <div class="form-row">
      <input type="text" id="depositErc20MinSweep" placeholder="Min sweep amount hex (optional)">
      <input type="text" id="depositErc20Destination" placeholder="Sweep destination (optional)">
      <input type="text" id="depositErc20Note" placeholder="Note (optional)">
    </div>
    <div class="form-row">
      <label class="checkbox-row">
        <input type="checkbox" id="depositErc20AutoQueue" checked>
        Auto queue sweep
      </label>
      <button class="btn-primary" data-action="createErc20Deposit">Create ERC-20</button>
    </div>

    <div class="section-divider"></div>

    <div class="form-row">
      <input type="number" id="depositRefreshLimit" placeholder="Refresh limit" value="50" class="input-mid">
      <label class="checkbox-row">
        <input type="checkbox" id="depositRefreshAutoEnqueue" checked>
        Auto enqueue sweeps
      </label>
      <button class="btn-ghost" data-action="refreshDepositRegistry">Refresh Deposits</button>
    </div>
    <div id="depositRefreshResult" class="result-box hidden"></div>
    <div id="depositList"></div>
  </div>

  <!-- Queue Card -->
  <div class="card hidden" id="queueCard" data-nav-label="Queue" data-workspace-section="operations">
    <div class="eyebrow">Operations</div>
    <h2>Queue</h2>
    <p class="helper-text">
      Inspect queued sends and sweep jobs, then process a batch or rerun an individual job.
    </p>
    <div class="form-row">
      <input type="number" id="queueProcessLimit" placeholder="Process limit" value="20" class="input-mid">
      <button class="btn-primary" data-action="processQueueBatch">Process Queue</button>
      <button class="btn-ghost" data-action="loadQueueJobs">Refresh Queue</button>
    </div>
    <p class="card-note">Processing executes pending local jobs immediately with the current daemon state, then refreshes the list below.</p>
    <div id="queueProcessResult" class="result-box hidden"></div>
    <div id="queueList"></div>
  </div>

  <!-- Maintenance Card -->
  <div class="card hidden" id="maintenanceCard" data-nav-label="Maintenance" data-workspace-section="operations">
    <div class="eyebrow">Operations</div>
    <h2>Maintenance</h2>
    <p class="helper-text">
      Run one local operator cycle that refreshes deposits, auto-enqueues eligible sweeps, and
      drains the queue using the same policy settings the daemon already supports.
    </p>
    <div class="form-row">
      <input type="number" id="maintenanceDepositLimit" placeholder="Deposit refresh limit" value="50" class="input-wide">
      <input type="number" id="maintenanceQueueLimit" placeholder="Queue process limit" value="20" class="input-wide">
      <label class="checkbox-row">
        <input type="checkbox" id="maintenanceAutoEnqueue" checked>
        Auto enqueue sweeps
      </label>
      <button class="btn-primary" data-action="runMaintenanceCycle">Run Maintenance</button>
    </div>
    <p class="card-note">A maintenance run refreshes deposits, auto-enqueues eligible sweeps, and processes queued jobs as one local operator cycle.</p>
    <div id="maintenanceResult" class="result-box hidden"></div>
  </div>

  <!-- FIDO2 Management Card -->
  <div class="card hidden" id="fido2Card" data-nav-label="FIDO2" data-workspace-section="recovery">
    <div class="eyebrow">Recovery & Access</div>
    <h2>Hardware keys</h2>
    <div id="fido2DeviceStatus" class="info-box">Checking for devices...</div>
    <div id="fido2RegisterSection">
      <p class="text-meta" style="margin-bottom:10px;">
        Register another FIDO2 hardware key. All compartments must be unlocked first so Sigillum can safely reshare the vault material.
      </p>
      <div class="info-box" style="margin-bottom:12px;">
        Fresh backup keys may not have a FIDO2 PIN yet. Touch-only enrollment works when the authenticator allows it. Set a PIN here only if you want the inserted key to require one.
      </div>
      <div class="form-row">
        <input type="password" id="fido2NewPin" placeholder="New PIN for inserted key (min 4 chars)">
        <input type="password" id="fido2NewPinConfirm" placeholder="Confirm new PIN">
        <button class="btn-ghost" data-action="fido2SetNewPin">Set key PIN</button>
      </div>
      <div class="form-row">
        <input type="password" id="fido2RegPin" placeholder="Current FIDO2 PIN if this key requires one">
        <input type="text" id="fido2RegLabel" placeholder="Key label">
        <button class="btn-primary" data-action="fido2Register">Register Key</button>
      </div>
      <div class="form-row-center">
        <label class="checkbox-label">
          <input type="checkbox" id="fido2Poison" data-action="togglePoisonWarning"> Poison key (duress)
        </label>
        <input type="text" id="fido2SkipKeys" placeholder="Skip keys (comma-separated labels)">
      </div>
      <div id="fido2PoisonWarning" class="hidden poison-warning">
        This key will contain RANDOM shard data. Including it during unlock causes silent failure.
        No data is destroyed. Exclude it and retry with real keys to unlock normally.
      </div>
    </div>
    <div id="fido2KeyListSection" style="margin-top:14px;"></div>
  </div>

  <!-- Snapshot Backup Card -->
  <div class="card hidden" id="backupCard" data-nav-label="Snapshots" data-workspace-section="recovery">
    <div class="eyebrow">Recovery & Access</div>
    <h2>Encrypted snapshots</h2>
    <p class="text-meta" style="margin-bottom:12px;">
      Export the local Sigillum data directory as a passphrase-encrypted snapshot, or restore one.
      Restoring replaces on-disk state and logs you out.
    </p>
    <div class="form-row">
      <input type="password" id="backupExportPass" placeholder="Export passphrase (min 8 chars)">
      <button class="btn-primary" data-action="exportSnapshot">Export Snapshot</button>
    </div>
    <div class="form-row">
      <input type="file" id="backupRestoreFile" style="flex:1;">
      <input type="password" id="backupRestorePass" placeholder="Restore passphrase" style="flex:1;">
      <button class="btn-danger" data-action="restoreSnapshot">Restore Snapshot</button>
    </div>
    <p class="card-note">Exports create an encrypted file you can store elsewhere. Restores replace this daemon data directory and require a fresh unlock afterward.</p>
    <div class="section-divider"></div>
    <div class="section-title">Start over on this machine</div>
    <p class="helper-text">Use this only when you intentionally want to wipe the local Sigillum data directory and return this daemon to first-run setup.</p>
    <div class="form-row">
      <input type="text" id="backupResetConfirm" placeholder="Type RESET LOCAL SIGILLUM DATA to reset">
      <button class="btn-danger" data-action="resetLocalData" data-arg0="backupResetConfirm">Reset local data</button>
    </div>
  </div>

  <!-- Audit Card -->
  <div class="card hidden" id="auditCard" data-nav-label="Audit" data-workspace-section="recovery">
    <div class="eyebrow">Recovery & Access</div>
    <h2>Audit trail</h2>
    <p class="helper-text">Recent local audit events from this daemon process and its persisted audit log.</p>
    <div id="auditList" class="text-meta">No events yet.</div>
  </div>

  <!-- Diagnostics Card -->
  <div class="card hidden" id="diagCard" data-nav-label="Diagnostics" data-workspace-section="recovery">
    <div class="eyebrow">Recovery & Access</div>
    <h2>Diagnostics</h2>
    <p class="helper-text">Low-level daemon health, queue, audit, and runtime policy details for debugging and operations.</p>
    <div class="stats" id="diagGrid">
      <div class="stat"><div class="value">-</div><div class="label">Version</div></div>
    </div>
  </div>
</main>

<footer>
  Sigillum local daemon &mdash; hardware-backed secret management &mdash;
  <a href="https://github.com/caelator/sigillum">GitHub</a>
</footer>
"##;

const INDEX_HTML_SCRIPT: &str = r##"
const API = '';
const SETUP_RESET_CONFIRMATION = 'RESET LOCAL SIGILLUM DATA';
const SESSION_TOKEN_KEY = 'sigillumSessionToken';
const REFRESH_INTERVAL_MS = 5000;
const OPERATOR_CARD_IDS = [
  'nextStepCard',
  'guideCard',
  'compartmentCard',
  'pushCard',
  'apiKeysCard',
  'secretsCard',
  'profilesCard',
  'xpubCard',
  'depositsCard',
  'queueCard',
  'maintenanceCard',
  'fido2Card',
  'backupCard',
  'auditCard',
  'diagCard',
];
const WORKSPACE_SECTION_KEY = 'sigillumWorkspaceSection';
const WORKSPACE_SECTIONS = [
  {
    id: 'overview',
    label: 'Overview',
    summary: 'Product framing, current status, and the highest-leverage next move.',
  },
  {
    id: 'access',
    label: 'Access',
    summary: 'Setup, unlock, session controls, and machine-level recovery.',
  },
  {
    id: 'vault',
    label: 'Vault',
    summary: 'Compartments, connection keys, encrypted secrets, and secret movement.',
  },
  {
    id: 'wallets',
    label: 'Wallets',
    summary: 'Providers, stealth wallets, and xpub receive-wallet setup.',
  },
  {
    id: 'operations',
    label: 'Operations',
    summary: 'Deposits, queue execution, and local maintenance cycles.',
  },
  {
    id: 'recovery',
    label: 'Recovery',
    summary: 'Hardware keys, snapshots, audit trail, and diagnostics.',
  },
];
let currentStatus = null;
let currentUiMode = 'loading';
let activeWorkspaceSection = 'overview';
let wizCompartments = [];
let customCompartments = [];
let wizRequiredKeyCount = 1;
let wizRegisteredKeyCount = 0;
let wizPrimaryKeyLabel = '';
let lastRefreshAt = null;
let refreshPromise = null;
let refreshQueued = false;
let refreshTimer = null;
let lastFidoDetect = null;
let lastProviderProfiles = [];
let lastWalletProfiles = [];
let lastXpubWalletProfiles = [];
let lastApiKeys = [];
let lastSecretKeys = [];
let lastDeposits = [];
let lastQueueJobs = [];
let lastFidoKeys = [];
let nextStepPrimaryTarget = null;
let nextStepSecondaryTarget = null;
let volatileSessionToken = null;

try {
  activeWorkspaceSection =
    window.sessionStorage.getItem(WORKSPACE_SECTION_KEY) || 'overview';
} catch (_) {}

const WIZARD_CHROME = {
  wizStep0: {
    pill: 'Step 1 of 3',
    title: 'Choose a protection model',
    summary: 'Pick the starting vault shape that best matches your risk level and how many hardware keys you want to manage.',
    checklist: [
      'Choose the plan that matches how many access layers you want on this machine.',
      'If you want hardware-key setup, attach a FIDO2 key and make sure you know its PIN.',
      'Decide whether you also want a fallback passphrase for local recovery.',
    ],
  },
  wizStepPassphrase: {
    pill: 'Step 2 of 3',
    title: 'Create your first local compartment',
    summary: 'Choose the compartment name and passphrase that will protect the first vault on this machine.',
    checklist: [
      'Pick the compartment name you want to see later in the unlocked workspace.',
      'Create a passphrase with at least 8 characters.',
      'Confirm it once so Sigillum can initialize the vault cleanly.',
    ],
  },
  wizStepCompartments: {
    pill: 'Step 2 of 3',
    title: 'Review what Sigillum will create',
    summary: 'Thresholds tell Sigillum how many distinct hardware keys are required to unlock each compartment.',
    checklist: [
      'Review each compartment label and the threshold attached to it.',
      'Use higher thresholds only for workflows that need stronger separation.',
      'Continue when the compartment plan matches your local operating model.',
    ],
  },
  wizStepCustomComps: {
    pill: 'Step 2 of 3',
    title: 'Design custom access layers',
    summary: 'Define the compartments you want first, then continue to hardware-key registration.',
    checklist: [
      'Add each compartment you want Sigillum to create in this local vault.',
      'Choose a unique threshold for every custom access layer.',
      'Continue once the list matches the workflow split you actually want.',
    ],
  },
  wizStepFido2Pin: {
    pill: 'Step 3 of 3',
    title: 'Register your first hardware key',
    summary: 'Leave the PIN blank for touch-only authenticators, or enter the current PIN for keys that require one, then give the key a label you will recognize later.',
    checklist: [
      'Leave the PIN field empty for touch-only keys, or enter the current FIDO2 PIN if the inserted key already requires one.',
      'Use a label you will recognize later, such as primary.',
      'Optionally set one fallback passphrase for local recovery if the keys are unavailable.',
    ],
  },
  wizStepAdditionalKeys: {
    pill: 'Add backup keys',
    title: 'Finish enrolling the keys this plan expects',
    summary: 'Higher-threshold compartments only become usable once enough distinct hardware keys are enrolled.',
    checklist: [
      'Insert the next hardware key you want to trust for this vault.',
      'Leave the PIN field empty for touch-only keys, or set and enter the current FIDO2 PIN only if you want that backup key to require one.',
      'Finish for now only if you are comfortable leaving the higher-threshold compartments unavailable until you add more keys later.',
    ],
  },
  wizStepTouch: {
    pill: 'Finishing setup',
    title: 'Complete the hardware-key touch',
    summary: 'Sigillum is waiting for a successful FIDO2 registration touch before it can finish the vault setup.',
    checklist: [
      'Keep the hardware key connected to this machine.',
      'Touch the device when it prompts for confirmation.',
      'Stay on this page until Sigillum confirms the vault is ready.',
    ],
  },
  wizStepDone: {
    pill: 'Vault ready',
    title: 'You can start using the daemon now',
    summary: 'Unlock the vault, store your first secret, and add more keys or operator profiles when you are ready.',
    checklist: [
      'Unlock once to confirm the setup behaves the way you expect.',
      'Store a first secret or connection key so the workspace becomes useful immediately.',
      'Add more keys, profiles, or deposits whenever you are ready for the next workflow.',
    ],
  },
};

function getSessionToken() {
  try {
    return window.sessionStorage.getItem(SESSION_TOKEN_KEY) || volatileSessionToken;
  } catch (_) {
    return volatileSessionToken;
  }
}

function setSessionToken(token) {
  if (!token) return;
  volatileSessionToken = token;
  try { window.sessionStorage.setItem(SESSION_TOKEN_KEY, token); }
  catch (_) {}
}

function clearSessionToken() {
  volatileSessionToken = null;
  try { window.sessionStorage.removeItem(SESSION_TOKEN_KEY); }
  catch (_) {}
}

function setHidden(id, hidden) {
  const el = document.getElementById(id);
  if (el) el.classList.toggle('hidden', hidden);
}

function setCardsHidden(ids, hidden) {
  ids.forEach(id => setHidden(id, hidden));
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) el.textContent = value;
}

function setTrustedHtml(id, value) {
  const el = document.getElementById(id);
  if (el) el.innerHTML = value;
}

function setStatusBadge(className, label) {
  const badge = document.getElementById('statusBadge');
  badge.className = 'status-badge ' + className;
  badge.textContent = label;
}

function formatRefreshTime(date) {
  return date.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function setRefreshMeta(label, state) {
  const el = document.getElementById('refreshMeta');
  if (!el) return;
  el.textContent = label;
  el.dataset.state = state;
}

function shouldAutoRefresh() {
  return document.visibilityState === 'visible';
}

function clearRefreshTimer() {
  if (!refreshTimer) return;
  clearTimeout(refreshTimer);
  refreshTimer = null;
}

function updateRefreshMeta(stateOverride) {
  if (stateOverride === 'busy') {
    setRefreshMeta('Syncing', 'busy');
    return;
  }
  if (stateOverride === 'error') {
    setRefreshMeta('Connection issue', 'error');
    return;
  }
  const prefix = shouldAutoRefresh() ? 'Live' : 'Paused';
  const label = lastRefreshAt
    ? prefix + ' · ' + formatRefreshTime(lastRefreshAt)
    : prefix;
  setRefreshMeta(label, shouldAutoRefresh() ? 'live' : 'paused');
}

function scheduleRefresh() {
  clearRefreshTimer();
  if (!shouldAutoRefresh()) {
    updateRefreshMeta();
    return;
  }
  updateRefreshMeta();
  refreshTimer = setTimeout(() => {
    void refresh();
  }, REFRESH_INTERVAL_MS);
}

function setSecretsAccess(unlocked) {
  setHidden('secretsLocked', unlocked);
  setHidden('secretsUnlocked', !unlocked);
}

function resetVaultCounts() {
  setText('apiKeyCount', '-');
  setText('secretCount', '-');
  setText('compartmentCount', '-');
}

function visibleWorkspaceCards() {
  return Array.from(document.querySelectorAll('main .card[data-workspace-section]'))
    .filter(card => !card.classList.contains('hidden'));
}

function availableWorkspaceSections() {
  const visibleSections = new Set(
    visibleWorkspaceCards()
      .map(card => card.dataset.workspaceSection)
      .filter(Boolean)
  );
  return WORKSPACE_SECTIONS.filter(section => visibleSections.has(section.id));
}

function firstVisibleCardInSection(sectionId) {
  return visibleWorkspaceCards().find(
    card => card.dataset.workspaceSection === sectionId
  );
}

function storeWorkspaceSection(sectionId) {
  try {
    window.sessionStorage.setItem(WORKSPACE_SECTION_KEY, sectionId);
  } catch (_) {}
}

function ensureActiveWorkspaceSection() {
  const sections = availableWorkspaceSections();
  if (!sections.length) {
    activeWorkspaceSection = 'overview';
    return;
  }
  if (!sections.some(section => section.id === activeWorkspaceSection)) {
    activeWorkspaceSection = sections[0].id;
    storeWorkspaceSection(activeWorkspaceSection);
  }
}

function syncWorkspaceSections() {
  ensureActiveWorkspaceSection();
  const targetSection = activeWorkspaceSection;
  document.querySelectorAll('main .card[data-workspace-section]').forEach(card => {
    card.classList.toggle(
      'section-hidden',
      Boolean(targetSection) && card.dataset.workspaceSection !== targetSection
    );
  });
}

function syncSectionNav() {
  const nav = document.getElementById('sectionNav');
  const main = document.querySelector('main');
  if (!nav) return;

  const sections = availableWorkspaceSections();
  if (sections.length <= 1) {
    nav.classList.add('hidden');
    nav.innerHTML = '';
    if (main) main.classList.remove('has-nav');
    syncWorkspaceSections();
    return;
  }

  ensureActiveWorkspaceSection();
  nav.innerHTML = sections.map(section =>
    '<button type="button" class="workspace-tab' +
      (section.id === activeWorkspaceSection ? ' active' : '') +
      '" data-action="selectWorkspaceSection" data-arg0="' + escAttr(section.id) + '">' +
      '<strong>' + esc(section.label) + '</strong>' +
      '<span>' + esc(section.summary) + '</span>' +
    '</button>'
  ).join('');
  nav.classList.remove('hidden');
  if (main) main.classList.add('has-nav');
  syncWorkspaceSections();
}

function selectWorkspaceSection(sectionId) {
  activeWorkspaceSection = sectionId;
  storeWorkspaceSection(sectionId);
  syncSectionNav();
  const firstCard = firstVisibleCardInSection(sectionId);
  if (firstCard) {
    firstCard.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
}

function jumpToCard(id) {
  const el = document.getElementById(id);
  if (!el || el.classList.contains('hidden')) return;
  const targetSection = el.dataset.workspaceSection;
  if (targetSection && targetSection !== activeWorkspaceSection) {
    activeWorkspaceSection = targetSection;
    storeWorkspaceSection(targetSection);
    syncSectionNav();
  }
  requestAnimationFrame(() => {
    if (!el.classList.contains('hidden') && !el.classList.contains('section-hidden')) {
      el.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }
  });
}

function heroPrimaryAction() {
  if (currentUiMode === 'setup') {
    jumpToCard('setupCard');
    return;
  }
  if (currentUiMode === 'locked') {
    jumpToCard('authCard');
    const input = document.getElementById('passphrase');
    if (input) input.focus();
    return;
  }
  jumpToCard('secretsCard');
}

function heroSecondaryAction() {
  if (currentUiMode === 'setup') {
    jumpToCard('setupCard');
    return;
  }
  if (currentUiMode === 'locked') {
    jumpToCard('authCard');
    switchUnlockTab('fido2');
    const input = document.getElementById('fido2Pin');
    if (input) input.focus();
    return;
  }
  jumpToCard('profilesCard');
}

function renderHeroContext(items) {
  return items.map(item =>
    '<div class="context-row"><strong>' + esc(item.title) + '</strong><span>' + esc(item.body) + '</span></div>'
  ).join('');
}

function renderChecklist(items) {
  return items.map((item, index) =>
    '<div class="checklist-item"><div class="checklist-mark">' +
      String(index + 1).padStart(2, '0') +
      '</div><div>' + esc(item) + '</div></div>'
  ).join('');
}

function renderNextStepItems(items) {
  return items.map(item =>
    '<div class="next-step-item"><strong>' + esc(item.title) + '</strong><span>' + esc(item.body) + '</span></div>'
  ).join('');
}

function setUnlockGuidance(mode) {
  const el = document.getElementById('authGuidance');
  if (!el) return;

  if (mode === 'session') {
    el.textContent = 'Lock All zeroizes unlocked vault material from daemon memory. Log Out Session only clears the browser session token and leaves daemon state unchanged.';
    return;
  }

  if (mode === 'fido2') {
    const deviceLine = lastFidoDetect && lastFidoDetect.device_present
      ? lastFidoDetect.device_count + ' hardware key(s) detected. '
      : '';
    el.textContent = deviceLine + 'Set tap count to the number of distinct key touches Sigillum should wait for. Leave the PIN field blank for touch-only authenticators, or enter the current PIN only for keys that require one. For a two-key compartment, enter 2 and touch two enrolled keys in sequence.';
    return;
  }

  el.textContent = 'Passphrase unlock works for passphrase-only vaults or fallback access. A successful unlock reveals the protected workspace in this browser tab only.';
}

function friendlyFidoError(message) {
  const raw = String(message || '');
  const normalized = raw.toLowerCase();

  if (
    normalized.includes('pin_required') ||
    normalized.includes('pin is required for the selected operation')
  ) {
    return 'This hardware key is configured to require its current FIDO2 PIN. Enter that PIN and retry, or use a touch-only key.';
  }
  if (
    normalized.includes('pin_not_set') ||
    normalized.includes('no pin has been set') ||
    normalized.includes('no fido2 pin is configured')
  ) {
    return 'This hardware key does not have a FIDO2 PIN yet. Leave the PIN field empty to keep using touch-only access, or use Set key PIN first if you want this key to require one.';
  }
  if (
    normalized.includes('pin already set') ||
    normalized.includes('already has a fido2 pin') ||
    normalized.includes('already has a pin configured')
  ) {
    return 'This hardware key already has a FIDO2 PIN. Enter the existing PIN when you use this key.';
  }
  if (
    normalized.includes('pin_policy') ||
    normalized.includes('pin policy') ||
    normalized.includes('at least 4 characters')
  ) {
    return 'That new FIDO2 PIN does not meet the key policy. Use at least 4 characters and avoid unsupported patterns.';
  }
  if (
    normalized.includes('pin_auth_blocked') ||
    normalized.includes('pin authentication is temporarily blocked') ||
    normalized.includes('power recycle')
  ) {
    return 'This hardware key has temporarily blocked PIN authentication. Unplug and reinsert the key, then retry with the correct PIN.';
  }
  if (
    normalized.includes('pin_blocked') ||
    normalized.includes('pin is fully blocked') ||
    normalized.includes('fully blocked on the hardware key')
  ) {
    return 'This hardware key has fully blocked PIN attempts. Recover or reset the key with vendor tooling before trying again.';
  }
  if (normalized.includes('incorrect pin') || normalized.includes('pin_invalid')) {
    return 'The hardware key rejected that PIN. Re-enter the current PIN carefully, or use a touch-only key if that is the policy you want.';
  }
  if (
    normalized.includes('cannot tell which one to use') ||
    normalized.includes('leave only the target key inserted')
  ) {
    return 'More than one hardware key is attached and this step needs a specific target. Leave only the key you want Sigillum to act on, then retry.';
  }
  if (
    normalized.includes('already appears to be registered') ||
    normalized.includes('insert the new key you want to add')
  ) {
    return 'The attached hardware keys all look like ones Sigillum already knows. Insert the new key you want to add, then retry.';
  }
  if (
    normalized.includes('matched the sigillum credential needed for this step') ||
    normalized.includes('matched the expected sigillum credential')
  ) {
    return 'The attached hardware keys do not include the enrolled key Sigillum expected for this step. Keep the required registered key connected and retry.';
  }
  if (normalized.includes('no fido2 device') || normalized.includes('no device')) {
    return 'No FIDO2 hardware key is currently available. Insert the key and try again.';
  }
  if (normalized.includes('timeout')) {
    return 'Sigillum timed out while waiting for the hardware key. Keep it connected and touch it when prompted, then try again.';
  }
  if (normalized.includes('ctap1 device') || normalized.includes('hmac-secret')) {
    return 'This key does not support the FIDO2 features Sigillum requires. Use a CTAP2 key with hmac-secret support.';
  }
  if (normalized.includes('clientpin support')) {
    return 'This key does not expose PIN management in a way Sigillum can use. Set the PIN with the vendor tool, then return here.';
  }
  return raw;
}

function isAlreadyUnlockedConflict(message) {
  return String(message || '').toLowerCase().includes('already unlocked');
}

function setInlineInfo(id, message, tone = 'error') {
  const el = document.getElementById(id);
  if (!el) return;
  const color = tone === 'success'
    ? 'var(--success)'
    : tone === 'warning'
      ? 'var(--warning)'
      : 'var(--danger)';
  el.textContent = message;
  el.style.color = color;
}

function nextStepPrimaryAction() {
  if (nextStepPrimaryTarget) jumpToCard(nextStepPrimaryTarget);
}

function nextStepSecondaryAction() {
  if (nextStepSecondaryTarget) jumpToCard(nextStepSecondaryTarget);
}

function updateNextStepCard() {
  const card = document.getElementById('nextStepCard');
  if (!card || currentUiMode !== 'unlocked') {
    setHidden('nextStepCard', true);
    return;
  }
  const hasStealthWalletProfiles = lastWalletProfiles.length > 0;
  const hasXpubWalletProfiles = lastXpubWalletProfiles.length > 0;

  let nextStep = {
    title: 'Choose the next concrete operation',
    summary: 'The vault is live. Use the cards below to run maintenance, inspect queue work, review audit history, and verify local daemon health.',
    items: [
      { title: 'Operations', body: 'Maintenance refreshes deposits and drains queue work with the current local policy settings.' },
      { title: 'Recovery', body: 'Snapshots, audit trail, and diagnostics help you validate and recover the local daemon state.' },
    ],
    primaryLabel: 'Open maintenance',
    primaryTarget: 'maintenanceCard',
    secondaryLabel: 'Review diagnostics',
    secondaryTarget: 'diagCard',
    note: 'This card stays focused on the next useful step instead of the whole workspace at once.',
  };

  if (lastSecretKeys.length === 0 && lastApiKeys.length === 0) {
    nextStep = {
      title: 'Store the first protected value',
      summary: 'The vault is unlocked, but it does not hold any working data yet. Start by storing a secret or a connection key so the daemon can do useful work locally.',
      items: [
        { title: 'Encrypted Secrets', body: 'Use this for values you want encrypted at rest and revealed only after unlock.' },
        { title: 'Connection Keys', body: 'Use this for RPC or auth tokens the daemon needs directly during profile or queue workflows.' },
      ],
      primaryLabel: 'Store a secret',
      primaryTarget: 'secretsCard',
      secondaryLabel: 'Store a connection key',
      secondaryTarget: 'apiKeysCard',
      note: 'Start with one real value. That is enough to prove the local vault flow end to end.',
    };
  } else if (lastProviderProfiles.length === 0) {
    nextStep = {
      title: 'Connect an EVM provider',
      summary: 'Profiles turn stored credentials into reusable operator configuration. Save an RPC endpoint and fee policy before setting up wallets or deposits.',
      items: [
        { title: 'Provider profile', body: 'This defines RPC URL, chain ID, optional auth token key, and fee defaults.' },
        { title: 'Why now', body: 'Wallet profiles, deposits, and queue work all depend on provider configuration being in place first.' },
      ],
      primaryLabel: 'Open profiles',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Review connection keys',
      secondaryTarget: 'apiKeysCard',
      note: 'If your provider needs an auth token, store that connection key first and then reference it from the provider profile.',
    };
  } else if (!hasStealthWalletProfiles && !hasXpubWalletProfiles) {
    nextStep = {
      title: 'Choose a wallet family',
      summary: 'Provider settings are ready. Next choose whether you want a stealth operator wallet for deposits and queue work, or an xpub receive wallet for public receive-tree export and address previews.',
      items: [
        { title: 'Stealth wallet', body: 'Use this when you want tracked deposits, sweep queues, and maintenance workflows today.' },
        { title: 'Xpub receive wallet', body: 'Use this when you want a public receive branch and deterministic receive-address previews without exposing private key material.' },
      ],
      primaryLabel: 'Open wallets',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Read operator guide',
      secondaryTarget: 'guideCard',
      note: 'Stealth is the current end-to-end operator path. Xpub is now available for receive-branch export and preview, but not yet for discovery or sweeping.',
    };
  } else if (!hasStealthWalletProfiles && hasXpubWalletProfiles) {
    nextStep = {
      title: 'Add a stealth wallet for live operator flows',
      summary: 'Your xpub receive profile is ready for public address derivation, but tracked deposits, sweep queues, and maintenance still run on stealth wallets today.',
      items: [
        { title: 'Keep xpub for receive trees', body: 'Use the xpub card to export receive branches and preview deposit addresses by index.' },
        { title: 'Add stealth for operations', body: 'Use the stealth wallet card when you want deposits, queue jobs, and maintenance cycles to run locally.' },
      ],
      primaryLabel: 'Open wallets',
      primaryTarget: 'profilesCard',
      secondaryLabel: 'Open xpub tools',
      secondaryTarget: 'xpubCard',
      note: 'This keeps the current product honest: xpub is live for receive-branch export, while stealth remains the operational wallet family.',
    };
  } else if (lastFidoKeys.length === 1) {
    nextStep = {
      title: 'Add a backup hardware key',
      summary: 'The workspace is usable, but a single enrolled hardware key is still a recovery risk. Register one more trusted key while the vault is already unlocked.',
      items: [
        { title: 'Primary plus backup', body: 'A second enrolled key gives you a much safer path if the primary device is lost or unavailable.' },
        { title: 'Unlock behavior', body: 'Higher-threshold compartments only become practical when you have enough enrolled keys to satisfy them.' },
      ],
      primaryLabel: 'Register another key',
      primaryTarget: 'fido2Card',
      secondaryLabel: 'Review snapshots',
      secondaryTarget: 'backupCard',
      note: 'If you intentionally rely on passphrase recovery, keep the snapshot flow healthy too.',
    };
  } else if (lastDeposits.length === 0) {
    nextStep = {
      title: 'Create the first tracked deposit',
      summary: 'Profiles are ready. The next meaningful test is generating a deposit address, monitoring it, and letting the daemon enqueue the follow-up work.',
      items: [
        { title: 'Native or ERC-20', body: 'Use the deposit card to create an address, set expected values, and decide whether sweep jobs should auto-queue.' },
        { title: 'Why this matters', body: 'This is the clearest end-to-end validation of wallet, provider, queue, and maintenance behavior.' },
      ],
      primaryLabel: 'Open deposits',
      primaryTarget: 'depositsCard',
      secondaryLabel: 'Open maintenance',
      secondaryTarget: 'maintenanceCard',
      note: 'After the first deposit exists, you can refresh balances and verify queue work without leaving the daemon.',
    };
  } else if (lastQueueJobs.length > 0) {
    nextStep = {
      title: 'Work the queue that is already waiting',
      summary: 'Sigillum has queued operator jobs. Review them first, then process a batch or run a full maintenance cycle if you want the daemon to move forward now.',
      items: [
        { title: 'Queue review', body: 'Inspect job kind, attempts, state, and recent errors before processing.' },
        { title: 'Maintenance option', body: 'Use maintenance when you want refresh, enqueue, and queue execution to happen as one cycle.' },
      ],
      primaryLabel: 'Open queue',
      primaryTarget: 'queueCard',
      secondaryLabel: 'Run maintenance',
      secondaryTarget: 'maintenanceCard',
      note: 'Queue processing changes live operator state, so review the job list before pushing a batch through.',
    };
  }

  nextStepPrimaryTarget = nextStep.primaryTarget;
  nextStepSecondaryTarget = nextStep.secondaryTarget;
  setText('nextStepTitle', nextStep.title);
  setText('nextStepSummary', nextStep.summary);
  setTrustedHtml('nextStepList', renderNextStepItems(nextStep.items));
  setText('nextStepNote', nextStep.note);
  const primaryBtn = document.getElementById('nextStepPrimaryBtn');
  const secondaryBtn = document.getElementById('nextStepSecondaryBtn');
  primaryBtn.textContent = nextStep.primaryLabel;
  secondaryBtn.textContent = nextStep.secondaryLabel;
  setHidden('nextStepCard', false);
}

function updateHeroState(mode, active, unlocked) {
  const primary = document.getElementById('heroPrimaryBtn');
  const secondary = document.getElementById('heroSecondaryBtn');

  if (mode === 'setup') {
    setText('statusEyebrow', 'First run');
    setText('statusTitle', 'Set up the vault');
    setText('statusSummary', 'This daemon is ready, but the machine does not have a vault yet. Finish setup first, then Sigillum will reveal the working operator surface below.');
    setText('heroModeValue', 'Setup required');
    setText('heroModeDetail', 'You only do this once per local data directory. Everything else stays quiet until setup is complete.');
    primary.textContent = 'Start setup';
    secondary.textContent = 'View options';
    setTrustedHtml('statusContext', renderHeroContext([
      { title: 'Pick a model', body: 'Start with Daily + Secure if you want the best default for one person managing everyday and higher-trust work.' },
      { title: 'Register access', body: 'Use a hardware key for stronger local protection, or choose passphrase-only if this machine will not use hardware keys.' },
      { title: 'Verify the path', body: 'When setup finishes, unlock once and store a first real value so the workflow is proven end to end.' },
    ]));
    return;
  }

  if (mode === 'locked') {
    setText('statusEyebrow', 'Locked state');
    setText('statusTitle', 'Unlock to continue');
    setText('statusSummary', 'Your local data is still on disk, but this browser session is not authenticated. Unlock to reach secrets, profiles, deposits, queue actions, backups, and diagnostics.');
    setText('heroModeValue', 'Locked');
    setText('heroModeDetail', 'Use the same passphrase or hardware-key threshold you configured during setup. The session token stays only in this tab.');
    primary.textContent = 'Unlock now';
    secondary.textContent = 'Use hardware key';
    setTrustedHtml('statusContext', renderHeroContext([
      { title: 'Passphrase path', body: 'Use this if you configured passphrase fallback or built a passphrase-only vault.' },
      { title: 'Hardware-key path', body: 'Use the hardware-key tab when a FIDO2 device is attached and you want threshold-based unlock.' },
      { title: 'Session scope', body: 'Unlock state lives in the daemon process and the session token lives only in this browser tab.' },
    ]));
    return;
  }

  const activeLabel = active ? (active.compartment_label || ('Compartment ' + active.compartment_id)) : 'No active compartment';
  setText('statusEyebrow', 'Unlocked workspace');
  setText('statusTitle', 'Local vault workspace');
  setText('statusSummary', 'The vault is unlocked. Use the workspace modes above to move between overview, access, vault, wallets, operations, and recovery without losing your place in one long page.');
  setText('heroModeValue', activeLabel);
  setText('heroModeDetail', unlocked.length > 1
    ? 'Multiple compartments are unlocked. Use the switcher in Access to choose which compartment new operations should target.'
    : 'One compartment is unlocked in this session. Additional compartments appear when their thresholds are met.');
  primary.textContent = 'Open secrets';
  secondary.textContent = 'Open profiles';
  setTrustedHtml('statusContext', renderHeroContext([
    { title: 'Protected values', body: 'Use Encrypted Secrets for sensitive data and Connection Keys for values the daemon needs during operator workflows.' },
    { title: 'Wallet families', body: 'Stealth wallets drive deposits and queue workflows today, while xpub receive wallets export public receive branches and preview deterministic addresses.' },
    { title: 'Operator loop', body: 'Deposits, queue, maintenance, snapshots, audit, and diagnostics now live in dedicated workspace modes instead of one scrolling operator page.' },
  ]));
}

function renderCompartmentSwitcher(unlocked, active) {
  const switcher = document.getElementById('compSwitcher');
  if (unlocked.length <= 1) {
    switcher.innerHTML = '';
    setHidden('compSwitcher', true);
    return;
  }

  let html = '';
  unlocked.forEach(c => {
    const isActive = active && active.compartment_id === c.id;
    html += '<button class="' + (isActive ? 'active' : '') + '" data-action="switchCompartment" data-arg0="' +
      escAttr(String(c.id)) + '" data-arg0-type="number">' + esc(c.label) + '</button>';
  });
  switcher.innerHTML = html;
  setHidden('compSwitcher', false);
}

function renderActiveCompartment(active, unlocked) {
  const compBadge = document.getElementById('compartmentBadge');
  if (active) {
    compBadge.textContent = active.compartment_label || ('Compartment ' + active.compartment_id);
    setHidden('compartmentBadge', false);
    setText('apiKeyCount', active.api_key_count || 0);
    setText('secretCount', active.secret_count != null ? active.secret_count : '(locked)');
  } else {
    setHidden('compartmentBadge', true);
    setText('apiKeyCount', '-');
    setText('secretCount', '-');
  }

  setText('compartmentCount', unlocked.length);
}

function applySetupUi() {
  currentUiMode = 'setup';
  document.body.dataset.mode = 'setup';
  wizRequiredKeyCount = 1;
  wizRegisteredKeyCount = 0;
  wizPrimaryKeyLabel = '';
  clearSessionToken();
  setStatusBadge('status-no-vault', 'NO VAULT');
  setHidden('compartmentBadge', true);
  setHidden('setupCard', false);
  setHidden('authCard', true);
  setCardsHidden(OPERATOR_CARD_IDS, true);
  setSecretsAccess(false);
  resetVaultCounts();
  setUnlockGuidance('passphrase');
  updateHeroState('setup');
  updateWizardChrome(document.querySelector('.wizard-step.active')?.id || 'wizStep0');
}

function applyLockedUi() {
  currentUiMode = 'locked';
  document.body.dataset.mode = 'locked';
  clearSessionToken();
  setStatusBadge('status-locked', 'LOCKED');
  setHidden('compartmentBadge', true);
  setCardsHidden(OPERATOR_CARD_IDS, true);
  resetVaultCounts();
  setHidden('lockForm', true);
  setHidden('authRecovery', false);
  setHidden('compSwitcher', true);
  setText('authTitle', 'Unlock this local session');
  setText('authLead', 'Unlock with the passphrase or hardware-key threshold you configured during setup. The resulting session token stays only in this browser tab.');
  setSecretsAccess(false);
  setUnlockGuidance('passphrase');
  updateHeroState('locked');
}

function applyUnlockedUi(active, unlocked) {
  currentUiMode = 'unlocked';
  document.body.dataset.mode = 'unlocked';
  setStatusBadge('status-unlocked', 'UNLOCKED');
  setHidden('unlockPassphrase', true);
  setHidden('unlockFido2', true);
  setHidden('unlockTabs', true);
  setHidden('lockForm', false);
  setHidden('authRecovery', true);
  setText('authTitle', 'Session controls');
  setText('authLead', 'This browser currently holds a valid local session token. Locking clears unlocked keys from daemon memory; logging out only clears this browser session.');
  setUnlockGuidance('session');

  renderCompartmentSwitcher(unlocked, active);
  renderActiveCompartment(active, unlocked);
  setSecretsAccess(true);

  setHidden('compartmentCard', false);
  setHidden('pushCard', unlocked.length < 2);
  if (unlocked.length >= 2) buildPushSelectors(unlocked);

  setHidden('guideCard', false);
  setHidden('profilesCard', false);
  setHidden('xpubCard', false);
  setHidden('depositsCard', false);
  setHidden('queueCard', false);
  setHidden('maintenanceCard', false);
  setHidden('backupCard', false);
  setHidden('auditCard', false);
  setHidden('diagCard', false);
  updateHeroState('unlocked', active, unlocked);
}

async function api(method, path, body) {
  const headers = { 'Content-Type': 'application/json' };
  const token = getSessionToken();
  if (token) headers.Authorization = 'Bearer ' + token;

  const opts = { method, headers };
  if (body) opts.body = JSON.stringify(body);
  const r = await fetch(API + path, opts);
  let payload = {};
  try {
    payload = await r.json();
  } catch (_) {}
  if (payload && payload.session_token) setSessionToken(payload.session_token);
  if (r.status === 401) clearSessionToken();
  return payload;
}

function toast(msg, type = 'success') {
  const el = document.createElement('div');
  el.className = 'toast toast-' + type;
  el.textContent = msg;
  el.setAttribute('role', 'status');
  el.setAttribute('aria-live', 'polite');
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 3000);
}

async function runRefreshCycle() {
  const s = await api('GET', '/api/status');
  currentStatus = s;
  const active = s.active_compartment;
  const unlocked = s.unlocked_compartments || [];

  // Not initialized — show setup wizard
  if (!s.initialized) {
    applySetupUi();
    syncSectionNav();
    wizDetectDevice();
    return;
  }

  setHidden('setupCard', true);
  setHidden('authCard', false);

  if (s.locked) {
    applyLockedUi();
    syncSectionNav();
    showUnlockTabs();
    return;
  }

  applyUnlockedUi(active, unlocked);

  await Promise.all([
    loadSecrets(),
    loadApiKeys(),
    loadProfiles(),
    loadDepositRegistry(),
    loadQueueJobs(),
    loadFido2(),
    loadCompartments(),
    loadAudit(),
    loadDiagnostics(),
  ]);
  updateNextStepCard();
  syncSectionNav();
}

async function refresh() {
  if (refreshPromise) {
    refreshQueued = true;
    return refreshPromise;
  }

  updateRefreshMeta('busy');
  refreshPromise = (async () => {
    try {
      await runRefreshCycle();
      lastRefreshAt = new Date();
      updateRefreshMeta();
    } catch (e) {
      console.error('refresh failed', e);
      updateRefreshMeta('error');
    } finally {
      const rerun = refreshQueued;
      refreshQueued = false;
      refreshPromise = null;

      if (rerun) {
        void refresh();
      } else {
        scheduleRefresh();
      }
    }
  })();

  return refreshPromise;
}

async function showUnlockTabs() {
  // Detect which unlock methods are available without revealing compartment info
  try {
    const detect = await api('GET', '/api/fido2/detect');
    const hasFido = detect.device_present;
    lastFidoDetect = detect;
    const tabs = document.getElementById('unlockTabs');
    const activeTab = document.getElementById('unlockFido2').classList.contains('hidden')
      ? 'passphrase'
      : 'fido2';
    if (hasFido) {
      tabs.classList.remove('hidden');
      setText('authLead', detect.device_count + ' hardware key(s) detected. Passphrase unlock stays available, or switch to Hardware Key when you want threshold-based unlock.');
      switchUnlockTab(activeTab);
    } else {
      tabs.classList.add('hidden');
      setText('authLead', 'No FIDO2 device is currently detected. Unlock with a passphrase, or attach a hardware key and refresh to use threshold-based unlock.');
      switchUnlockTab('passphrase');
    }
  } catch(e) {
    setText('authLead', 'Unlock with the passphrase you configured during setup. Hardware-key unlock becomes available when a FIDO2 device is detected.');
    switchUnlockTab('passphrase');
  }
}

function buildPushSelectors(unlocked) {
  const from = document.getElementById('pushFrom');
  const to = document.getElementById('pushTo');
  from.innerHTML = '';
  to.innerHTML = '';
  unlocked.forEach(c => {
    const opt1 = document.createElement('option');
    opt1.value = c.id;
    opt1.textContent = c.label + ' (#' + c.id + ')';
    from.appendChild(opt1);
    const opt2 = document.createElement('option');
    opt2.value = c.id;
    opt2.textContent = c.label + ' (#' + c.id + ')';
    to.appendChild(opt2);
  });
  if (unlocked.length > 1) to.selectedIndex = 1;
}

async function switchCompartment(id) {
  const r = await api('POST', '/api/compartment/switch', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Switched to compartment #' + id);
  refresh();
}

async function pushSecret() {
  const from = parseInt(document.getElementById('pushFrom').value);
  const to = parseInt(document.getElementById('pushTo').value);
  const key = document.getElementById('pushKey').value;
  const newKey = document.getElementById('pushNewKey').value || null;
  const tier = parseInt(document.getElementById('pushTier').value);
  if (!key) { toast('Key name required', 'error'); return; }
  if (from === to) { toast('Source and target must differ', 'error'); return; }
  const r = await api('POST', '/api/secrets/push', {
    from_compartment: from, to_compartment: to, key, new_key: newKey, tier,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  document.getElementById('pushKey').value = '';
  document.getElementById('pushNewKey').value = '';
  toast('Secret pushed');
  refresh();
}

async function loadCompartments() {
  try {
    const r = await api('GET', '/api/compartment/list');
    const el = document.getElementById('compartmentList');
    const comps = r.compartments || [];
    if (comps.length === 0) {
      el.innerHTML = '<p class="text-meta">No compartments available.</p>';
      return;
    }
    let html = '<ul class="key-list">';
    comps.forEach(c => {
      const active = c.is_active ? ' <span style="color:var(--success);">(active)</span>' : '';
      html += '<li><span>' + esc(c.label) + ' <span style="color:var(--text-dim);font-size:11px;">' +
        'threshold=' + c.threshold + active + '</span></span></li>';
    });
    html += '</ul>';
    el.innerHTML = html;
  } catch(e) {}
}

// ── Form helpers ──────────────────────────────────────────────

function clearFields(ids) {
  ids.forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
}

function renderEntityList(containerId, items, emptyMsg, renderItem) {
  const el = document.getElementById(containerId);
  if (!items.length) {
    el.innerHTML = '<p class="helper-text">' + esc(emptyMsg) + '</p>';
    return;
  }
  let html = '<ul class="entity-list">';
  items.forEach(item => { html += renderItem(item); });
  html += '</ul>';
  el.innerHTML = html;
}

function textValue(id) {
  return document.getElementById(id).value.trim();
}

function optionalTextValue(id) {
  const value = textValue(id);
  return value ? value : null;
}

function optionalNumberValue(id) {
  const value = textValue(id);
  if (!value) return null;
  const parsed = parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

function setSelectOptions(id, items, placeholder) {
  const el = document.getElementById(id);
  if (!el) return;
  const previous = el.value;
  let html = '';
  if (placeholder) {
    html += '<option value="">' + esc(placeholder) + '</option>';
  }
  items.forEach(item => {
    html += '<option value="' + escAttr(String(item.value)) + '">' + esc(item.label) + '</option>';
  });
  el.innerHTML = html;

  if (items.some(item => String(item.value) === previous)) {
    el.value = previous;
  } else if (!placeholder && items[0]) {
    el.value = String(items[0].value);
  } else {
    el.value = '';
  }
}

async function copyText(value, label) {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(value);
      toast(label + ' copied');
      return;
    }
  } catch (_) {}
  window.prompt('Copy ' + label + ':', value);
}

function formatTs(unix) {
  if (!unix) return '-';
  return new Date(unix * 1000).toLocaleString();
}

function pillClass(status) {
  const value = (status || '').toLowerCase();
  if (value.includes('fail') || value.includes('error')) return 'pill-danger';
  if (value.includes('ok') || value.includes('success') || value.includes('sent') || value.includes('broadcast')) return 'pill-good';
  if (value.includes('queue') || value.includes('funded') || value.includes('detected') || value.includes('processing') || value.includes('block') || value.includes('retry')) return 'pill-warn';
  return 'pill-neutral';
}

function statusPill(status) {
  const label = (status || 'unknown').replace(/_/g, ' ');
  return '<span class="pill ' + pillClass(status) + '">' + esc(label) + '</span>';
}

function showResultBox(id, html) {
  const el = document.getElementById(id);
  el.innerHTML = html;
  el.setAttribute('role', 'status');
  el.setAttribute('aria-live', 'polite');
  el.classList.remove('hidden');
}

function describeQueueJob(job) {
  const kind = job.kind || 'unknown';
  switch (kind) {
    case 'eth_stealth_transfer':
      return 'native transfer · ' + (job.wallet_profile || '-') + ' · to ' + (job.destination_address || job.to_address || '-');
    case 'eth_stealth_erc20_transfer':
      return 'erc20 transfer · ' + (job.wallet_profile || '-') + ' · token ' + (job.token_address || '-');
    case 'eth_stealth_native_sweep':
      return 'native sweep · ' + (job.wallet_profile || '-') + ' · ' + (job.destination_address || 'wallet default');
    case 'eth_stealth_erc20_sweep':
      return 'erc20 sweep · ' + (job.wallet_profile || '-') + ' · token ' + (job.token_address || '-');
    default:
      return kind;
  }
}

function queueScheduleLine(job) {
  if (job.next_attempt_after_unix) {
    return 'nextAttempt=' + formatTs(job.next_attempt_after_unix);
  }
  if ((job.state || '').toLowerCase().includes('retry')) {
    return 'nextAttempt=manual-or-immediate';
  }
  return 'nextAttempt=-';
}

function depositObservedLine(deposit) {
  const observedAmount = deposit.observed_amount_hex || '-';
  const nativeBalance = deposit.observed_native_balance_wei_hex || '-';
  return 'expected=' + esc(deposit.expected_amount_hex || '-') +
    ' · observed=' + esc(observedAmount) +
    ' · native=' + esc(nativeBalance);
}

function renderProviderProfiles(profiles) {
  renderEntityList('providerProfileList', profiles, 'No provider profiles yet. Save an RPC endpoint and fee policy above to let deposits and queue work talk to a chain.', profile => {
    const feeInfo = 'priority=' + (profile.max_priority_fee_per_gas_hex || '-') +
      ' · max=' + (profile.max_fee_per_gas_hex || '-') +
      ' · nativeGas=' + (profile.native_gas_limit || '-') +
      ' · erc20Gas=' + (profile.erc20_gas_limit || '-');
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'rpc=' + esc(profile.rpc_url) + '<br>' +
      'chain=' + esc(String(profile.chain_id)) +
      ' · compartment=' + esc(String(profile.compartment_id)) +
      ' · authKey=' + esc(profile.auth_token_key || '-') + '<br>' +
      esc(feeInfo) +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(profile.rpc_url) + '" data-arg1="RPC URL">Copy RPC</button>' +
      '<button class="btn-danger" data-action="deleteProviderProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderWalletProfiles(profiles) {
  renderEntityList('walletProfileList', profiles, 'No wallet profiles yet. Create one above to bind a Sigillum wallet label to a provider before you generate deposits.', profile => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'wallet=' + esc(profile.wallet) +
      ' · short=' + esc(profile.short_name) +
      ' · provider=' + esc(profile.provider_profile) + '<br>' +
      'compartment=' + esc(String(profile.compartment_id)) +
      ' · chain=' + esc(profile.chain_id != null ? String(profile.chain_id) : '-') +
      ' · defaultDestination=' + esc(profile.default_destination_address || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="exportWalletMeta" data-arg0="' + escAttr(profile.wallet) + '" data-arg1="' + escAttr(profile.short_name) + '">Export Meta</button>' +
      '<button class="btn-danger" data-action="deleteWalletProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

function renderXpubWalletProfiles(profiles) {
  renderEntityList('xpubWalletProfileList', profiles, 'No xpub wallet profiles yet. Save one above when you want a public receive tree without exposing private key material.', profile => {
    const accountPath = "m/44'/60'/" + profile.project_account + "'";
    const receivePath = accountPath + '/0';
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(profile.name) + '</div>' +
      '<div class="entity-meta">' +
      'projectAccount=' + esc(String(profile.project_account)) +
      ' · provider=' + esc(profile.provider_profile) + '<br>' +
      'accountPath=' + esc(accountPath) +
      ' · receivePath=' + esc(receivePath) + '<br>' +
      'compartment=' + esc(String(profile.compartment_id)) +
      ' · chain=' + esc(profile.chain_id != null ? String(profile.chain_id) : '-') +
      ' · defaultDestination=' + esc(profile.default_destination_address || '-') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="exportXpubWalletProfile" data-arg0="' + escAttr(profile.name) + '">Export Xpub</button>' +
      '<button class="btn-danger" data-action="deleteXpubWalletProfile" data-arg0="' + escAttr(profile.name) + '">Delete</button>' +
      '</div></li>';
  });
}

async function loadProfiles() {
  try {
    const [providerResp, walletResp, xpubResp] = await Promise.all([
      api('GET', '/api/profiles/evm'),
      api('GET', '/api/profiles/eth-stealth'),
      api('GET', '/api/profiles/eth-xpub'),
    ]);
    if (providerResp.error || walletResp.error || xpubResp.error) return;

    const providers = providerResp.profiles || [];
    const wallets = walletResp.profiles || [];
    const xpubWallets = xpubResp.profiles || [];
    lastProviderProfiles = providers;
    lastWalletProfiles = wallets;
    lastXpubWalletProfiles = xpubWallets;

    renderProviderProfiles(providers);
    renderWalletProfiles(wallets);
    renderXpubWalletProfiles(xpubWallets);

    setSelectOptions(
      'walletProviderProfile',
      providers.map(profile => ({
        value: profile.name,
        label: profile.name + ' · chain ' + profile.chain_id,
      })),
      providers.length ? 'Select provider profile' : 'No provider profiles available'
    );
    setSelectOptions(
      'xpubProviderProfile',
      providers.map(profile => ({
        value: profile.name,
        label: profile.name + ' · chain ' + profile.chain_id,
      })),
      providers.length ? 'Select provider profile' : 'No provider profiles available'
    );

    const walletOptions = wallets.map(profile => ({
      value: profile.name,
      label: profile.name + ' · ' + profile.wallet,
    }));
    setSelectOptions(
      'depositNativeWalletProfile',
      walletOptions,
      wallets.length ? 'Select wallet profile' : 'No wallet profiles available'
    );
    setSelectOptions(
      'depositErc20WalletProfile',
      walletOptions,
      wallets.length ? 'Select wallet profile' : 'No wallet profiles available'
    );
    setSelectOptions(
      'xpubPreviewProfile',
      xpubWallets.map(profile => ({
        value: profile.name,
        label: profile.name + ' · account ' + profile.project_account,
      })),
      xpubWallets.length ? 'Select xpub profile' : 'No xpub profiles available'
    );
  } catch (e) {}
}

async function upsertProviderProfile() {
  const name = textValue('providerName');
  const rpcUrl = textValue('providerRpcUrl');
  const chainId = parseInt(textValue('providerChainId'), 10);
  if (!name || !rpcUrl || !chainId) {
    toast('Provider name, RPC URL, and chain ID are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/evm/upsert', {
    name,
    rpc_url: rpcUrl,
    auth_token_key: optionalTextValue('providerAuthTokenKey'),
    compartment_id: optionalNumberValue('providerCompartmentId'),
    chain_id: chainId,
    max_priority_fee_per_gas_hex: optionalTextValue('providerMaxPriorityFee'),
    max_fee_per_gas_hex: optionalTextValue('providerMaxFee'),
    native_gas_limit: optionalNumberValue('providerNativeGasLimit'),
    erc20_gas_limit: optionalNumberValue('providerErc20GasLimit'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields(['providerName', 'providerRpcUrl', 'providerAuthTokenKey',
    'providerCompartmentId', 'providerMaxPriorityFee', 'providerMaxFee',
    'providerNativeGasLimit', 'providerErc20GasLimit']);
  toast('Provider profile saved');
  refresh();
}

async function deleteProviderProfile(name) {
  if (!confirm('Delete provider profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/evm/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Provider profile deleted');
  refresh();
}

async function upsertWalletProfile() {
  const name = textValue('walletProfileName');
  const wallet = textValue('walletLabel');
  const providerProfile = textValue('walletProviderProfile');
  if (!name || !wallet || !providerProfile) {
    toast('Wallet profile name, wallet label, and provider profile are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/eth-stealth/upsert', {
    name,
    wallet,
    short_name: optionalTextValue('walletShortName'),
    provider_profile: providerProfile,
    compartment_id: optionalNumberValue('walletCompartmentId'),
    chain_id: optionalNumberValue('walletChainId'),
    default_destination_address: optionalTextValue('walletDefaultDestination'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields(['walletProfileName', 'walletLabel', 'walletShortName',
    'walletCompartmentId', 'walletChainId', 'walletDefaultDestination']);
  toast('Wallet profile saved');
  refresh();
}

async function deleteWalletProfile(name) {
  if (!confirm('Delete wallet profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/eth-stealth/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Wallet profile deleted');
  refresh();
}

async function upsertXpubWalletProfile() {
  const name = textValue('xpubProfileName');
  const providerProfile = textValue('xpubProviderProfile');
  const projectAccount = parseInt(textValue('xpubProjectAccount'), 10);
  if (!name || !providerProfile || !Number.isInteger(projectAccount) || projectAccount < 0) {
    toast('Profile name, provider profile, and a non-negative project account are required', 'error');
    return;
  }

  const r = await api('POST', '/api/profiles/eth-xpub/upsert', {
    name,
    project_account: projectAccount,
    provider_profile: providerProfile,
    compartment_id: optionalNumberValue('xpubCompartmentId'),
    chain_id: optionalNumberValue('xpubChainId'),
    default_destination_address: optionalTextValue('xpubDefaultDestination'),
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearFields([
    'xpubProfileName',
    'xpubCompartmentId',
    'xpubChainId',
    'xpubDefaultDestination',
  ]);
  document.getElementById('xpubProjectAccount').value = '0';
  toast('Xpub wallet profile saved');
  refresh();
}

async function deleteXpubWalletProfile(name) {
  if (!confirm('Delete xpub wallet profile "' + name + '"?')) return;
  const r = await api('POST', '/api/profiles/eth-xpub/delete', { name });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Xpub wallet profile deleted');
  refresh();
}

async function exportSelectedXpubWallet() {
  const walletProfile = textValue('xpubPreviewProfile');
  if (!walletProfile) {
    toast('Choose an xpub wallet profile first', 'error');
    return;
  }
  await exportXpubWalletProfile(walletProfile);
}

async function exportXpubWalletProfile(walletProfile) {
  const r = await api('POST', '/api/wallets/eth-xpub/export', {
    wallet_profile: walletProfile,
  });
  if (r.error) { toast(r.error, 'error'); return; }

  const exportedXpub = r.receive_xpub || '';
  document.getElementById('xpubPreviewProfile').value = walletProfile;
  document.getElementById('xpubReceiveXpub').value = exportedXpub;

  showResultBox(
    'xpubExportResult',
    '<strong>' + esc(walletProfile) + '</strong><br>' +
      'accountPath=' + esc(r.account_path || '-') + '<br>' +
      'receivePath=' + esc(r.receive_path || '-') + '<br>' +
      'xpub=' + esc(exportedXpub) + '<br>' +
      '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(exportedXpub) + '" data-arg1="Receive branch xpub">Copy Xpub</button>' +
      '</div>'
  );

  await previewXpubReceiveAddress();
}

async function previewXpubReceiveAddress() {
  const xpub = textValue('xpubReceiveXpub');
  const index = parseInt(textValue('xpubPreviewIndex'), 10);
  if (!xpub) {
    toast('Export or paste a receive-branch xpub first', 'error');
    return;
  }
  if (!Number.isInteger(index) || index < 0) {
    toast('Receive index must be a non-negative number', 'error');
    return;
  }

  const r = await api('POST', '/api/wallets/eth-xpub/derive', { xpub, index });
  if (r.error) { toast(r.error, 'error'); return; }

  showResultBox(
    'xpubPreviewResult',
    '<strong>Receive index ' + esc(String(r.index)) + '</strong><br>' +
      'path=' + esc('receive/' + r.index) + '<br>' +
      'address=' + esc(r.address) + '<br>' +
      '<div style="margin-top:10px;display:flex;gap:8px;flex-wrap:wrap;">' +
        '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(r.address) + '" data-arg1="Receive address">Copy Address</button>' +
      '</div>'
  );
}

async function exportWalletMeta(wallet, shortName) {
  const r = await api('POST', '/api/wallets/eth-stealth/export', {
    wallet,
    short_name: shortName || null,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  await copyText(r.stealth_meta_address, 'Stealth meta-address');
}

function renderDeposits(deposits) {
  renderEntityList('depositList', deposits, 'No tracked deposits yet. Create a native or ERC-20 deposit above to start monitoring incoming funds and queue follow-up work.', deposit => {
    const queueInfo = deposit.queue_job_id
      ? 'job=' + deposit.queue_job_id + ' · state=' + (deposit.queue_job_state || '-')
      : 'job=-';
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(deposit.id) + ' ' + statusPill(deposit.status) + '</div>' +
      '<div class="entity-meta">' +
      'walletProfile=' + esc(deposit.wallet_profile) +
      ' · asset=' + esc(deposit.asset_kind) +
      ' · short=' + esc(deposit.short_name) + '<br>' +
      'stealth=' + esc(deposit.stealth_address) + '<br>' +
      'ephemeral=' + esc(deposit.ephemeral_public_key_hex) +
      ' · viewTag=' + esc(deposit.view_tag_hex) + '<br>' +
      depositObservedLine(deposit) + '<br>' +
      'token=' + esc(deposit.token_address || '-') +
      ' · autoQueue=' + esc(String(deposit.auto_queue_sweep)) +
      ' · ' + esc(queueInfo) + '<br>' +
      'created=' + esc(formatTs(deposit.created_at_unix)) +
      ' · checked=' + esc(formatTs(deposit.last_checked_at_unix)) +
      ' · updated=' + esc(formatTs(deposit.updated_at_unix)) +
      (deposit.note ? '<br>note=' + esc(deposit.note) : '') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-ghost" data-action="copyText" data-arg0="' + escAttr(deposit.stealth_address) + '" data-arg1="Deposit address">Copy Address</button>' +
      '<button class="btn-ghost" data-action="refreshSingleDeposit" data-arg0="' + escAttr(deposit.id) + '">Refresh</button>' +
      '<button class="btn-success" data-action="enqueueDepositSweep" data-arg0="' + escAttr(deposit.id) + '">Queue Sweep</button>' +
      '<button class="btn-danger" data-action="deleteDeposit" data-arg0="' + escAttr(deposit.id) + '">Delete</button>' +
      '</div></li>';
  });
}

async function loadDepositRegistry() {
  try {
    const r = await api('GET', '/api/deposits/eth-stealth');
    if (r.error) return;
    lastDeposits = r.deposits || [];
    renderDeposits(lastDeposits);
  } catch (e) {}
}

async function createNativeDeposit() {
  const walletProfile = textValue('depositNativeWalletProfile');
  if (!walletProfile) {
    toast('Select a wallet profile first', 'error');
    return;
  }
  const r = await api('POST', '/api/deposits/eth-stealth/create-native', {
    wallet_profile: walletProfile,
    expected_value_wei_hex: optionalTextValue('depositNativeExpected'),
    auto_queue_sweep: document.getElementById('depositNativeAutoQueue').checked,
    sweep_destination_address: optionalTextValue('depositNativeDestination'),
    min_sweep_value_wei_hex: optionalTextValue('depositNativeMinSweep'),
    note: optionalTextValue('depositNativeNote'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['depositNativeExpected', 'depositNativeMinSweep',
    'depositNativeDestination', 'depositNativeNote']);
  toast('Native deposit created');
  refresh();
}

async function createErc20Deposit() {
  const walletProfile = textValue('depositErc20WalletProfile');
  const tokenAddress = textValue('depositErc20TokenAddress');
  if (!walletProfile || !tokenAddress) {
    toast('Wallet profile and token address are required', 'error');
    return;
  }
  const r = await api('POST', '/api/deposits/eth-stealth/create-erc20', {
    wallet_profile: walletProfile,
    token_address: tokenAddress,
    expected_amount_hex: optionalTextValue('depositErc20Expected'),
    auto_queue_sweep: document.getElementById('depositErc20AutoQueue').checked,
    sweep_destination_address: optionalTextValue('depositErc20Destination'),
    min_sweep_amount_hex: optionalTextValue('depositErc20MinSweep'),
    note: optionalTextValue('depositErc20Note'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['depositErc20TokenAddress', 'depositErc20Expected',
    'depositErc20MinSweep', 'depositErc20Destination', 'depositErc20Note']);
  toast('ERC-20 deposit created');
  refresh();
}

async function refreshDepositRegistry() {
  const r = await api('POST', '/api/deposits/eth-stealth/refresh', {
    id: null,
    limit: optionalNumberValue('depositRefreshLimit'),
    auto_enqueue: document.getElementById('depositRefreshAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0))
  );
  lastDeposits = r.deposits || [];
  renderDeposits(lastDeposits);
  updateNextStepCard();
  toast('Deposits refreshed');
  loadQueueJobs();
}

async function refreshSingleDeposit(id) {
  const r = await api('POST', '/api/deposits/eth-stealth/refresh', {
    id,
    limit: 1,
    auto_enqueue: document.getElementById('depositRefreshAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0)) +
    ' · target=' + esc(id)
  );
  lastDeposits = r.deposits || [];
  renderDeposits(lastDeposits);
  updateNextStepCard();
  loadQueueJobs();
}

async function enqueueDepositSweep(id) {
  const r = await api('POST', '/api/deposits/eth-stealth/enqueue-sweep', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'depositRefreshResult',
    'queued sweep for deposit ' + esc(id) + ' · job=' + esc(r.job?.id || '-')
  );
  toast('Deposit sweep queued');
  refresh();
}

async function deleteDeposit(id) {
  if (!confirm('Delete deposit "' + id + '"?')) return;
  const r = await api('POST', '/api/deposits/eth-stealth/delete', { id });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deposit deleted');
  refresh();
}

function renderQueueJobs(jobs) {
  renderEntityList('queueList', jobs, 'Queue is empty. Once deposits enqueue sweeps or you create manual work, jobs will appear here for review and processing.', job => {
    return '<li><div class="entity-main">' +
      '<div class="entity-title">' + esc(job.id) + ' ' + statusPill(job.state) + '</div>' +
      '<div class="entity-meta">' +
      'kind=' + esc(job.kind || '-') +
      ' · attempts=' + esc(String(job.attempts || 0)) + '<br>' +
      esc(describeQueueJob(job)) + '<br>' +
      'created=' + esc(formatTs(job.created_at_unix)) +
      ' · updated=' + esc(formatTs(job.updated_at_unix)) +
      ' · ' + esc(queueScheduleLine(job)) + '<br>' +
      'tx=' + esc(job.transaction_hash_hex || '-') +
      ' · broadcast=' + esc(job.broadcast_transaction_hash_hex || '-') +
      (job.last_error ? '<br>lastError=' + esc(job.last_error) : '') +
      '</div></div>' +
      '<div class="entity-actions">' +
      '<button class="btn-primary" data-action="processQueueJob" data-arg0="' + escAttr(job.id) + '">Process</button>' +
      '</div></li>';
  });
}

async function loadQueueJobs() {
  try {
    const r = await api('GET', '/api/queue/jobs');
    if (r.error) return;
    lastQueueJobs = r.jobs || [];
    renderQueueJobs(lastQueueJobs);
  } catch (e) {}
}

async function processQueueBatch() {
  const r = await api('POST', '/api/queue/process', {
    id: null,
    limit: optionalNumberValue('queueProcessLimit'),
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'queueProcessResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0))
  );
  lastQueueJobs = r.jobs || [];
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  loadDepositRegistry();
  toast('Queue processed');
}

async function processQueueJob(id) {
  const r = await api('POST', '/api/queue/process', { id, limit: 1 });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'queueProcessResult',
    'processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0)) +
    ' · target=' + esc(id)
  );
  lastQueueJobs = r.jobs || [];
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  loadDepositRegistry();
}

async function runMaintenanceCycle() {
  const r = await api('POST', '/api/maintenance/run', {
    deposit_refresh_limit: optionalNumberValue('maintenanceDepositLimit'),
    queue_process_limit: optionalNumberValue('maintenanceQueueLimit'),
    auto_enqueue: document.getElementById('maintenanceAutoEnqueue').checked,
  });
  if (r.error) { toast(r.error, 'error'); return; }
  showResultBox(
    'maintenanceResult',
    'refreshed=' + esc(String(r.refreshed || 0)) +
    ' · detected=' + esc(String(r.detected || 0)) +
    ' · queued=' + esc(String(r.queued || 0)) +
    ' · processed=' + esc(String(r.processed || 0)) +
    ' · succeeded=' + esc(String(r.succeeded || 0)) +
    ' · blocked=' + esc(String(r.blocked || 0)) +
    ' · retrying=' + esc(String(r.retrying || 0)) +
    ' · failed=' + esc(String(r.failed || 0))
  );
  lastDeposits = r.deposits || [];
  lastQueueJobs = r.jobs || [];
  renderDeposits(lastDeposits);
  renderQueueJobs(lastQueueJobs);
  updateNextStepCard();
  toast('Maintenance cycle complete');
}

async function loadFido2() {
  const fido2Card = document.getElementById('fido2Card');
  if (!currentStatus || currentStatus.locked) {
    fido2Card.classList.add('hidden');
    return;
  }
  fido2Card.classList.remove('hidden');

  try {
    const detect = await api('GET', '/api/fido2/detect');
    const devEl = document.getElementById('fido2DeviceStatus');
    if (detect.device_present) {
      devEl.innerHTML = '<span style="color:var(--success);">' + detect.device_count +
        ' FIDO2 device(s) connected</span>';
    } else {
      devEl.innerHTML = '<span style="color:var(--warning);">No FIDO2 device detected.</span>';
    }
  } catch(e) {}

  try {
    const keys = await api('GET', '/api/fido2/list');
    const listEl = document.getElementById('fido2KeyListSection');
    lastFidoKeys = keys.keys || [];
    if (keys.keys && keys.keys.length > 0) {
      let html = '<ul class="key-list">';
      keys.keys.forEach(k => {
        html += '<li><span>' + esc(k.label) + ' <span style="color:var(--text-dim);font-size:11px;">(' +
          esc(k.credential_id_short) + '...) ' + esc(k.registered_at) + '</span></span>' +
          '<div class="key-actions"><button class="btn-danger" data-action="fido2RemoveKey" data-arg0="' +
          escAttr(k.label) + '">Remove</button></div></li>';
      });
      html += '</ul>';
      listEl.innerHTML = html;
    } else {
      listEl.innerHTML = '<p class="text-meta">No additional hardware keys are registered yet. Add one above to improve recovery and higher-threshold unlock paths.</p>';
    }
  } catch(e) {}
}

function togglePoisonWarning() {
  const checked = document.getElementById('fido2Poison').checked;
  document.getElementById('fido2PoisonWarning').classList.toggle('hidden', !checked);
}

async function submitNewFido2Pin(pinId, confirmId, hintId, copyToId, focusId) {
  const pin = document.getElementById(pinId).value;
  const confirmPin = document.getElementById(confirmId).value;
  if (!pin) { toast('New PIN required', 'error'); return; }
  if (pin.length < 4) { toast('New PIN must be at least 4 characters', 'error'); return; }
  if (pin !== confirmPin) { toast('PIN entries do not match', 'error'); return; }

  const r = await api('POST', '/api/fido2/pin/set', { new_pin: pin });
  if (r.error) {
    const message = friendlyFidoError(r.error);
    if (hintId) setInlineInfo(hintId, message);
    toast(message, 'error');
    return;
  }

  if (copyToId) {
    const target = document.getElementById(copyToId);
    if (target) target.value = pin;
  }
  clearFields([pinId, confirmId]);
  if (hintId) {
    setInlineInfo(
      hintId,
      'FIDO2 PIN set on the inserted hardware key. Use that PIN in the registration field and continue.',
      'success'
    );
  }
  const focusTarget = document.getElementById(focusId);
  if (focusTarget) focusTarget.focus();
  toast('Hardware-key PIN set');
}

async function fido2SetNewPin() {
  await submitNewFido2Pin(
    'fido2NewPin',
    'fido2NewPinConfirm',
    'fido2DeviceStatus',
    'fido2RegPin',
    'fido2RegLabel'
  );
}

async function fido2Register() {
  const pin = document.getElementById('fido2RegPin').value;
  const label = document.getElementById('fido2RegLabel').value;
  const poison = document.getElementById('fido2Poison').checked;
  const skipRaw = document.getElementById('fido2SkipKeys').value.trim();
  const skipKeys = skipRaw ? skipRaw.split(',').map(s => s.trim()).filter(Boolean) : [];
  if (!label) { toast('Label required', 'error'); return; }
  if (poison && !confirm('Register "' + label + '" as a POISON key? Including it during unlock will cause silent failure.')) return;
  toast('Touch your FIDO2 key now...');
  const body = { label };
  if (pin) body.pin = pin;
  if (poison) body.poison = true;
  if (skipKeys.length > 0) body.skip_keys = skipKeys;
  const r = await api('POST', '/api/fido2/register', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('fido2DeviceStatus', message);
    toast(message, 'error');
    return;
  }
  clearFields(['fido2RegPin', 'fido2RegLabel', 'fido2SkipKeys']);
  document.getElementById('fido2Poison').checked = false;
  togglePoisonWarning();
  toast('Key "' + label + '" registered' + (poison ? ' (poison)' : ''));
  refresh();
}

async function fido2RemoveKey(label) {
  if (!confirm('Remove FIDO2 key "' + label + '"?')) return;
  const pin = await promptPin('Enter the current FIDO2 PIN only if the remaining keys require one:');
  const body = { label };
  if (pin) body.pin = pin;
  const r = await api('POST', '/api/fido2/remove', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('fido2DeviceStatus', message);
    toast(message, 'error');
    return;
  }
  toast('Key removed');
  refresh();
}

function promptPin(msg) {
  return new Promise(resolve => {
    const overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;inset:0;background:rgba(0,0,0,0.7);z-index:200;display:flex;align-items:center;justify-content:center;';
    overlay.innerHTML = '<div class="card pin-modal"><h2>' + esc(msg) + '</h2>' +
      '<div class="form-row"><input type="password" id="pinModalInput" placeholder="Current PIN (leave blank if not required)">' +
      '<button class="btn-primary" id="pinModalOk">OK</button></div></div>';
    document.body.appendChild(overlay);
    const inp = document.getElementById('pinModalInput');
    inp.focus();
    const done = () => { const v = inp.value; overlay.remove(); resolve(v || null); };
    document.getElementById('pinModalOk').addEventListener('click', done);
    inp.addEventListener('keydown', e => {
      if (e.key === 'Enter') done();
      if (e.key === 'Escape') {
        overlay.remove();
        resolve(null);
      }
    });
    overlay.addEventListener('click', e => {
      if (e.target === overlay) {
        overlay.remove();
        resolve(null);
      }
    });
  });
}

function switchUnlockTab(tab) {
  document.querySelectorAll('.unlock-tab').forEach(t => t.classList.remove('active'));
  if (tab === 'fido2') {
    document.getElementById('unlockPassphrase').classList.add('hidden');
    document.getElementById('unlockFido2').classList.remove('hidden');
    document.querySelectorAll('.unlock-tab')[1].classList.add('active');
    setUnlockGuidance('fido2');
  } else {
    document.getElementById('unlockPassphrase').classList.remove('hidden');
    document.getElementById('unlockFido2').classList.add('hidden');
    document.querySelectorAll('.unlock-tab')[0].classList.add('active');
    setUnlockGuidance('passphrase');
  }
}

async function unlock() {
  const p = document.getElementById('passphrase').value;
  if (!p) return;
  const r = await api('POST', '/api/unlock', { passphrase: p });
  if (r.error) {
    if (isAlreadyUnlockedConflict(r.error)) {
      toast('Session already active. Refreshing workspace…');
      await refresh();
      return;
    }
    toast(r.error, 'error');
    return;
  }
  document.getElementById('passphrase').value = '';
  if (r.unlocked_compartments && r.unlocked_compartments.length > 0) {
    const labels = r.unlocked_compartments.map(c => c.label).join(', ');
    toast('Unlocked: ' + labels);
  } else {
    toast('Unlocked');
  }
  refresh();
}

async function fido2Unlock() {
  const pin = document.getElementById('fido2Pin').value;
  const tapCount = parseInt(document.getElementById('fido2TapCount').value);
  if (!tapCount || tapCount < 1) { toast('Enter number of keys', 'error'); return; }
  toast('Touch your hardware key now...');
  const r = await api('POST', '/api/fido2/unlock', { pins: pin ? [pin] : [], tap_count: tapCount });
  if (r.error) {
    if (isAlreadyUnlockedConflict(r.error)) {
      toast('Session already active. Refreshing workspace…');
      await refresh();
      return;
    }
    const message = friendlyFidoError(r.error);
    setText('authLead', message);
    toast(message, 'error');
    return;
  }
  document.getElementById('fido2Pin').value = '';
  if (r.unlocked_compartments && r.unlocked_compartments.length > 0) {
    const labels = r.unlocked_compartments.map(c => c.label).join(', ');
    toast('Unlocked: ' + labels);
  } else {
    toast('Unlocked');
  }
  refresh();
}

async function lock() {
  if (!confirm('Lock all compartments? Master keys will be zeroized from memory.')) return;
  const r = await api('POST', '/api/lock');
  if (r.error) { toast(r.error, 'error'); return; }
  clearSessionToken();
  toast('All compartments locked');
  refresh();
}

async function logoutSession() {
  const r = await api('POST', '/api/session/revoke');
  if (r.error) { toast(r.error, 'error'); return; }
  clearSessionToken();
  toast('Session logged out');
  refresh();
}

// ── Setup Wizard ──────────────────────────────────────────────

function wizShowStep(id) {
  document.querySelectorAll('.wizard-step').forEach(s => s.classList.remove('active'));
  document.getElementById(id).classList.add('active');
  updateWizardChrome(id);
}

function wizPreset(preset) {
  switch (preset) {
    case 'simple':
      wizCompartments = [{ label: 'daily', threshold: 1 }];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'secure':
      wizCompartments = [
        { label: 'daily', threshold: 1 },
        { label: 'secure', threshold: 2 },
      ];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'legacy':
      wizCompartments = [
        { label: 'hot', threshold: 1 },
        { label: 'cold', threshold: 2 },
        { label: 'legacy', threshold: 3 },
      ];
      wizRenderCompList();
      wizShowStep('wizStepCompartments');
      break;
    case 'custom':
      customCompartments = [];
      document.getElementById('wizCustomCompList').innerHTML = '';
      wizShowStep('wizStepCustomComps');
      break;
    case 'passphrase':
      wizShowStep('wizStepPassphrase');
      break;
  }
}

function wizCompRowHtml(comps) {
  let html = '';
  comps.forEach(c => {
    html += '<div class="wiz-comp-row">' +
      '<span class="wiz-comp-label">' + esc(c.label) + '</span>' +
      '<span class="wiz-comp-threshold">Tap ' + c.threshold + ' key' + (c.threshold > 1 ? 's' : '') + '</span></div>';
  });
  return html;
}

function wizRenderCompList() {
  document.getElementById('wizCompList').innerHTML = wizCompRowHtml(wizCompartments);
}

function updateWizardChrome(id) {
  const meta = WIZARD_CHROME[id] || WIZARD_CHROME.wizStep0;
  setText('wizStagePill', meta.pill);
  setText('wizStageTitle', meta.title);
  setText('wizStageSummary', meta.summary);
  setTrustedHtml('wizChecklist', renderChecklist(meta.checklist || []));
}

function wizBackToPresets() {
  wizRequiredKeyCount = 1;
  wizRegisteredKeyCount = 0;
  wizPrimaryKeyLabel = '';
  wizShowStep('wizStep0');
}

async function wizDetectDevice() {
  try {
    const r = await api('GET', '/api/fido2/detect');
    lastFidoDetect = r;
    const hint = document.getElementById('wizDeviceHint');
    if (hint) {
      if (r.device_present) {
        hint.textContent = r.device_count + ' FIDO2 device(s) detected on this machine. You can continue with hardware-key setup.';
      } else {
        hint.textContent = 'No FIDO2 device detected right now. You can insert a hardware key and retry, or choose passphrase-only.';
      }
    }
  } catch(e) {
    const hint = document.getElementById('wizDeviceHint');
    if (hint) {
      hint.textContent = 'Sigillum could not verify hardware-key presence right now. You can still continue, then insert the device before registration if needed.';
    }
  }
}

async function wizInitPassphrase() {
  const label = document.getElementById('wizPLabel').value || 'default';
  const p = document.getElementById('wizPassphrase').value;
  const pc = document.getElementById('wizPassphraseConfirm').value;
  if (p.length < 8) { toast('Min 8 characters', 'error'); return; }
  if (p !== pc) { toast('Passphrases do not match', 'error'); return; }

  const initR = await api('POST', '/api/compartment/init', {
    id: 0,
    label,
    threshold: 1,
    passphrase: p,
  });
  if (initR.error) { toast(initR.error, 'error'); return; }

  document.getElementById('wizDoneMsg').textContent = 'Vault Created';
  document.getElementById('wizDoneDetail').textContent =
    'Compartment "' + label + '" initialized. You are unlocked.';
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

function wizProceedFido2() {
  let comps = wizCompartments;
  if (customCompartments.length > 0) comps = customCompartments;
  if (comps.length === 0) { toast('Add at least one compartment', 'error'); return; }
  wizCompartments = comps;
  wizRequiredKeyCount = Math.max(1, ...wizCompartments.map(c => c.threshold || 1));
  wizRegisteredKeyCount = 0;
  wizShowStep('wizStepFido2Pin');
}

function wizBackFromFido2Pin() {
  if (customCompartments.length > 0) {
    wizShowStep('wizStepCustomComps');
  } else if (wizCompartments.length > 0) {
    wizShowStep('wizStepCompartments');
  } else {
    wizShowStep('wizStep0');
  }
}

function wizAddCustomComp() {
  const label = document.getElementById('wizCustomLabel').value;
  const threshold = parseInt(document.getElementById('wizCustomThreshold').value);
  if (!label || !threshold) { toast('Label and threshold required', 'error'); return; }
  if (customCompartments.some(c => c.threshold === threshold)) {
    toast('Threshold ' + threshold + ' already used', 'error'); return;
  }
  customCompartments.push({ label, threshold });
  clearFields(['wizCustomLabel', 'wizCustomThreshold']);
  document.getElementById('wizCustomCompList').innerHTML = wizCompRowHtml(customCompartments);
  document.getElementById('wizCustomContinue').disabled = false;
}

function wizRenderAdditionalKeyState() {
  const remaining = Math.max(wizRequiredKeyCount - wizRegisteredKeyCount, 0);
  const status = document.getElementById('wizAdditionalKeyStatus');
  if (status) {
    status.textContent = wizRegisteredKeyCount + ' of ' + wizRequiredKeyCount +
      ' required hardware key' + (wizRequiredKeyCount > 1 ? 's' : '') + ' enrolled so far.';
  }

  const lead = document.getElementById('wizAdditionalKeysLead');
  if (lead) {
    if (remaining > 0) {
      lead.textContent = 'Your chosen plan needs ' + wizRequiredKeyCount + ' distinct hardware keys. Register ' +
        remaining + ' more now so every compartment you just created can actually be unlocked later.';
    } else {
      lead.textContent = 'You have enrolled enough hardware keys for the thresholds in this plan. You can finish setup now.';
    }
  }

  const note = document.getElementById('wizAdditionalKeysNote');
  if (note) {
    if (remaining > 0) {
      note.textContent = 'If you finish with fewer keys than the highest threshold, the lower-threshold compartments will work now, but the stronger access layers will stay unavailable until you enroll more keys later.';
    } else {
      note.textContent = 'Every configured compartment now has enough enrolled keys behind it to be usable when the corresponding threshold is met.';
    }
  }
}

function wizCompleteFido2Setup() {
  document.getElementById('wizDoneMsg').textContent = 'Setup Complete';
  if (wizRegisteredKeyCount >= wizRequiredKeyCount) {
    document.getElementById('wizDoneDetail').textContent =
      wizCompartments.length + ' compartment(s) created. ' + wizRegisteredKeyCount +
      ' hardware key(s) are enrolled for this plan, including "' + wizPrimaryKeyLabel + '".';
  } else {
    document.getElementById('wizDoneDetail').textContent =
      wizCompartments.length + ' compartment(s) created. ' + wizRegisteredKeyCount + ' of ' +
      wizRequiredKeyCount + ' planned hardware key(s) are enrolled so far, so only the lower-threshold access layers are ready today.';
  }
  wizShowStep('wizStepDone');
  setTimeout(refresh, 1500);
}

async function wizRegisterKey() {
  const pin = document.getElementById('wizFido2Pin').value;
  const label = document.getElementById('wizFido2Label').value;
  const passphrase = document.getElementById('wizFallbackPass').value || null;
  if (!label) { toast('Label required', 'error'); return; }

  wizShowStep('wizStepTouch');

  const body = {
    label,
    compartments: wizCompartments.map(c => ({
      label: c.label,
      threshold: c.threshold,
      passphrase_mode: null,
    })),
    passphrase: passphrase && passphrase.length >= 8 ? passphrase : null,
  };
  if (pin) body.pin = pin;

  const r = await api('POST', '/api/fido2/setup', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    wizShowStep('wizStepFido2Pin');
    setText('wizDeviceHint', message);
    const input = document.getElementById('wizFido2Pin');
    if (input) input.focus();
    toast(message, 'error');
    return;
  }

  wizRegisteredKeyCount = r.total_keys || 1;
  wizPrimaryKeyLabel = label;
  if (wizRequiredKeyCount > wizRegisteredKeyCount) {
    clearFields(['wizAdditionalKeyPin', 'wizAdditionalKeyLabel', 'wizAdditionalNewPin', 'wizAdditionalNewPinConfirm']);
    wizRenderAdditionalKeyState();
    wizShowStep('wizStepAdditionalKeys');
    toast('Primary key registered. Insert the next trusted key to finish this plan.');
    return;
  }

  wizCompleteFido2Setup();
}

async function wizSetNewPin() {
  await submitNewFido2Pin(
    'wizNewFido2Pin',
    'wizNewFido2PinConfirm',
    'wizDeviceHint',
    'wizFido2Pin',
    'wizFido2Label'
  );
}

async function wizSetAdditionalKeyPin() {
  await submitNewFido2Pin(
    'wizAdditionalNewPin',
    'wizAdditionalNewPinConfirm',
    'wizAdditionalKeyStatus',
    'wizAdditionalKeyPin',
    'wizAdditionalKeyLabel'
  );
}

async function wizRegisterAdditionalKey() {
  const pin = document.getElementById('wizAdditionalKeyPin').value;
  const label = document.getElementById('wizAdditionalKeyLabel').value;
  if (!label) { toast('Label required', 'error'); return; }

  toast('Touch your hardware key now...');
  const body = { label };
  if (pin) body.pin = pin;
  const r = await api('POST', '/api/fido2/register', body);
  if (r.error) {
    const message = friendlyFidoError(r.error);
    setInlineInfo('wizAdditionalKeyStatus', message);
    toast(message, 'error');
    return;
  }

  wizRegisteredKeyCount = r.total_keys || (wizRegisteredKeyCount + 1);
  clearFields(['wizAdditionalKeyPin', 'wizAdditionalKeyLabel', 'wizAdditionalNewPin', 'wizAdditionalNewPinConfirm']);
  wizRenderAdditionalKeyState();

  if (wizRegisteredKeyCount < wizRequiredKeyCount) {
    const remaining = wizRequiredKeyCount - wizRegisteredKeyCount;
    toast('Key "' + label + '" registered. Insert ' + remaining + ' more key' + (remaining > 1 ? 's' : '') + ' to finish this plan.');
    return;
  }

  toast('Key "' + label + '" registered. Your plan now has enough enrolled hardware keys.');
  wizCompleteFido2Setup();
}

function wizFinishForNow() {
  wizCompleteFido2Setup();
}

// ── API Keys & Secrets ────────────────────────────────────────

async function loadApiKeys() {
  try {
    const r = await api('GET', '/api/api-keys');
    if (r.error) return;
    const list = document.getElementById('apiKeyList');
    lastApiKeys = r.keys || [];
    list.innerHTML = '';
    if (lastApiKeys.length === 0) {
      list.innerHTML = '<li><span class="helper-text">No connection keys yet. Store the first RPC or auth token above so provider-backed workflows can run locally.</span></li>';
      return;
    }
    lastApiKeys.forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" data-action="revealApiKeyButton" data-arg0="' + escAttr(k) + '" data-self="append">Reveal</button>' +
        '<button class="btn-danger" data-action="deleteApiKey" data-arg0="' + escAttr(k) + '">Delete</button></div>';
      list.appendChild(li);
    });
  } catch(e) {}
}

async function loadSecrets() {
  try {
    const r = await api('GET', '/api/secrets');
    if (r.error) return;
    const list = document.getElementById('secretList');
    lastSecretKeys = r.keys || [];
    list.innerHTML = '';
    if (lastSecretKeys.length === 0) {
      list.innerHTML = '<li><span class="helper-text">No encrypted secrets yet. Store the first protected value above to confirm the vault flow end to end.</span></li>';
      return;
    }
    lastSecretKeys.forEach(k => {
      const li = document.createElement('li');
      li.innerHTML = '<span>' + esc(k) + '</span>' +
        '<div class="key-actions">' +
        '<button class="btn-ghost" data-action="revealSecretButton" data-arg0="' + escAttr(k) + '" data-self="append">Reveal</button>' +
        '<button class="btn-danger" data-action="deleteSecret" data-arg0="' + escAttr(k) + '">Delete</button></div>';
      list.appendChild(li);
    });
  } catch(e) {}
}

async function setApiKey() {
  const key = document.getElementById('apiKeyName').value;
  const value = document.getElementById('apiKeyValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/api-keys/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['apiKeyName', 'apiKeyValue']);
  toast('API key stored');
  refresh();
}

async function setSecret() {
  const key = document.getElementById('secretName').value;
  const value = document.getElementById('secretValue').value;
  if (!key || !value) { toast('Key and value required', 'error'); return; }
  const r = await api('POST', '/api/secrets/set', { key, value });
  if (r.error) { toast(r.error, 'error'); return; }
  clearFields(['secretName', 'secretValue']);
  toast('Secret stored');
  refresh();
}

function showRevealedValue(li, btn, value) {
  let existing = li.querySelector('.secret-value');
  if (existing) { existing.remove(); btn.textContent = 'Reveal'; return; }
  const div = document.createElement('div');
  div.className = 'secret-value';
  div.textContent = value;
  li.appendChild(div);
  btn.textContent = 'Hide';
  setTimeout(() => {
    const el = li.querySelector('.secret-value');
    if (el) { el.remove(); btn.textContent = 'Reveal'; }
  }, 30000);
}

async function revealApiKey(key, btn) {
  const r = await api('POST', '/api/api-keys/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  showRevealedValue(btn.closest('li'), btn, r.value);
}

async function revealSecret(key, btn) {
  const r = await api('POST', '/api/secrets/get', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  showRevealedValue(btn.closest('li'), btn, r.value);
}

function revealApiKeyButton(key, btn) {
  return revealApiKey(key, btn);
}

function revealSecretButton(key, btn) {
  return revealSecret(key, btn);
}

async function deleteApiKey(key) {
  if (!confirm('Delete API key "' + key + '"?')) return;
  const r = await api('POST', '/api/api-keys/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deleted');
  refresh();
}

async function deleteSecret(key) {
  if (!confirm('Delete secret "' + key + '"?')) return;
  const r = await api('POST', '/api/secrets/delete', { key });
  if (r.error) { toast(r.error, 'error'); return; }
  toast('Deleted');
  refresh();
}

function bytesToHex(bytes) {
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
}

function hexToBytes(hex) {
  if (hex.length % 2 !== 0) throw new Error('Invalid hex length');
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    out[i / 2] = parseInt(hex.slice(i, i + 2), 16);
  }
  return out;
}

async function exportSnapshot() {
  const passphrase = document.getElementById('backupExportPass').value;
  if (!passphrase || passphrase.length < 8) {
    toast('Export passphrase must be at least 8 characters', 'error');
    return;
  }

  const r = await api('POST', '/api/backup/export', { passphrase });
  if (r.error) { toast(r.error, 'error'); return; }

  try {
    const bytes = hexToBytes(r.snapshot_hex);
    const blob = new Blob([bytes], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'sigillum-snapshot-' + (r.summary?.created_at_unix || Date.now()) + '.json';
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    document.getElementById('backupExportPass').value = '';
    toast('Snapshot downloaded');
  } catch (e) {
    toast('Failed to prepare snapshot download', 'error');
  }
}

async function restoreSnapshot(
  fileInputId = 'backupRestoreFile',
  passphraseId = 'backupRestorePass',
  successMessage = 'Snapshot restored. Unlock the vault again.'
) {
  const fileInput = document.getElementById(fileInputId);
  const passphraseInput = document.getElementById(passphraseId);
  if (!fileInput || !passphraseInput) {
    toast('Snapshot restore controls are unavailable.', 'error');
    return;
  }
  const passphrase = passphraseInput.value;
  const file = fileInput.files && fileInput.files[0];
  if (!file) { toast('Choose a snapshot file', 'error'); return; }
  if (!passphrase || passphrase.length < 8) {
    toast('Restore passphrase must be at least 8 characters', 'error');
    return;
  }
  if (!confirm('Restore this snapshot? Current on-disk Sigillum data will be replaced and you will need to unlock again.')) return;

  let snapshotHex;
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    snapshotHex = bytesToHex(bytes);
  } catch (e) {
    toast('Failed to read snapshot file', 'error');
    return;
  }

  const r = await api('POST', '/api/backup/restore', {
    passphrase,
    snapshot_hex: snapshotHex,
  });
  if (r.error) { toast(r.error, 'error'); return; }

  clearSessionToken();
  passphraseInput.value = '';
  fileInput.value = '';
  toast(successMessage);
  refresh();
}

function restoreSetupSnapshot() {
  return restoreSnapshot(
    'setupRestoreFile',
    'setupRestorePass',
    'Snapshot restored. Continue setup or unlock the restored vault.'
  );
}

function restoreAuthSnapshot() {
  return restoreSnapshot(
    'authRestoreFile',
    'authRestorePass',
    'Snapshot restored. Unlock the restored vault to continue.'
  );
}

async function resetLocalData(confirmId = 'setupResetConfirm') {
  const confirmInput = document.getElementById(confirmId);
  if (!confirmInput) {
    toast('Reset controls are unavailable.', 'error');
    return;
  }
  const confirmation = confirmInput.value.trim();
  if (confirmation !== SETUP_RESET_CONFIRMATION) {
    toast("Type '" + SETUP_RESET_CONFIRMATION + "' exactly to continue.", 'error');
    return;
  }
  if (!confirm('Erase all local Sigillum data on this machine and return to first-run setup?')) {
    return;
  }

  const r = await api('POST', '/api/setup/reset', { confirmation });
  if (r.error) { toast(r.error, 'error'); return; }

  clearSessionToken();
  clearFields([
    'setupResetConfirm',
    'authResetConfirm',
    'backupResetConfirm',
    'setupRestorePass',
    'authRestorePass',
    'backupRestorePass',
  ]);
  ['setupRestoreFile', 'authRestoreFile', 'backupRestoreFile'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
  toast('Local Sigillum data cleared. You can start setup again or restore a snapshot.');
  refresh();
}

function formatAuditEvent(event) {
  const details = event.details || {};
  const labels = {
    'unlock.passphrase': 'Unlocked with passphrase',
    'unlock.fido2': 'Unlocked with FIDO2',
    'lock.all': 'Locked all compartments',
    'session.revoke': 'Revoked session',
    'compartment.add': 'Added compartment',
    'compartment.init': 'Initialized compartment',
    'compartment.remove': 'Removed compartment',
    'compartment.switch': 'Switched compartment',
    'api_key.set': 'Stored API key',
    'api_key.delete': 'Deleted API key',
    'secret.set': 'Stored encrypted secret',
    'secret.delete': 'Deleted encrypted secret',
    'secret.push': 'Pushed secret between compartments',
    'profiles.eth_xpub_wallet.upsert': 'Saved xpub wallet profile',
    'profiles.eth_xpub_wallet.delete': 'Deleted xpub wallet profile',
    'wallet.eth_xpub.export': 'Exported xpub receive branch',
    'fido2.setup': 'Completed FIDO2 setup',
    'fido2.register': 'Registered FIDO2 key',
    'fido2.register_poison': 'Registered poison FIDO2 key',
    'fido2.remove': 'Removed FIDO2 key',
    'snapshot.export': 'Exported encrypted snapshot',
    'snapshot.restore': 'Restored encrypted snapshot',
  };
  let suffix = '';
  if (details.label) suffix = ' - ' + details.label;
  else if (details.key) suffix = ' - ' + details.key;
  else if (details.name) suffix = ' - ' + details.name;
  else if (details.wallet_profile) suffix = ' - ' + details.wallet_profile;
  else if (details.compartment_count) suffix = ' - ' + details.compartment_count + ' compartments';
  else if (details.count) suffix = ' - ' + details.count + ' compartments';
  else if (details.file_count) suffix = ' - ' + details.file_count + ' files';
  return (labels[event.kind] || event.kind) + suffix;
}

async function loadAudit() {
  try {
    const r = await api('GET', '/api/audit?limit=20');
    if (r.error) return;
    const list = document.getElementById('auditList');
    const events = r.events || [];
    if (events.length === 0) {
      list.innerHTML = '<p class="text-meta">No audit events yet.</p>';
      return;
    }
    let html = '<ul class="key-list">';
    events.forEach(event => {
      const when = new Date((event.created_at_unix || 0) * 1000).toLocaleString();
      const comp = event.compartment_id != null
        ? '<span style="color:var(--text-dim);font-size:11px;">compartment #' + event.compartment_id + '</span>'
        : '<span style="color:var(--text-dim);font-size:11px;">global</span>';
      html += '<li><span>' + esc(formatAuditEvent(event)) +
        '<div style="color:var(--text-dim);font-size:11px;margin-top:4px;">' +
        esc(when) + ' · ' + comp + '</div></span></li>';
    });
    html += '</ul>';
    list.innerHTML = html;
  } catch (e) {}
}

async function loadDiagnostics() {
  try {
    const r = await api('GET', '/api/diagnostics');
    const el = document.getElementById('diagGrid');
    if (r.error) {
      el.innerHTML = '<div style="color:var(--danger);font-size:13px;">' + esc(r.error) + '</div>';
      return;
    }
    const started = r.started_at_unix ? new Date(r.started_at_unix * 1000).toLocaleString() : '-';
    el.innerHTML = [
      statBox(r.version || '-', 'Version'),
      statBox(r.unlock_scope || '-', 'Unlock Scope'),
      statBox(r.session_scope || '-', 'Session Scope'),
      statBox(String(r.active_session_count ?? 0), 'Sessions'),
      statBox(String(r.unlocked_compartment_count ?? 0), 'Unlocked'),
      statBox(r.max_unlocked_threshold != null ? String(r.max_unlocked_threshold) : '-', 'Max Threshold'),
      statBox(r.default_active_compartment_id != null ? String(r.default_active_compartment_id) : '-', 'Default Active'),
      statBox(r.audit_log_present ? 'yes' : 'no', 'Audit Log'),
      statBox(String(r.pending_operation_count ?? 0), 'Pending Ops'),
      statBox(String(r.startup_interrupted_operation_count ?? 0), 'Interrupted Ops'),
      statBox(String(r.startup_recovered_operation_count ?? 0), 'Recovered Ops'),
      statBox(String(r.startup_unresolved_operation_count ?? 0), 'Unresolved Ops'),
      statBox(String(r.queue_job_count ?? 0), 'Queue Jobs'),
      statBox(String(r.blocked_queue_job_count ?? 0), 'Blocked Jobs'),
      statBox(String(r.retrying_queue_job_count ?? 0), 'Retrying Jobs'),
      statBox(String(r.failed_queue_job_count ?? 0), 'Failed Jobs'),
      statBox(String(r.startup_recovered_queue_job_count ?? 0), 'Recovered Jobs'),
      statBox(String(r.startup_reconciled_deposit_count ?? 0), 'Recovered Deposits'),
      statBox(
        (r.runtime_policy?.queue_default_process_limit ?? '-') + '/' + (r.runtime_policy?.queue_max_process_limit ?? '-'),
        'Queue Limit'
      ),
      statBox(
        (r.runtime_policy?.deposit_default_refresh_limit ?? '-') + '/' + (r.runtime_policy?.deposit_max_refresh_limit ?? '-'),
        'Refresh Limit'
      ),
      statBox(
        (r.runtime_policy?.queue_retry_base_delay_secs ?? '-') + '/' + (r.runtime_policy?.queue_retry_max_delay_secs ?? '-'),
        'Retry Backoff'
      ),
      statBox(String(r.runtime_policy?.provider_balance_observation_concurrency ?? '-'), 'RPC Concurrency'),
      statBox(
        (r.runtime_policy?.audit_default_limit ?? '-') + '/' + (r.runtime_policy?.audit_max_limit ?? '-'),
        'Audit Limit'
      ),
      statBox(String(r.eth_stealth_deposit_count ?? 0), 'Deposits'),
      statBox(r.initialized ? 'yes' : 'no', 'Initialized'),
      statBox(started, 'Started'),
    ].join('');
  } catch (e) {}
}

function statBox(value, label) {
  return '<div class="stat"><div class="value" style="font-size:16px;">' + esc(String(value)) +
    '</div><div class="label">' + esc(label) + '</div></div>';
}

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}
function escAttr(s) {
  return esc(s).replace(/'/g, '&#39;');
}

function coerceActionArg(value, type) {
  if (type === 'number') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : value;
  }
  return value;
}

function collectActionArgs(actionEl) {
  const args = [];
  for (let index = 0; ; index += 1) {
    const key = 'arg' + index;
    if (!(key in actionEl.dataset)) break;
    args.push(coerceActionArg(
      actionEl.dataset[key],
      actionEl.dataset[key + 'Type']
    ));
  }
  if (actionEl.dataset.self === 'append') args.push(actionEl);
  return args;
}

function dispatchDataAction(actionEl) {
  const actionName = actionEl.dataset.action;
  const action = globalThis[actionName];
  if (typeof action !== 'function') {
    console.warn('Unknown UI action:', actionName);
    return;
  }
  Promise.resolve(action(...collectActionArgs(actionEl))).catch(error => {
    console.error('UI action failed:', actionName, error);
    toast('Action failed: ' + actionName, 'error');
  });
}

function handleActionEvent(event) {
  const actionEl = event.target instanceof Element
    ? event.target.closest('[data-action]')
    : null;
  if (!actionEl) return;
  if (actionEl.tagName === 'BUTTON') event.preventDefault();
  dispatchDataAction(actionEl);
}

document.addEventListener('keydown', e => {
  if (e.key !== 'Enter') return;
  if (e.target.id === 'passphrase') unlock();
  if (e.target.id === 'fido2Pin') fido2Unlock();
  if (e.target.id === 'wizPassphraseConfirm') wizInitPassphrase();
  if (e.target.id === 'wizFido2Label') wizRegisterKey();
});

document.addEventListener('visibilitychange', () => {
  if (shouldAutoRefresh()) {
    void refresh();
  } else {
    clearRefreshTimer();
    updateRefreshMeta();
  }
});

document.addEventListener('click', handleActionEvent);
window.addEventListener('beforeunload', clearRefreshTimer);

void refresh();
"##;

const INDEX_HTML_SHELL_AFTER_SCRIPT: &str = r##"
</body>
</html>
"##;
