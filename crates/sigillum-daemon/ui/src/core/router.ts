/**
 * core/router.ts — hash routing and the legacy-section adapter (plan task 4.1).
 *
 * URL shape: `#/<destination>[/<sub-path…>]`, e.g. `#/move` or
 * `#/move/plan/abc-123`. Destinations are the five console IA sections:
 * `overview | receive | portfolio | move | vault`.
 *
 * ────────────────────────────────────────────────────────────────────────────
 * THE MIGRATION SEAM — adapter contract (destination agents depend on this)
 * ────────────────────────────────────────────────────────────────────────────
 *
 * The legacy console has no URL routing: it tracks one active "workspace
 * section" in sessionStorage and toggles `.card[data-workspace-section]`
 * visibility. The router and the legacy switcher therefore BOTH drive the
 * same UI. {@link createLegacySectionAdapter} keeps them in sync under a
 * precise ownership split:
 *
 *  1. **Segment ownership.** The FIRST path segment (the destination) is
 *     shared: the legacy switcher and the router may both set it. Everything
 *     AFTER the first segment is owned by the destination's migrated
 *     controller — the legacy adapter never rewrites or clears it, and
 *     legacy code never reads it.
 *
 *  2. **Route → legacy.** On every effective route change the adapter calls
 *     `bridge.selectSection(destination)` when the legacy section differs,
 *     so the legacy chrome (sidebar nav highlight, topbar title, card
 *     visibility) always tracks the URL — for migrated AND unmigrated
 *     destinations alike.
 *
 *  3. **Legacy → route.** Legacy code reports section changes through
 *     `adapter.notifyLegacySection(sectionId)` (wired inside the legacy
 *     section-store helper — one call site). The adapter navigates to
 *     `#/<sectionId>` ONLY when the route's destination differs; when it
 *     matches, the existing sub-path (owned by a migrated controller, per
 *     rule 1) is preserved untouched.
 *
 *  4. **Migrated destinations.** A destination registers a controller with
 *     `migrated: true`, `mount(route)`, and `unmount()`. When the route
 *     enters the destination the adapter calls `mount(route)` (after the
 *     legacy sync of rule 2); when it leaves, `unmount()`. The controller
 *     owns everything below the destination segment and renders into its
 *     own host element. Unmigrated destinations need NO controller — the
 *     adapter keeps the legacy section flow working exactly as before.
 *
 *  5. **Boot.** On `start()`, a valid hash wins (deep link): the adapter
 *     publishes it and rule 2 pulls the legacy console to that section. An
 *     empty/invalid hash is REPLACED (no history entry) with the legacy
 *     section's hash, so sessionStorage restore keeps working unchanged.
 *
 *  6. **Loop safety.** Legacy → route writes happen only on destination
 *     mismatch; route → legacy selection happens only on section mismatch.
 *     A change originating on one side therefore never echoes back.
 *
 * Deep links into UNMIGRATED destinations (`#/move/plan/abc` before Move is
 * rebuilt) are inert but preserved: the adapter selects the legacy section
 * and the full route stays in the store for the future controller.
 */

import type { Unsubscribe } from "./store";

export const DESTINATIONS = [
  "overview",
  "receive",
  "portfolio",
  "move",
  "vault",
] as const;

export type Destination = (typeof DESTINATIONS)[number];

export const DEFAULT_DESTINATION: Destination = "overview";

export function isDestination(value: string): value is Destination {
  return (DESTINATIONS as readonly string[]).includes(value);
}

export interface Route {
  /** First path segment — the IA destination. */
  destination: Destination;
  /** Sub-path segments after the destination, e.g. `["plan", "abc-123"]`. */
  path: string[];
  /** Named captures from the matched registered pattern, e.g. `{ id: "abc-123" }`. */
  params: Record<string, string>;
  /** Normalized hash form, e.g. `#/move/plan/abc-123`. */
  hash: string;
}

export function formatHash(
  destination: Destination,
  ...path: string[]
): string {
  const suffix = path.length ? "/" + path.map(encodeURIComponent).join("/") : "";
  return `#/${destination}${suffix}`;
}

/**
 * Split a raw hash into segments. Returns `null` when the hash is empty,
 * malformed, or names an unknown destination.
 */
export function parseHash(hash: string): {
  destination: Destination;
  path: string[];
} | null {
  const trimmed = hash.trim();
  if (!trimmed || trimmed === "#" || trimmed === "#/") return null;
  if (!trimmed.startsWith("#/")) return null;
  const segments = trimmed
    .slice(2)
    .split("/")
    .filter((segment) => segment.length > 0)
    .map((segment) => decodeURIComponent(segment));
  if (!segments.length || !isDestination(segments[0])) return null;
  return { destination: segments[0], path: segments.slice(1) };
}

// ── Route patterns ────────────────────────────────────────────────────
// Destinations register their own sub-route shapes, e.g. Move registers
// "plan/:id" so `#/move/plan/abc-123` parses with `params.id = "abc-123"`.
// A `:name` segment captures one segment; literal segments must match
// exactly. More segments = more specific; patterns are tried most-specific
// first. An unmatched sub-path still parses (rule: deep links stay inert
// but preserved) with `params = {}`.

interface RoutePattern {
  destination: Destination;
  segments: string[];
}

function matchPattern(
  pattern: RoutePattern,
  path: string[],
): Record<string, string> | null {
  if (pattern.segments.length !== path.length) return null;
  const params: Record<string, string> = {};
  for (let i = 0; i < path.length; i++) {
    const segment = pattern.segments[i];
    if (segment.startsWith(":")) {
      params[segment.slice(1)] = path[i];
    } else if (segment !== path[i]) {
      return null;
    }
  }
  return params;
}

// ── Hash source (injectable for tests) ────────────────────────────────

export interface HashSource {
  /** Current raw hash (`"#/move"` or `""`). */
  read(): string;
  /** Push a new hash (fires `hashchange` in the browser). */
  write(hash: string): void;
  /** Replace the hash without a history entry or event. */
  replace(hash: string): void;
  /** Subscribe to external hash changes (back/forward, pasted URLs). */
  onChange(listener: () => void): Unsubscribe;
}

export function windowHashSource(win: Window): HashSource {
  return {
    read: () => win.location.hash,
    write: (hash) => {
      win.location.hash = hash;
    },
    replace: (hash) => {
      win.history.replaceState(null, "", hash);
    },
    onChange: (listener) => {
      win.addEventListener("hashchange", listener);
      return () => win.removeEventListener("hashchange", listener);
    },
  };
}

// ── Router ────────────────────────────────────────────────────────────

export interface RouterOptions {
  source: HashSource;
  /** Called synchronously on every effective route change, including boot. */
  onRoute: (route: Route) => void;
  /**
   * Destination used when the boot hash is empty/invalid — a value, or a
   * function evaluated at `start()` (the composition root passes the
   * legacy-restored section so sessionStorage restore keeps working).
   */
  fallback?: Destination | (() => Destination);
}

export interface Router {
  /** Current route (valid after `start()`). */
  route(): Route;
  /**
   * Register a destination sub-route pattern, e.g. `register("move",
   * "plan/:id")`. Destinations call this for every deep-linkable sub-state
   * they own. Re-registering the same pattern is a no-op.
   */
  register(destination: Destination, pattern: string): void;
  /** Navigate (push). No-op when the hash already matches. */
  navigate(hash: string): void;
  /** Publish the boot route and subscribe to hash changes. */
  start(): void;
  stop(): void;
}

export function createRouter(options: RouterOptions): Router {
  const patterns: RoutePattern[] = [];
  let current: Route | null = null;
  let unsubscribe: Unsubscribe | null = null;

  function resolve(
    parsed: { destination: Destination; path: string[] },
  ): Route {
    const candidates = patterns
      .filter((pattern) => pattern.destination === parsed.destination)
      .sort((a, b) => b.segments.length - a.segments.length);
    for (const pattern of candidates) {
      const params = matchPattern(pattern, parsed.path);
      if (params) {
        return {
          destination: parsed.destination,
          path: parsed.path,
          params,
          hash: formatHash(parsed.destination, ...parsed.path),
        };
      }
    }
    return {
      destination: parsed.destination,
      path: parsed.path,
      params: {},
      hash: formatHash(parsed.destination, ...parsed.path),
    };
  }

  function publish(route: Route): void {
    if (current && current.hash === route.hash) return;
    current = route;
    options.onRoute(route);
  }

  function syncFromSource(): void {
    const parsed = parseHash(options.source.read());
    if (parsed) {
      publish(resolve(parsed));
    }
  }

  return {
    route() {
      if (!current) {
        throw new Error("router used before start()");
      }
      return current;
    },
    register(destination, pattern) {
      const segments = pattern.split("/").filter((segment) => segment.length > 0);
      const exists = patterns.some(
        (candidate) =>
          candidate.destination === destination &&
          candidate.segments.join("/") === segments.join("/"),
      );
      if (!exists) patterns.push({ destination, segments });
    },
    navigate(hash) {
      const parsed = parseHash(hash);
      if (!parsed) return;
      const route = resolve(parsed);
      if (current && current.hash === route.hash) return;
      current = route;
      options.source.write(route.hash);
      options.onRoute(route);
    },
    start() {
      const parsed = parseHash(options.source.read());
      const fallback = options.fallback ?? DEFAULT_DESTINATION;
      const fallbackDestination =
        typeof fallback === "function" ? fallback() : fallback;
      if (parsed) {
        publish(resolve(parsed));
      } else {
        const route = resolve({ destination: fallbackDestination, path: [] });
        options.source.replace(route.hash);
        publish(route);
      }
      if (!unsubscribe) {
        unsubscribe = options.source.onChange(syncFromSource);
      }
    },
    stop() {
      unsubscribe?.();
      unsubscribe = null;
    },
  };
}

// ── Legacy-section adapter (the migration seam — see file header) ─────

/** Hooks into the legacy console's section switcher. */
export interface LegacySectionBridge {
  /** The section the legacy console currently shows (sessionStorage-backed). */
  readSection(): string;
  /**
   * The legacy switcher: stores the section, toggles
   * `.card[data-workspace-section]` visibility, and syncs the nav/topbar.
   */
  selectSection(sectionId: string): void;
}

/** A migrated destination's controller (registered by destination agents). */
export interface DestinationController {
  id: Destination;
  migrated: boolean;
  /** Route entered this destination. `route.path`/`route.params` are yours. */
  mount(route: Route): void;
  /** Route is leaving this destination. */
  unmount(): void;
}

export interface LegacySectionAdapter {
  /** Rule 3: legacy reports its own section changes here. */
  notifyLegacySection(sectionId: string): void;
  /**
   * Router `onRoute` entry point — wire this as the router's onRoute so the
   * adapter observes every effective route change (rules 2 and 4).
   */
  handleRoute(route: Route): void;
  /** Boot the seam (rule 5). Call once, after the legacy console is ready. */
  start(): void;
  /** Look up a destination's controller (undefined when unmigrated). */
  controller(destination: Destination): DestinationController | undefined;
}

export function createLegacySectionAdapter(args: {
  router: Router;
  bridge: LegacySectionBridge;
  destinations?: DestinationController[];
  /** Route publisher — typically `store.set.bind(store, "route")`. */
  onRoute: (route: Route) => void;
}): LegacySectionAdapter {
  const controllers = new Map<Destination, DestinationController>();
  for (const controller of args.destinations ?? []) {
    controllers.set(controller.id, controller);
  }

  let mounted: DestinationController | null = null;
  let started = false;

  function handleRoute(route: Route): void {
    // Fan the route out to the store first so subscribers always see a
    // consistent `route` slice before any side effects below run.
    args.onRoute(route);

    const legacySection = args.bridge.readSection();
    if (legacySection !== route.destination) {
      // Rule 2: legacy chrome tracks the URL. The legacy switcher reports
      // back via notifyLegacySection, which no-ops (same destination).
      args.bridge.selectSection(route.destination);
    }

    // Rule 4: mount/unmount migrated controllers.
    const next = controllers.get(route.destination) ?? null;
    if (next !== mounted) {
      mounted?.unmount();
      mounted = null;
    }
    if (next?.migrated && next !== mounted) {
      next.mount(route);
      mounted = next;
    }
  }

  return {
    notifyLegacySection(sectionId) {
      if (!started || !isDestination(sectionId)) return;
      const route = args.router.route();
      if (route.destination === sectionId) return; // rule 1: keep sub-path
      args.router.navigate(formatHash(sectionId as Destination));
    },
    handleRoute,
    start() {
      if (started) return;
      started = true;
      // Rule 5: a valid hash wins (deep link); otherwise the router's
      // fallback (wired to the legacy-restored section by the composition
      // root) is REPLACED into the URL without a history entry.
      args.router.start();
    },
    controller(destination) {
      return controllers.get(destination);
    },
  };
}

// ── Store wiring helper ───────────────────────────────────────────────

/**
 * Bind a router to a store's `route` slice: every effective route change
 * replaces the slice. Returns the adapter-ready `onRoute` publisher.
 */
export function bindRouteToStore(store: {
  set(key: "route", next: Route): void;
}): (route: Route) => void {
  return (route) => store.set("route", route);
}
