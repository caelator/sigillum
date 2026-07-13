export const LOCK_UNCONFIRMED_LATCH_KEY = "sigillumLockUnconfirmed";

export interface LockLatchStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export type LockLatchSubscriber = (listener: (latched: boolean) => void) => () => void;

export interface LockUnconfirmedStateDeps {
  lockWithToken: (token: string) => Promise<boolean>;
  clearGeneralToken: () => void;
  closeSessionUi: () => void;
  clearWarning: () => void;
  showWarning: (canRetry: boolean, canAcknowledge: boolean) => void;
  storage?: LockLatchStorage | null;
  subscribe?: LockLatchSubscriber;
}

export type LockContainment = "confirmed" | "unconfirmed" | "obsolete";

function browserStorage(): LockLatchStorage | null {
  try { return globalThis.localStorage; }
  catch (_) { return null; }
}

function browserSubscriber(listener: (latched: boolean) => void): () => void {
  if (typeof globalThis.addEventListener !== "function") return () => undefined;
  const onStorage = (event: Event) => {
    const storageEvent = event as StorageEvent;
    if (storageEvent.key !== LOCK_UNCONFIRMED_LATCH_KEY) return;
    listener(storageEvent.newValue === "1");
  };
  globalThis.addEventListener("storage", onStorage);
  return () => globalThis.removeEventListener("storage", onStorage);
}

export function createLockUnconfirmedState(deps: LockUnconfirmedStateDeps) {
  const storage = deps.storage === undefined ? browserStorage() : deps.storage;
  const subscribe = deps.subscribe || browserSubscriber;
  let isolatedLockToken: string | null = null;
  let volatileLatch = false;
  let retrying = false;
  let containing = false;
  let stateVersion = 0;

  function readLatch(): boolean {
    try {
      return storage?.getItem(LOCK_UNCONFIRMED_LATCH_KEY) === "1" || volatileLatch;
    } catch (_) {
      return volatileLatch;
    }
  }

  function writeLatch(latched: boolean): void {
    volatileLatch = latched;
    try {
      if (latched) storage?.setItem(LOCK_UNCONFIRMED_LATCH_KEY, "1");
      else storage?.removeItem(LOCK_UNCONFIRMED_LATCH_KEY);
    } catch (_) {}
  }

  function isolate(token: string | null): void {
    stateVersion += 1;
    isolatedLockToken = token;
    writeLatch(true);
  }

  function clear(): void {
    stateVersion += 1;
    isolatedLockToken = null;
    writeLatch(false);
  }

  function canRetry(): boolean {
    return readLatch() && Boolean(isolatedLockToken) && !retrying && !containing;
  }

  function canAcknowledgeRestart(): boolean {
    return readLatch() && isolatedLockToken == null && !retrying && !containing;
  }

  async function contain(token: string | null, stillCurrent: () => boolean):
    Promise<LockContainment> {
    if (!stillCurrent() || containing || readLatch()) return "obsolete";
    containing = true;
    // Start Lock(T), then remove T from general storage in the same turn.
    let attempt = Promise.resolve(false);
    try {
      if (token) attempt = deps.lockWithToken(token).catch(() => false);
    } catch (_) {}
    isolate(token);
    const containmentVersion = stateVersion;
    deps.clearGeneralToken();
    const confirmed = await attempt;
    containing = false;
    const obsolete = !stillCurrent() || stateVersion !== containmentVersion ||
      !readLatch() || isolatedLockToken !== token;
    if (obsolete) return "obsolete";
    // No late writer may promote a replacement token while containment owns UI.
    deps.clearGeneralToken();
    if (confirmed) {
      clear();
      deps.closeSessionUi();
      deps.clearWarning();
    } else {
      deps.closeSessionUi();
      deps.showWarning(canRetry(), canAcknowledgeRestart());
    }
    return confirmed ? "confirmed" : "unconfirmed";
  }

  async function retry(): Promise<boolean> {
    const token = isolatedLockToken;
    if (!token || retrying || containing || !readLatch()) return false;
    retrying = true;
    const retryVersion = stateVersion;
    let confirmed = false;
    try {
      confirmed = await deps.lockWithToken(token).catch(() => false);
      if (stateVersion !== retryVersion || isolatedLockToken !== token) {
        return false;
      }
      if (confirmed) {
        clear();
        deps.clearGeneralToken();
        deps.closeSessionUi();
        deps.clearWarning();
      }
      return confirmed;
    } finally {
      retrying = false;
      if (
        !confirmed && stateVersion === retryVersion &&
        isolatedLockToken === token && readLatch()
      ) deps.showWarning(canRetry(), canAcknowledgeRestart());
    }
  }

  function restore(): boolean {
    if (!readLatch()) return false;
    deps.clearGeneralToken();
    deps.closeSessionUi();
    deps.showWarning(canRetry(), canAcknowledgeRestart());
    return true;
  }

  function acknowledgeRestart(confirmation: string): boolean {
    if (!canAcknowledgeRestart() || confirmation !== "I STOPPED SIGILLUM") {
      return false;
    }
    clear();
    deps.clearWarning();
    return true;
  }

  function listen(listener: (latched: boolean) => void): () => void {
    return subscribe((latched) => {
      // Storage events are cross-tab evidence only. Never import a token from
      // another tab; an external clear also destroys this tab's stale retry.
      stateVersion += 1;
      isolatedLockToken = null;
      volatileLatch = latched;
      listener(latched);
    });
  }

  return {
    acknowledgeRestart,
    canAcknowledgeRestart,
    canRetry,
    contain,
    isLatched: readLatch,
    listen,
    restore,
    retry,
  };
}
