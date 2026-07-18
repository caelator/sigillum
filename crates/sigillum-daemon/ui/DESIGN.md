# Sigillum Console Design System v2

Rules and vocabulary for the destination rebuilds and interaction layer (plan
tasks 4.1–4.4).
The legacy partials keep working untouched; **new code uses the core
(`src/core/`) and this system**. When anything here is ambiguous, the
governing principle is: visual weight and friction proportional to money
at stake.

## Hard rules for new code

1. **No inline styles in new code.** No `style="…"` attributes, no
   `el.style.foo = …`, no per-screen hex values. If a look isn't covered by
   tokens/components, add a token or a component class — never a one-off.
   (Legacy views are exempt until they migrate; do not extend the pattern.)
2. **No `key=value` meta lines.** State reads as plain sentences, pills, or
   table cells — never `status=funded_needs_gas chain=1`. Enum values from
   the API are translated (`render/format.ts` helpers, `pillClass`), never
   printed raw.
3. **Human units only.** ETH/gwei (never wei hex), chain names from the
   chain registry (never "chain N"), locale or relative time (never unix
   seconds) in every default rendering. Raw values may live behind a
   "details" disclosure.
4. **Data flows through `textContent`, never `innerHTML`.** Build DOM with
   `core/dom.ts` `el()`/`renderList()`. The `html` prop is reserved for
   markup whose interpolations are already escaped with `esc()`/`escAttr()`.
5. **Lists render by key.** Always `renderList(container, items, keyFn,
   renderItem)`; patch the `existing` node and return it. Wholesale
   `innerHTML` list re-renders are a bug (they wipe focus and in-progress
   input).

## Consequence tiers

Three grades, applied via one attribute — `[data-tier="quiet|review|danger"]`
— which re-maps the component's private custom properties (`--_tier-*`) to
the tier tokens (`00-design-tokens.css` §1b). Components never hard-code
tier colors.

| Tier | Meaning | Friction (confirm-dialog tier) | Accent |
|------|---------|--------------------------------|--------|
| `quiet` | Browsing, reading, reversible navigation | none / inform | info blue |
| `review` | Deliberate attention before proceeding (queue a job, run a scan, rotate an address) | confirm dialog | warning amber |
| `danger` | Value-moving or irreversible (broadcasts, deletions, policy changes, snapshot restore) | confirm with consequence copy, typed phrase for the worst | danger red |

A screen's default state is `quiet`. Escalate only the element that carries
the consequence (a row, a button, a banner) — never the whole page.

## Tokens (00-design-tokens.css §1b)

- **Tiers**: `--tier-{quiet,review,danger}-{surface,border,text,accent,accent-text,tint}`.
- **Numerals**: put `.nums` on amounts, stat values, balances, and figure
  columns (`font-variant-numeric: tabular-nums` — aligned digits, no jitter
  on live updates). `.stat .value` carries it by default.
- **Density**: `--density-pad-{x,y}`, `--density-row-h[-compact]` — generous
  forms at rest, compact rows for scan-heavy lists (`.table.compact`).
- **Motion**: `--t-fast` state flips, `--t-med` reveals, `--t-slow`
  overlays/sheets only; `--ease-standard` / `--ease-entrance`. Motion must
  never carry meaning alone: every animated state has a static rendering
  under `prefers-reduced-motion` (the global guard zeroes durations; see
  `.skeleton` and `.status-dot` for the pattern).

## Components (14-components-v2.css)

- `.page-header` — one per destination: the question the screen answers +
  plain-language summary + optional `.page-header-actions`.
- `.section-empty` — empty state: why it's empty + one next action. Never a
  bare "No items" line.
- `.skeleton` (+ `.skeleton-text`, `.skeleton-block`) — loading placeholders
  with a reduced-motion static fallback.
- `.table` (+ `.compact`) — the console's data-table pattern; `.nums` on
  figure columns, `[data-tier]` on `<tr>` to grade a row.
- `.status-dot` — tiny live-ness dot; `[data-state]` for transport/health,
  `[data-tier]` for consequence.
- `.attention-item` — one ranked "needs you" row: tier stripe, title,
  why-it-matters body, exactly one `.attention-item-action`.

## The core (`src/core/`) in one paragraph

`store` holds per-resource slices (reference equality, microtask-batched);
`events` feeds `status`/`operations`/`queueEvents` over SSE with a passive
polling fallback and retires the authenticated stream generation whenever the
session token is revoked or rotated; `api` is the preferred shared typed client
(some destination endpoints still use thin session-aware wrappers with the
same `ApiFailure` contract). Every session request binds its `401` handling to
the token it sent, and same-tab token clear applies the locked shell immediately;
`palette` exposes only five navigation commands, refresh, and self-check behind
unlocked/modal-safe policy; `router` owns
`#/destination[/sub-path]` and the
legacy-section adapter is the migration seam (see the contract in
`core/router.ts` — it is binding for destination agents); `dom` is `el()` +
`renderList()`.

## Binding destination-controller contract

1. A controller declares the exact host and legacy siblings it owns. Mount
   stashes them and prevents legacy writers from mutating taken-over DOM;
   unmount unsubscribes listeners and restores the stashed nodes.
2. Store subscriptions and browser listeners are paired with cleanup. Async
   loads carry a generation or slice-revision guard so a stale completion can
   never replace newer state after navigation, refresh, or mutation.
3. Preserve the last good rendering when a refresh fails and show an honest
   stale/error banner. Do not clear useful data into a success-shaped empty
   state.
4. Use `core/api.ts` when it exposes the endpoint. A temporary local wrapper
   must remain thin, session-aware, typed, and conform to `ApiFailure`; it must
   not invent a second response envelope.
5. Keyed lists preserve stable nodes and operator input. Patch `existing` when
   practical. A renderer may return a fresh node only when replacement is
   intentional; `renderList` removes the old node in that case.
6. Use the shared modal coordinator exclusively. Only one modal may be active;
   it traps focus, closes on Escape/backdrop where allowed, and restores focus
   only to a connected element. Existing and dynamically appended background
   siblings stay inert, and escaped programmatic focus returns to the dialog.
   Cancellation is distinct from explicit blank input and must cause zero
   mutation requests.
7. Signing, broadcast, delete, reset, restore, key removal, and policy changes
   are never optimistic. A safe optimistic mutation must retain a rollback
   snapshot, restore it on write failure, and distinguish a failed write from a
   successful write followed by failed refresh.
8. Focus has an owner. Do not autofocus on every render. Locked/setup focus may
   run once on a state transition but yields to an active modal and to operator
   focus. Enter activates only the intended visible form action; Escape never
   triggers a mutation. Delayed locked-mode FIDO detection must recheck the
   current mode before mutating unlock guidance.
9. Default views use semantic landmarks, heading order, labels, lists, and
   tables. Native file inputs keep a programmatic label. Do not encode meaning
   only in color, motion, placeholder text, or visual placement.
10. The command palette is an allowlist, never a reflection of the broad action
    map. Keep it navigation/read-refresh only. It must refuse locked state and
    active modals, recheck lock state before dispatch, and close before an async
    command or error handler runs.

## Required verification

For every controller or shared-interaction change:

```sh
npm test
npm run typecheck
npm run build
node ../../../scripts/ui-screenshots/server.test.mjs
../../../scripts/check-ui-accessibility.sh
../../../scripts/check-browser-smoke.sh
```

The unit/fake-DOM, strict-mock screenshot/axe, and real-daemon browser layers
prove different things. Mock rendering is not evidence of daemon auth, RPC,
signing, broadcast, persistence, or packaging. The release claim comes only
from the complete clean-tree release gate.
