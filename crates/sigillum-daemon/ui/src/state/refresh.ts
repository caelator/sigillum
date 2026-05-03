export const REFRESH_INTERVAL_MS = 5000;

export type RefreshMetaState = "busy" | "error" | "live" | "paused";

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
