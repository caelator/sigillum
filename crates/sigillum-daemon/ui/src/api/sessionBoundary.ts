import { daemonHttpStatus } from "./session";
export const SESSION_BOUNDARY_EVENT_KEY = "sigillumSessionBoundary";
export const SESSION_BOUNDARY_SEEN_KEY = "sigillumSessionBoundarySeen";
const BOUNDARY_PATHS = new Set([
  "/api/compartment/switch",
  "/api/lock",
  "/api/setup/reset",
  "/api/backup/restore",
  "/api/session/revoke",
]);
export function isSessionBoundaryPath(path: string): boolean {
  return BOUNDARY_PATHS.has(path);
}
export function isSessionBoundarySuccess(path: string, payload: any): boolean {
  const status = daemonHttpStatus(payload);
  if (
    status == null || status < 200 || status >= 300 ||
    Object.prototype.hasOwnProperty.call(payload || {}, "error")
  ) return false;
  if (path === "/api/setup/reset") {
    return payload.status === "reset" &&
      (payload.archived_to == null || typeof payload.archived_to === "string");
  }
  if (path === "/api/backup/restore") {
    return payload.status === "restored" && payload.requires_reauth === true &&
      payload.summary && typeof payload.summary === "object" &&
      !Array.isArray(payload.summary);
  }
  if (path === "/api/session/revoke") return payload.status === "revoked" &&
    payload.requires_reauth === true;
  return false;
}
export interface SessionBoundaryChannel {
  current: () => string | null; seen: () => string | null;
  markSeen: (value: string) => void;
  publish: (reason: string, phase: "pending" | "settled") => string | null;
  subscribe: (listener: (value: string) => void) => () => void;
}
export interface SessionBoundaryStateDeps {
  channel?: SessionBoundaryChannel;
  invalidateOwner: () => void; abortReads: () => void;
  clearToken: () => void; scrubPrivateWorkspace: () => void;
  resetPrivateState: () => void;
  closeSessionUi: (forcePrivateReset?: boolean) => void;
  setTransitionUi: (active: boolean) => void; markRefreshQueued: () => void;
}
let browserEventSerial = 0;
function browserChannel(): SessionBoundaryChannel {
  const current = () => {
    try { return globalThis.localStorage?.getItem(SESSION_BOUNDARY_EVENT_KEY) || null; }
    catch (_) { return null; }
  };
  const seen = () => {
    try { return globalThis.sessionStorage?.getItem(SESSION_BOUNDARY_SEEN_KEY) || null; }
    catch (_) { return null; }
  };
  const markSeen = (value: string) => {
    try { globalThis.sessionStorage?.setItem(SESSION_BOUNDARY_SEEN_KEY, value); }
    catch (_) {}
  };
  const publish = (reason: string, phase: "pending" | "settled") => {
    const value = JSON.stringify({
      version: 1,
      id: `${Date.now()}:${++browserEventSerial}:${Math.random()}`,
      phase,
      reason,
    });
    markSeen(value);
    try {
      if (!globalThis.localStorage) return null;
      globalThis.localStorage.setItem(SESSION_BOUNDARY_EVENT_KEY, value);
      return value;
    } catch (_) { return null; }
  };
  const subscribe = (listener: (value: string) => void) => {
    if (typeof globalThis.addEventListener !== "function") return () => undefined;
    const onStorage = (event: Event) => {
      const storageEvent = event as StorageEvent;
      if (storageEvent.key === SESSION_BOUNDARY_EVENT_KEY && storageEvent.newValue) {
        listener(storageEvent.newValue);
      }
    };
    globalThis.addEventListener("storage", onStorage);
    return () => globalThis.removeEventListener("storage", onStorage);
  };
  return { current, markSeen, publish, seen, subscribe };
}
export function createSessionBoundaryState(deps: SessionBoundaryStateDeps) {
  const channel = deps.channel || browserChannel();
  let generation = 0;
  let lastHandled: string | null = null;
  let started = false;

  function isPending(value: string): boolean {
    try {
      const event = JSON.parse(value);
      return event?.version !== 1 || event.phase !== "settled" ||
        typeof event.id !== "string" || event.id.length === 0 ||
        typeof event.reason !== "string" || !BOUNDARY_PATHS.has(event.reason);
    }
    catch (_) { return true; }
  }

  function invalidateLocal(): void {
    generation += 1;
    deps.invalidateOwner();
    deps.abortReads();
    deps.clearToken();
    deps.scrubPrivateWorkspace();
    deps.resetPrivateState();
    deps.closeSessionUi(true);
    deps.setTransitionUi(false);
    deps.markRefreshQueued();
  }

  function invalidate(value: string): void {
    if (value === lastHandled) return;
    if (value === channel.seen() && !isPending(value)) return;
    lastHandled = value;
    channel.markSeen(value);
    invalidateLocal();
  }

  function publish(path: string): boolean {
    if (!BOUNDARY_PATHS.has(path)) return true;
    generation += 1;
    const value = channel.publish(path, "pending");
    if (!value) return false;
    if (value && value !== channel.seen()) channel.markSeen(value);
    return true;
  }

  function settle(path: string): void {
    if (!BOUNDARY_PATHS.has(path)) return;
    generation += 1;
    const value = channel.publish(path, "settled");
    if (value && value !== channel.seen()) channel.markSeen(value);
  }

  function start(): void {
    if (started) return;
    started = true;
    channel.subscribe(invalidate);
    const current = channel.current();
    if (current) invalidate(current);
  }

  return { generation: () => generation, invalidateLocal, publish, settle, start };
}
