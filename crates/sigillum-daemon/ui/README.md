# Sigillum Daemon UI

This directory is the typed source home for Sigillum's embedded operator
console.

The daemon embeds `src/*.html`, checked-in `src/styles.css`, and checked-in
`src/app.js` with `include_str!`, keeping the shipped binary self-contained and
allowing per-request CSP nonce injection. Authored runtime source is
`src/app.ts`; authored styles live under `src/styles/*`. Vite regenerates the
embedded bundles.

## Source layout

- `src/app.ts` — boot/composition shell and residual legacy integration.
- `src/core/` — store, router/adapter, SSE reconciliation, shared typed API,
  DOM/keyed-list helpers, keyboard behavior, safe command palette, and live
  runtime composition.
- `src/destinations/` — active Overview, Move, Receiving, Portfolio, and Vault
  controllers.
- `src/render/` — formatting, confirmation/modal, secret-prompt, and shared DOM
  presentation helpers.
- `src/views/` — setup/session plumbing and residual legacy views not yet
  replaced by a destination controller.
- `src/styles/` — authored tokens, shell, components, destination, responsive,
  and accessibility/focus styles.

Controller ownership, takeover/restoration, async-generation, mutation,
focus, and accessibility rules are binding in [DESIGN.md](./DESIGN.md).
The unlocked command palette exposes exactly five destination commands plus
refresh and self-check; it contains no destructive action. The events client
retires an authenticated SSE stream when the browser session token is revoked
or rotated, then reconnects only with the current authorization. Session
requests cannot let a stale `401` clear a newer token, and same-tab token clear
applies the locked shell and palette policy synchronously.

## Local verification

```sh
npm ci --ignore-scripts
npm audit --audit-level=high
npm run typecheck
npm test
npm run build

cd ../../..
node scripts/ui-screenshots/server.test.mjs
node scripts/ui-screenshots/drive.mjs
./scripts/check-ui-accessibility.sh
./scripts/check-browser-smoke.sh
```

Run `npm run build` after editing `src/app.ts`, any imported TypeScript module,
or `src/styles/*`. Commit the regenerated `src/app.js` and `src/styles.css`
with the source change; never edit either generated bundle by hand.

The fake-DOM tests, strict mock screenshot/accessibility harness, and
real-daemon browser smoke are separate proof layers. Mock results prove the
checked-in frontend renders representative envelopes; they do not prove daemon
authentication, provider RPC, persistence, signing, broadcast, desktop
packaging, or release readiness.
