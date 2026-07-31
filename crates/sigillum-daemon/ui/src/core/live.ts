/**
 * core/live.ts — composition root for the strict-typed console core
 * (plan task 4.1).
 *
 * Wires the pieces together for the running console:
 *
 * - one {@link CoreStore} (route/status/operations/queueEvents/sync/resync)
 * - the typed {@link DaemonApi} client
 * - the hash {@link Router} + legacy-section adapter (THE migration seam —
 *   see core/router.ts for the contract destination agents follow)
 * - the SSE {@link EventsClient} feeding the store (with polling fallback)
 * - the proof-of-life migration: the topbar `#refreshMeta` indicator now
 *   renders FROM THE STORE (label/state computed by the legacy refresh loop,
 *   transport by the events client), proving store + SSE end-to-end while
 *   every legacy view keeps its existing behavior.
 *
 * `app.ts` calls {@link startCoreRuntime} once; destination agents consume
 * the returned {@link CoreRuntime} (store/api/router) and register migrated
 * controllers via the adapter — no other legacy wiring is required.
 */

import { setRefreshMetaSink } from "../state/refresh";
import { createDaemonApi, type DaemonApi } from "./api";
import {
  createEventsClient,
  defaultEventSourceFactory,
  type EventSourceFactory,
  type EventsClient,
  type EventsClientOptions,
} from "./events";
import {
  createLegacySectionAdapter,
  createRouter,
  isDestination,
  windowHashSource,
  type Destination,
  type DestinationController,
  type HashSource,
  type LegacySectionAdapter,
  type LegacySectionBridge,
  type Route,
  type Router,
} from "./router";
import {
  createCoreStore,
  type CoreStore,
  type SyncSlice,
} from "./state";

export interface CoreRuntime {
  store: CoreStore;
  api: DaemonApi;
  router: Router;
  adapter: LegacySectionAdapter;
  events: EventsClient;
  /** Legacy → route seam (adapter contract rule 3). */
  notifyLegacySection(sectionId: string): void;
  stop(): void;
}

export interface CoreRuntimeOptions {
  bridge: LegacySectionBridge;
  /**
   * Migrated destination controller factories. Each is called with the
   * finished runtime (store/api/router/events available) and registered on
   * the adapter before it starts.
   */
  destinations?: Array<(runtime: CoreRuntime) => DestinationController>;
  hashSource?: HashSource;
  /** Pass `null` to force the polling fallback (no EventSource). */
  eventSourceFactory?: EventSourceFactory | null;
  /** Boot hash/router sync (default true). */
  router?: boolean;
  /** Boot the SSE/poll events client (default true). */
  events?: boolean;
  /** Render #refreshMeta from the store (default true). */
  refreshMeta?: boolean;
  /** Tuning overrides for the events client (tests). */
  eventsOptions?: Partial<
    Omit<EventsClientOptions, "store" | "api" | "eventSourceFactory">
  >;
}

function renderRefreshMeta(sync: SyncSlice): void {
  const element = document.getElementById("refreshMeta");
  if (!element) return;
  element.textContent = sync.refresh.label;
  element.dataset.state = sync.refresh.state;
  element.dataset.transport = sync.transport;
}

export function startCoreRuntime(options: CoreRuntimeOptions): CoreRuntime {
  const api = createDaemonApi();

  const hashSource = options.hashSource ?? windowHashSource(window);
  // Boot + fallback destination: a valid hash wins at router start; when the
  // hash is empty/invalid the router replaces it with this — the
  // legacy-restored section, so sessionStorage restore keeps working.
  const fallbackDestination = (): Destination => {
    const legacy = options.bridge.readSection();
    return isDestination(legacy) ? legacy : "overview";
  };

  const bootRoute: Route = {
    destination: fallbackDestination(),
    path: [],
    params: {},
    hash: `#/${fallbackDestination()}`,
  };
  const store = createCoreStore(bootRoute);

  // The adapter observes every route change (it fans out to the store and
  // syncs the legacy section); the router funnels through it.
  let routeHandler: (route: Route) => void = (route) =>
    store.set("route", route);
  const router = createRouter({
    source: hashSource,
    fallback: fallbackDestination,
    onRoute: (route) => routeHandler(route),
  });

  const adapter = createLegacySectionAdapter({
    router,
    bridge: options.bridge,
    onRoute: (route) => store.set("route", route),
  });
  routeHandler = adapter.handleRoute;

  const events = createEventsClient({
    store,
    api,
    eventSourceFactory:
      options.eventSourceFactory === null
        ? undefined
        : options.eventSourceFactory ??
          (typeof EventSource === "function"
            ? defaultEventSourceFactory
            : undefined),
    ...(options.eventsOptions ?? {}),
  });

  if (options.refreshMeta !== false) {
    // Proof-of-life: refresh-meta renders from the store. The legacy refresh
    // loop keeps computing the exact same labels; the sink redirects them
    // into the `sync` slice, and the events client adds the transport.
    setRefreshMetaSink((label, state) => {
      store.update("sync", (sync) =>
        sync.refresh.label === label && sync.refresh.state === state
          ? sync
          : { ...sync, refresh: { label, state, at: Date.now() } },
      );
    });
    store.subscribe("sync", renderRefreshMeta);
  }

  const runtime: CoreRuntime = {
    store,
    api,
    router,
    adapter,
    events,
    notifyLegacySection: (sectionId) => adapter.notifyLegacySection(sectionId),
    stop() {
      events.stop();
      router.stop();
      setRefreshMetaSink(null);
    },
  };

  // Destination factories run against the finished runtime, then register on
  // the adapter — before it starts, so a boot deep link mounts correctly.
  for (const factory of options.destinations ?? []) {
    adapter.register(factory(runtime));
  }

  if (options.router !== false) {
    adapter.start();
  }
  if (options.events !== false) {
    events.start();
  }

  return runtime;
}
