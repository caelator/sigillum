//! Embedded single-page web UI for Sigillum vault management.
//!
//! The interface source lives under `crates/sigillum-daemon/ui/src`. The Rust
//! host embeds checked-in HTML/CSS plus the Vite-generated `src/app.js` runtime
//! with `include_str!`, while the authored runtime remains TypeScript.
//!
//! ## Architecture
//!
//! The shipped daemon remains a single static binary. The frontend source has a
//! TypeScript/Vite workspace for typed modules; Vite writes the bundled runtime
//! back to the checked-in `src/app.js` asset that Rust embeds.
//!
//! ### CSS design system
//!
//! All colours, radii, and fonts are declared as CSS custom properties on
//! `:root`. Every visual component (cards, badges, pills, buttons, entity
//! lists) references these tokens, ensuring a single place to change the
//! palette or spacing scale.
//!
//! ### TypeScript architecture
//!
//! - **`api/session.ts`** — session token storage plus the daemon fetch wrapper
//!   that attaches the bearer token and auto-clears it on 401.
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
        "{INDEX_HTML_BEFORE_STYLE}<style>{INDEX_HTML_STYLE}</style>{INDEX_HTML_AFTER_STYLE}<script nonce=\"{nonce}\">{INDEX_HTML_SCRIPT}</script>{INDEX_HTML_AFTER_SCRIPT}",
    )
}

const INDEX_HTML_BEFORE_STYLE: &str = include_str!("../ui/src/index.before-style.html");
const INDEX_HTML_STYLE: &str = include_str!("../ui/src/styles.css");
const INDEX_HTML_AFTER_STYLE: &str = include_str!("../ui/src/index.after-style-before-script.html");
const INDEX_HTML_SCRIPT: &str = include_str!("../ui/src/app.js");
const INDEX_HTML_AFTER_SCRIPT: &str = include_str!("../ui/src/index.after-script.html");
