# Sigillum Daemon UI

This directory is the typed source home for the embedded operator UI.

The daemon currently embeds `src/*.html`, `src/styles.css`, and `src/app.js`
directly with `include_str!` so the shipped binary stays self-contained and
keeps its per-request CSP nonce injection. The Vite/TypeScript project exists
to type-check new modules and gradually migrate the legacy runtime script
without forcing a bundled asset pipeline into the Rust build yet.

Useful commands:

```sh
npm install
npm run typecheck
npm run build
```
