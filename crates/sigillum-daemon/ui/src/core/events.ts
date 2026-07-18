/**
 * core/events.ts — SSE client feeding the store (plan tasks 1.3 + 4.1).
 *
 * Subscribes to `GET /api/events` (decision D-D: loopback-only, session via
 * `?session=` because browser `EventSource` cannot set headers) and keeps
 * the `status`, `operations`, and `queueEvents` slices live:
 *
 * - `snapshot`  → reconcile live `operations`, passively fetch and reconcile
 *                 the bounded history from `GET /api/operations`, refetch
 *                 full status via `GET /api/status` (snapshots carry only the
 *                 locked flag), and bump `resync` so views refetch their own
 *                 resources. Snapshots arrive on connect AND on lag recovery.
 * - `operation` → upsert the full record into `operations` by id.
 * - `queue`     → prepend to `queueEvents` (capped); full records come from
 *                 `core/api.ts` list calls, per the slice contract.
 * - `status`    → refetch full status (lock/unlock/switch change it all).
 *
 * Resilience: consecutive transport errors reconnect with exponential
 * backoff; after {@link EventsClientOptions.maxConsecutiveErrors} the client
 * falls back to interval polling of the same slices via the PASSIVE
 * endpoints (`/api/status`, `/api/operations` — neither extends the idle
 * lock, matching the daemon's documented rule). A console that boots LOCKED
 * has no session token: the client starts in poll mode and upgrades to SSE
 * automatically once a poll succeeds with a token present (that is the only
 * auto-upgrade case — SSE failures with a token stay on the fallback, so a
 * daemon without the events route never thrashes). Unknown event names and
 * unknown payload fields are ignored per the 1.x compatibility rule.
 *
 * This REPLACES the legacy 5 s full-workspace refetch for migrated views
 * only; the legacy loop keeps running untouched until each view migrates.
 */

import {
  EVENTS_PROTOCOL_VERSION,
  EVENT_NAME_OPERATION,
  EVENT_NAME_QUEUE,
  EVENT_NAME_SNAPSHOT,
  EVENT_NAME_STATUS,
  type EventsSnapshot,
  type Operation,
  type OperationEvent,
  type QueueJobEvent,
  type StatusEvent,
  type StatusResponse,
} from "../contracts";
import { readSessionToken, subscribeSessionToken } from "../api/session";
import type { DaemonApi } from "./api";
import type { CoreStore, EventTransport } from "./state";

/** Max queue events retained in the slice (newest first). */
export const QUEUE_EVENTS_CAP = 50;
export const DEFAULT_MAX_CONSECUTIVE_ERRORS = 3;
export const DEFAULT_POLL_INTERVAL_MS = 5000;
export const MAX_RECONNECT_DELAY_MS = 30000;

const TERMINAL_OPERATION_STATES = new Set(["canceled", "completed", "failed"]);
const LIVE_OPERATION_STATES = new Set(["running", "cancel_requested"]);

function isTerminalOperation(operation: Operation): boolean {
  return TERMINAL_OPERATION_STATES.has(operation.state);
}

function isKnownLiveOperation(operation: Operation): boolean {
  return LIVE_OPERATION_STATES.has(operation.state);
}

/**
 * An SSE snapshot is authoritative for live operations, but intentionally
 * omits terminal records. Keep only terminal records from the prior slice and
 * let a snapshot record win if an id appears in both sets.
 */
function applyLiveSnapshot(
  liveOperations: Operation[],
  currentOperations: Operation[],
): Operation[] {
  const seen = new Set<string>();
  const next: Operation[] = [];
  for (const operation of liveOperations) {
    if (seen.has(operation.id)) continue;
    seen.add(operation.id);
    next.push(operation);
  }
  for (const operation of currentOperations) {
    if (!isTerminalOperation(operation) || seen.has(operation.id)) continue;
    seen.add(operation.id);
    next.push(operation);
  }
  return next;
}

/**
 * Reconcile a completed bounded-list request with the live slice. The list is
 * authoritative for which pre-request terminal records are still retained by
 * the daemon. Current records win for ids that are in both sets, and records
 * created or changed while the request was in flight are kept even when the
 * response snapshot does not contain them yet.
 *
 * All listed records are accepted, not just the terminal states known by this
 * client: operation states are an opaque, forward-compatible wire string.
 */
function reconcileOperationHistory(
  currentOperations: Operation[],
  listedOperations: Operation[],
  requestBaseline: Map<string, Operation>,
): Operation[] {
  const currentById = new Map<string, Operation>();
  for (const operation of currentOperations) {
    if (!currentById.has(operation.id)) currentById.set(operation.id, operation);
  }

  const seen = new Set<string>();
  const next: Operation[] = [];

  // Keep snapshot-live work and in-flight SSE transitions at the front of the
  // slice, where existing consumers expect active work to appear. A prior
  // non-live record absent from the list was evicted and is deliberately not
  // retained; only the server can classify an opaque future state reliably.
  for (const current of currentOperations) {
    const baseline = requestBaseline.get(current.id);
    const changedWhileInFlight = baseline === undefined || baseline !== current;
    if (
      seen.has(current.id) ||
      (!isKnownLiveOperation(current) && !changedWhileInFlight)
    ) {
      continue;
    }
    seen.add(current.id);
    next.push(current);
  }

  for (const listed of listedOperations) {
    if (seen.has(listed.id)) continue;
    seen.add(listed.id);
    // A live snapshot or a newer SSE transition must not be overwritten by a
    // list response that was serialized before that frame reached the client.
    next.push(currentById.get(listed.id) ?? listed);
  }

  return next;
}

/** Minimal EventSource surface the client relies on (mockable in tests). */
export interface EventSourceLike {
  addEventListener(
    type: string,
    listener: (event: { data?: string }) => void,
  ): void;
  close(): void;
}

export type EventSourceFactory = (url: string) => EventSourceLike;

export function defaultEventSourceFactory(url: string): EventSourceLike {
  return new EventSource(url) as unknown as EventSourceLike;
}

/** Parse one SSE frame. Returns null for unknown event names (ignored). */
export function parseDaemonEvent(
  eventName: string,
  data: string,
):
  | { name: typeof EVENT_NAME_SNAPSHOT; payload: EventsSnapshot }
  | { name: typeof EVENT_NAME_OPERATION; payload: OperationEvent }
  | { name: typeof EVENT_NAME_QUEUE; payload: QueueJobEvent }
  | { name: typeof EVENT_NAME_STATUS; payload: StatusEvent }
  | null {
  let payload: unknown;
  try {
    payload = JSON.parse(data);
  } catch (_) {
    return null;
  }
  if (typeof payload !== "object" || payload === null) return null;
  const versioned = payload as { v?: number };
  if (versioned.v !== EVENTS_PROTOCOL_VERSION) return null;
  switch (eventName) {
    case EVENT_NAME_SNAPSHOT:
      return { name: EVENT_NAME_SNAPSHOT, payload: payload as EventsSnapshot };
    case EVENT_NAME_OPERATION:
      return { name: EVENT_NAME_OPERATION, payload: payload as OperationEvent };
    case EVENT_NAME_QUEUE:
      return { name: EVENT_NAME_QUEUE, payload: payload as QueueJobEvent };
    case EVENT_NAME_STATUS:
      return { name: EVENT_NAME_STATUS, payload: payload as StatusEvent };
    default:
      return null;
  }
}

export interface EventsClientOptions {
  store: CoreStore;
  api: Pick<DaemonApi, "getStatus" | "listOperations">;
  eventSourceFactory?: EventSourceFactory;
  /** Session token source; defaults to the session storage token. */
  sessionToken?: () => string | null;
  /** Same-tab token change source; inferred for the default token source. */
  sessionTokenChanges?: (
    listener: (token: string | null) => void,
  ) => () => void;
  eventsPath?: string;
  /** Consecutive SSE errors before falling back to polling (default 3). */
  maxConsecutiveErrors?: number;
  pollIntervalMs?: number;
  /** Backoff between SSE reconnect attempts; default is exp (1s..30s cap). */
  reconnectDelayMs?: (attempt: number) => number;
  /** Test seam for timers; defaults to the global functions. */
  setTimeoutFn?: (fn: () => void, ms: number) => unknown;
  clearTimeoutFn?: (handle: unknown) => void;
  setIntervalFn?: (fn: () => void, ms: number) => unknown;
  clearIntervalFn?: (handle: unknown) => void;
}

export interface EventsClient {
  start(): void;
  stop(): void;
  transport(): EventTransport;
}

export function createEventsClient(options: EventsClientOptions): EventsClient {
  const {
    store,
    api,
    eventsPath = "/api/events",
    maxConsecutiveErrors = DEFAULT_MAX_CONSECUTIVE_ERRORS,
    pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
    reconnectDelayMs = (attempt: number) =>
      Math.min(1000 * 2 ** (attempt - 1), MAX_RECONNECT_DELAY_MS),
    setTimeoutFn = (fn, ms) => setTimeout(fn, ms),
    clearTimeoutFn = (handle) => clearTimeout(handle as never),
    setIntervalFn = (fn, ms) => setInterval(fn, ms),
    clearIntervalFn = (handle) => clearInterval(handle as never),
  } = options;
  const sessionToken = options.sessionToken ?? readSessionToken;
  const sessionTokenChanges =
    options.sessionTokenChanges ??
    (options.sessionToken === undefined ? subscribeSessionToken : undefined);

  let transport: EventTransport = "off";
  let source: EventSourceLike | null = null;
  let consecutiveErrors = 0;
  let reconnectTimer: unknown = null;
  let pollTimer: unknown = null;
  let running = false;
  let unsubscribeSessionToken: (() => void) | null = null;
  // True when the last SSE failure (or boot) happened with no session token —
  // the one case where a later poll success upgrades back to SSE (unlock).
  let lastErrorHadNoSession = false;
  // Guards against stale async completions after stop().
  let generation = 0;
  // Invalidates a terminal-history request when a newer snapshot arrives.
  let snapshotRevision = 0;
  // Last-trigger-wins guards for overlapping passive reads in one generation.
  let statusRevision = 0;
  let pollRequestRevision = 0;
  let historyRetryTimer: unknown = null;
  let historyRetryAttempt = 0;

  function setTransport(next: EventTransport): void {
    transport = next;
    store.update("sync", (sync) =>
      sync.transport === next ? sync : { ...sync, transport: next },
    );
  }

  async function refetchStatus(): Promise<void> {
    const gen = generation;
    const revision = ++statusRevision;
    const status: StatusResponse = await api.getStatus();
    if (!running || gen !== generation || revision !== statusRevision) return;
    store.set("status", status);
  }

  function clearHistoryRetry(resetAttempt = true): void {
    if (historyRetryTimer !== null) {
      clearTimeoutFn(historyRetryTimer);
      historyRetryTimer = null;
    }
    if (resetAttempt) historyRetryAttempt = 0;
  }

  function invalidateSnapshotHistory(): void {
    snapshotRevision += 1;
    clearHistoryRetry();
  }

  function scheduleHistoryRetry(gen: number, revision: number): void {
    if (
      !running ||
      gen !== generation ||
      revision !== snapshotRevision ||
      historyRetryTimer !== null
    ) {
      return;
    }
    historyRetryAttempt += 1;
    const delay = Math.min(
      1000 * 2 ** (historyRetryAttempt - 1),
      MAX_RECONNECT_DELAY_MS,
    );
    historyRetryTimer = setTimeoutFn(() => {
      historyRetryTimer = null;
      if (!running || gen !== generation || revision !== snapshotRevision) return;
      void refetchTerminalHistory(gen, revision);
    }, delay);
  }

  async function refetchTerminalHistory(
    gen: number,
    revision: number,
  ): Promise<void> {
    const requestBaseline = new Map(
      store.get("operations").map((operation) => [operation.id, operation]),
    );
    try {
      const response = await api.listOperations();
      if (!running || gen !== generation || revision !== snapshotRevision) return;
      clearHistoryRetry();
      store.update("operations", (operations) =>
        reconcileOperationHistory(
          operations,
          response.operations,
          requestBaseline,
        ),
      );
    } catch (_) {
      // The snapshot still carries complete live state. Retry this passive
      // enrichment without degrading the healthy SSE transport.
      scheduleHistoryRetry(gen, revision);
    }
  }

  async function pollTick(): Promise<void> {
    if (!running || pollTimer === null) return;
    const gen = generation;
    const pollRevision = ++pollRequestRevision;
    const pollStatusRevision = ++statusRevision;
    try {
      const [status, operations] = await Promise.all([
        api.getStatus(),
        api.listOperations(),
      ]);
      if (
        !running ||
        gen !== generation ||
        pollRevision !== pollRequestRevision ||
        pollTimer === null
      ) {
        return;
      }
      if (pollStatusRevision === statusRevision) store.set("status", status);
      store.set("operations", operations.operations);
      setTransport("poll");
      maybeUpgradeToSse();
    } catch (_) {
      if (
        !running ||
        gen !== generation ||
        pollRevision !== pollRequestRevision ||
        pollTimer === null
      ) {
        return;
      }
      setTransport("error");
    }
  }

  // Poll succeeded while the previous SSE failure was session-less: the
  // operator has unlocked since, so give the push channel another try.
  function maybeUpgradeToSse(): void {
    if (source || !lastErrorHadNoSession || !sessionToken()) return;
    lastErrorHadNoSession = false;
    consecutiveErrors = 0;
    stopPolling();
    connect();
  }

  function startPolling(): void {
    stopSource();
    if (pollTimer !== null) return;
    // A completed poll replaces the full operations slice, so no snapshot
    // request from the retired SSE transport may merge into it afterward.
    invalidateSnapshotHistory();
    setTransport("poll");
    pollTimer = setIntervalFn(() => void pollTick(), pollIntervalMs);
    void pollTick();
  }

  function stopPolling(): void {
    // Clearing an interval does not cancel callbacks or requests already in
    // flight. Invalidate both before connecting a new SSE source.
    pollRequestRevision += 1;
    if (pollTimer !== null) {
      clearIntervalFn(pollTimer);
      pollTimer = null;
    }
  }

  function stopSource(): void {
    if (source) {
      source.close();
      source = null;
    }
    if (reconnectTimer !== null) {
      clearTimeoutFn(reconnectTimer);
      reconnectTimer = null;
    }
  }

  function handleSessionTokenChange(): void {
    if (!running) return;
    // The daemon passively revalidates an EventSource for its full lifetime,
    // but a revoked or rotated browser token still retires the local source
    // immediately so the replacement token reconnects without waiting for
    // the bounded server-side revalidation interval.
    generation += 1;
    invalidateSnapshotHistory();
    consecutiveErrors = 0;
    stopSource();
    stopPolling();
    lastErrorHadNoSession = !sessionToken();
    connect();
  }

  function handleEvent(name: string, data: string): void {
    const event = parseDaemonEvent(name, data);
    if (!event) return;
    switch (event.name) {
      case EVENT_NAME_SNAPSHOT: {
        const gen = generation;
        clearHistoryRetry();
        const revision = ++snapshotRevision;
        store.update("operations", (operations) =>
          applyLiveSnapshot(event.payload.operations ?? [], operations),
        );
        store.update("resync", (n) => n + 1);
        void refetchTerminalHistory(gen, revision);
        // The snapshot carries only the locked flag; the full status (init
        // state, compartments) comes from the passive status endpoint.
        void refetchStatus().catch(() => {});
        break;
      }
      case EVENT_NAME_OPERATION: {
        const record: Operation = event.payload.operation;
        store.update("operations", (operations) => {
          const index = operations.findIndex((op) => op.id === record.id);
          if (index < 0) return [...operations, record];
          if (operations[index] === record) return operations;
          const next = operations.slice();
          next[index] = record;
          return next;
        });
        break;
      }
      case EVENT_NAME_QUEUE: {
        const entry = event.payload;
        store.update("queueEvents", (events) =>
          [entry, ...events].slice(0, QUEUE_EVENTS_CAP),
        );
        break;
      }
      case EVENT_NAME_STATUS: {
        void refetchStatus().catch(() => {});
        break;
      }
    }
  }

  function handleSourceError(): void {
    if (!running) return;
    consecutiveErrors += 1;
    lastErrorHadNoSession = !sessionToken();
    stopSource();
    if (consecutiveErrors >= maxConsecutiveErrors) {
      // SSE is not settling — degrade to the passive polling fallback.
      startPolling();
      return;
    }
    setTransport("connecting");
    reconnectTimer = setTimeoutFn(
      () => connect(),
      reconnectDelayMs(consecutiveErrors),
    );
  }

  function connect(): void {
    if (!running) return;
    // No session (locked console): the stream would 401 — poll instead and
    // let maybeUpgradeToSse bring SSE up after unlock.
    if (!sessionToken()) {
      lastErrorHadNoSession = true;
      startPolling();
      return;
    }
    const factory = options.eventSourceFactory;
    if (!factory) {
      // No EventSource in this environment — straight to the fallback.
      startPolling();
      return;
    }
    setTransport("connecting");
    const token = sessionToken();
    const url = token
      ? `${eventsPath}?session=${encodeURIComponent(token)}`
      : eventsPath;
    const es = factory(url);
    source = es;
    es.addEventListener("open", () => {
      if (!running || source !== es) return;
      consecutiveErrors = 0;
      setTransport("sse");
    });
    es.addEventListener("error", () => {
      if (source !== es) return;
      handleSourceError();
    });
    for (const name of [
      EVENT_NAME_SNAPSHOT,
      EVENT_NAME_OPERATION,
      EVENT_NAME_QUEUE,
      EVENT_NAME_STATUS,
    ]) {
      es.addEventListener(name, (event) => {
        if (!running || source !== es) return;
        handleEvent(name, event.data ?? "");
      });
    }
  }

  return {
    start() {
      if (running) return;
      running = true;
      generation += 1;
      unsubscribeSessionToken =
        sessionTokenChanges?.(() => handleSessionTokenChange()) ?? null;
      connect();
    },
    stop() {
      if (!running) return;
      running = false;
      generation += 1;
      unsubscribeSessionToken?.();
      unsubscribeSessionToken = null;
      invalidateSnapshotHistory();
      stopSource();
      stopPolling();
      setTransport("off");
    },
    transport() {
      return transport;
    },
  };
}
