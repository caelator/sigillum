# Sigillum Daemon UI

This directory is the typed source home for the embedded operator UI.

The daemon embeds `src/*.html`, the checked-in `src/styles.css`, and the
checked-in `src/app.js` runtime with `include_str!` so the shipped binary stays
self-contained and keeps its per-request CSP nonce injection. The authored
runtime source is `src/app.ts`, and the authored styles live under
`src/styles/*`; Vite bundles those sources back into `src/app.js` and
`src/styles.css` for Rust to embed.

`src/app.ts` is now the shell/boot surface. Domain rendering and actions live
under `src/views/*`, dispatching lives under `src/actions/dispatcher.ts`, and
shared DOM/form/HTML helpers live under `src/render/*`.

Useful commands:

```sh
npm install
npm run typecheck
npm test
npm run build
```

Run `npm run build` after editing `src/app.ts` or any imported UI module so the
embedded `src/app.js` stays in sync. Run the same build after editing
`src/styles/*`; `src/styles.css` is generated and should not be edited by hand.
