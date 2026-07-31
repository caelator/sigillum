# UI screenshot harness

Zero-dependency harness that renders the operator console exactly as the
daemon serves it and captures a canonical set of screenshots in headless
Chrome. Use it to review UI changes without a live vault, and to produce the
operator-reviewed screenshots the release plan calls for
(`docs/release-1.0-plan.md`: "screenshots of setup, locked, and unlocked
states reviewed by the operator").

## How it works

- `server.mjs` — a strict stateful mock daemon. It assembles the page exactly like
  `crates/sigillum-daemon/src/ui.rs` `render_index_html` (HTML fragments from
  `crates/sigillum-daemon/ui/src` plus the checked-in vite bundles
  `src/app.js` / `src/styles.css`; the per-request CSP nonce is dropped —
  irrelevant for local shots) and answers every `/api/*` route the UI calls
  with populated state from `mock-data.mjs`. Routes are explicitly allowlisted,
  mutations update mock state, responses use the daemon's real envelopes, and
  unknown routes are recorded and fail the run rather than returning a
  permissive `{}`.
- `drive.mjs` — starts the mock daemon in-process, drives headless Chrome
  over the raw DevTools protocol (same approach as
  `scripts/browser-smoke.mjs`), walks the shot list, and writes PNGs. Browser
  exceptions, contract-breaking console errors, and unknown mock requests fail
  the run.
- `mock-data.mjs` — the populated vault state: two compartments, providers,
  seed/xpub/stealth wallets, inventory with balances, a consolidation plan
  with six steps, queue jobs in mixed states, stealth deposits, parties and
  receive allocations, an enabled treasury policy, a self-check run with one
  failing domain, FIDO2 keys, audit events, and diagnostics.

The screenshot harness itself needs no npm install, build step, or daemon
binary.

Before relying on a mock change, run its contract tests:

```sh
node scripts/ui-screenshots/server.test.mjs
```

The same stateful mock also backs the automated accessibility release gate.
After installing the UI dependencies, run:

```sh
./scripts/check-ui-accessibility.sh
```

The gate injects the exactly pinned `axe-core` build into the shipped UI in
headless Chrome. Its 15 scenarios audit setup welcome and protection-model
states, the locked unlock screen, all five unlocked destinations, routed
Portfolio and Move subviews, and the open command palette. Any axe violation
or incomplete check, stale bundle, missing
scenario, browser exception, or unknown mock route fails the command. Finding
output includes the rule, impact, affected selectors and nodes, and the axe
help URL.

## Prerequisites

- Built UI bundles: `npm run build` in `crates/sigillum-daemon/ui`. The
  harness never builds anything itself — it refuses to run when `app.js` /
  `styles.css` are missing or older than the authored source, and tells you
  to rebuild. (Type-only sources such as `contracts.ts` are ignored by the
  staleness check; editing them cannot make the bundles stale.)
- Chrome or Chromium (`CHROME_BIN` / `GOOGLE_CHROME_BIN` to point at a
  specific executable).
- Node 18+ (uses the global `WebSocket` and `fetch`).

## Usage

```sh
node scripts/ui-screenshots/drive.mjs
```

Options (argv wins over env):

| Flag | Env | Default |
| --- | --- | --- |
| `--out=<dir>` | `SIGILLUM_UI_SHOTS_DIR` | `target/ui-screenshots/` (gitignored) |
| `--width=<px>` | `SIGILLUM_UI_SHOTS_WIDTH` | `1440` |
| `--height=<px>` | `SIGILLUM_UI_SHOTS_HEIGHT` | `900` |
| `--scale=<n>` | `SIGILLUM_UI_SHOTS_SCALE` | `2` (retina) |

Exit code is non-zero on any driver failure; page console errors are printed
at the end. To browse the mock UI interactively instead of shooting it:

```sh
node scripts/ui-screenshots/server.mjs 8080   # then open http://127.0.0.1:8080
# switch daemon mode while browsing:
curl -X POST -d '{"mode":"setup"|"locked"|"unlocked"}' http://127.0.0.1:8080/__mode
```

## Output

One PNG per shot in the output directory: `setup-welcome`,
`setup-protection-model`, `unlock`, one `section-<destination>` per workspace
destination (overview, receive, portfolio, move, vault), plus card-level
shots (`section-receive-deposits`, `section-move-plans`,
`section-move-queue`, `section-vault-diagnostics`) for populated surfaces
that a top-of-section viewport shot would leave below the fold.

## Adding or changing shots

The shot list is the `SHOTS` array at the top of `drive.mjs` — that array is
the harness's output contract. Each entry:

```js
{ name: "section-move-queue",   // writes <name>.png
  mode: "unlocked",             // daemon mode: setup | locked | unlocked
  section: "move",              // nav destination to open first (optional)
  click: "[data-action=...]",   // selector to click before shooting (optional)
  waitFor: "<js expression>",   // extra precondition (optional)
  scrollTo: "queueCard",        // element id scrolled into view (optional)
  fullPage: true }              // whole scroll height instead of viewport (optional)
```

Entries run in order; the driver reuses one page per daemon mode, so group
shots by mode. If a shot needs mock state the page does not already show,
extend `mock-data.mjs` — its shapes mirror
`crates/sigillum-api/src/response.rs` and `ui/src/contracts.ts`, and stale
shapes show up as empty cards in the shots.

## Release evidence

Run the harness on the release commit and attach the PNG set to the release
notes / PR as a mock-data walkthrough of setup, locked, and unlocked surfaces.
Because the page is assembled from the checked-in bundles and fragments, the
shots capture what the shipped daemon embeds; the bundle-staleness guard keeps
them aligned with authored UI source. For UI-affecting PRs, run before and
after the change (`--out=target/ui-screenshots/before|after`) and diff the sets.

This proof boundary is strict: screenshots and axe results prove the shipped
HTML/CSS/JavaScript renders representative, contract-shaped mock data. They do
not prove daemon authentication, session expiry, provider RPC, signing,
broadcast, persistence/restart, desktop installation, or live execution. The
real-daemon `scripts/check-browser-smoke.sh`, the complete clean-tree release
gate, and operator review remain separate requirements. Only the final
operator-reviewed set should be copied to the external release-evidence bundle;
generated PNGs stay untracked.
