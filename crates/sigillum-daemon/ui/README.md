# Sigillum Daemon UI

This directory is the typed source home for the embedded operator UI.

The daemon embeds `src/*.html`, `src/styles.css`, and the checked-in
`src/app.js` runtime with `include_str!` so the shipped binary stays
self-contained and keeps its per-request CSP nonce injection. The authored
runtime source is `src/app.ts`; Vite bundles that TypeScript entry and its
typed modules back into `src/app.js` for Rust to embed.

Useful commands:

```sh
npm install
npm run typecheck
npm run build
```

Run `npm run build` after editing `src/app.ts` or any imported UI module so the
embedded `src/app.js` stays in sync.
