export const REFRESH_INTERVAL_MS = 5000;

export type RefreshMetaState = "busy" | "error" | "live" | "paused";

export interface RefreshRunner {
  queue: () => void;
  run: () => Promise<void>;
}

export function createRefreshRunner(
  cycle: () => Promise<void>,
  onIdle: () => void,
): RefreshRunner {
  let queued = false;
  let active: Promise<void> | null = null;

  async function drain(): Promise<void> {
    try {
      do {
        queued = false;
        await cycle();
      } while (queued);
    } finally {
      // Relinquish ownership before this promise settles. A caller queued in
      // the promise-resolution microtask must start a new drain instead of
      // attaching to a completed-but-still-registered single flight.
      active = null;
      onIdle();
    }
  }

  function queue(): void {
    queued = true;
  }

  function run(): Promise<void> {
    queue();
    active ??= drain();
    return active;
  }

  return { queue, run };
}

let lastRefreshAt: Date | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

function formatRefreshTime(date: Date): string {
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function setRefreshMeta(label: string, state: RefreshMetaState): void {
  const element = document.getElementById("refreshMeta");
  if (!element) {
    return;
  }
  element.textContent = label;
  element.dataset.state = state;
}

export function shouldAutoRefresh(): boolean {
  return document.visibilityState === "visible";
}

export function clearRefreshTimer(): void {
  if (!refreshTimer) {
    return;
  }
  clearTimeout(refreshTimer);
  refreshTimer = null;
}

export function updateRefreshMeta(stateOverride?: "busy" | "error"): void {
  if (stateOverride === "busy") {
    setRefreshMeta("Syncing", "busy");
    return;
  }
  if (stateOverride === "error") {
    setRefreshMeta("Connection issue", "error");
    return;
  }
  const prefix = shouldAutoRefresh() ? "Live" : "Paused";
  const label = lastRefreshAt
    ? `${prefix} · ${formatRefreshTime(lastRefreshAt)}`
    : prefix;
  setRefreshMeta(label, shouldAutoRefresh() ? "live" : "paused");
}

export function markRefreshCompleted(date = new Date()): void {
  lastRefreshAt = date;
  updateRefreshMeta();
}

export function scheduleRefresh(refresh: () => void | Promise<void>): void {
  clearRefreshTimer();
  if (!shouldAutoRefresh()) {
    updateRefreshMeta();
    return;
  }
  updateRefreshMeta();
  refreshTimer = setTimeout(() => {
    void refresh();
  }, REFRESH_INTERVAL_MS);
}
