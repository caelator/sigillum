# Sigillum Console Design System v2

Rules and vocabulary for the destination rebuilds (plan tasks 4.1–4.3).
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
polling fallback; `api` is the single typed client (failures are a
discriminated union on daemon error codes — branch on them, e.g.
`vault_locked` → unlock); `router` owns `#/destination[/sub-path]` and the
legacy-section adapter is the migration seam (see the contract in
`core/router.ts` — it is binding for destination agents); `dom` is `el()` +
`renderList()`.
