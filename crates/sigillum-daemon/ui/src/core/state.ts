/**
 * core/state.ts — the shared store slices of the console core (plan task 4.1).
 *
 * One store, five slices. Migrated views read daemon state from here (fed
 * live by `core/events.ts` via SSE, or the polling fallback) instead of
 * fetching in a 5 s full-workspace loop like the legacy console. Slices are
 * replaced wholesale; subscribers fire only on reference change.
 */

import type {
  Operation,
  QueueJobEvent,
  StatusResponse,
} from "../contracts";
import type { Route } from "./router";
import { createStore, type Store } from "./store";

/** Live-ness of the events transport feeding the store. */
export type EventTransport =
  | "connecting"
  | "sse"
  | "poll"
  | "error"
  | "off";

/** Topbar refresh-meta state, written by BOTH the legacy refresh loop and the events client. */
export interface SyncSlice {
  refresh: {
    /** Display label exactly as the legacy refresh computes it ("Live · 12:00:01", …). */
    label: string;
    state: "busy" | "error" | "live" | "paused";
    /** unix-ms of the last completed refresh, null before the first. */
    at: number | null;
  };
  /** Events transport currently feeding the store (proof-of-life surface). */
  transport: EventTransport;
}

export interface CoreSlices {
  /** Current hash route (core/router.ts). Always present after boot. */
  route: Route;
  /** Full status payload; null until the first successful fetch/snapshot. */
  status: StatusResponse | null;
  /** Operation registry entries, upserted from SSE events / snapshots. */
  operations: Operation[];
  /**
   * Recent queue job state transitions (newest first, capped). Carries only
   * `job_id`/`state` by design — views owning the queue list refetch the
   * full records via `core/api.ts` when this slice moves.
   */
  queueEvents: QueueJobEvent[];
  /** Refresh-meta + transport state for the topbar dot. */
  sync: SyncSlice;
  /**
   * Monotonic counter bumped on every SSE resync snapshot (connect and
   * lag-recovery). Views subscribe and idempotently refetch the resources
   * they render — the snapshot itself only covers status/operations.
   */
  resync: number;
}

export type CoreStore = Store<CoreSlices>;

export const INITIAL_SYNC: SyncSlice = {
  refresh: { label: "syncing...", state: "busy", at: null },
  transport: "connecting",
};

export function createCoreStore(initialRoute: Route): CoreStore {
  return createStore<CoreSlices>({
    route: initialRoute,
    status: null,
    operations: [],
    queueEvents: [],
    sync: INITIAL_SYNC,
    resync: 0,
  });
}
