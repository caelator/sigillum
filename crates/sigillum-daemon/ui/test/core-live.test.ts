import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import { startCoreRuntime } from "../src/core/live";
import {
  clearRefreshTimer,
  markRefreshCompleted,
  scheduleRefresh,
  shouldAutoRefresh,
  updateRefreshMeta,
} from "../src/state/refresh";
import { installDom } from "./dom-fixture";
import {
  fakeBridge,
  MemoryHashSource,
  MockEventSource,
  mockFetchJson,
  sampleStatus,
  tick,
} from "./core-helpers";

test("proof-of-life: refresh-meta renders store sync state incl. SSE transport", async () => {
  const dom = installDom(["refreshMeta"]);
  (dom.document as unknown as { visibilityState: string }).visibilityState =
    "visible";
  MockEventSource.instances = [];
  const source = new MemoryHashSource();
  const bridge = fakeBridge("overview");
  mockFetchJson((path: string) => {
    if (path.startsWith("/api/status")) return sampleStatus(false);
    if (path.startsWith("/api/operations")) return { operations: [] };
    return {};
  });

  const runtime = startCoreRuntime({
    bridge,
    hashSource: source,
    eventSourceFactory: (url) => new MockEventSource(url),
    eventsOptions: { sessionToken: () => "tok" },
  });
  const meta = dom.el("refreshMeta");

  try {
    // Router booted through the adapter: hash normalized from the legacy section.
    equal(source.hash, "#/overview");
    equal(runtime.store.get("route").destination, "overview");

    // SSE opens → the transport slice lands on the topbar dot.
    MockEventSource.instances[0].emit("open");
    await tick();
    equal(meta.dataset.transport, "sse");

    // Legacy refresh loop computes the same labels as before, now via the store.
    updateRefreshMeta("busy");
    await tick();
    equal(meta.textContent, "Syncing");
    equal(meta.dataset.state, "busy");

    markRefreshCompleted(new Date(2026, 0, 1, 12, 0, 0));
    await tick();
    ok(meta.textContent.startsWith("Live · "), `label: ${meta.textContent}`);
    equal(meta.dataset.state, "live");
  } finally {
    runtime.stop();
  }

  // After stop, the legacy direct-DOM path is restored exactly as before.
  updateRefreshMeta("error");
  equal(meta.textContent, "Connection issue");
  equal(meta.dataset.state, "error");
});

test("refresh scheduling pauses hidden documents without arming a timer", () => {
  const dom = installDom(["refreshMeta"]);
  const meta = dom.el("refreshMeta");
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  let scheduledTimers = 0;
  let clearedTimers = 0;

  globalThis.setTimeout = ((..._args: Parameters<typeof setTimeout>) => {
    scheduledTimers += 1;
    return 1 as unknown as ReturnType<typeof setTimeout>;
  }) as typeof setTimeout;
  globalThis.clearTimeout = ((..._args: Parameters<typeof clearTimeout>) => {
    clearedTimers += 1;
  }) as typeof clearTimeout;

  try {
    (dom.document as unknown as { visibilityState: string }).visibilityState =
      "hidden";
    scheduleRefresh(() => {});
    equal(shouldAutoRefresh(), false);
    equal(scheduledTimers, 0);
    equal(meta.dataset.state, "paused");

    (dom.document as unknown as { visibilityState: string }).visibilityState =
      "visible";
    scheduleRefresh(() => {});
    equal(shouldAutoRefresh(), true);
    equal(scheduledTimers, 1);
    equal(meta.dataset.state, "live");

    (dom.document as unknown as { visibilityState: string }).visibilityState =
      "hidden";
    scheduleRefresh(() => {});
    equal(shouldAutoRefresh(), false);
    equal(scheduledTimers, 1);
    equal(clearedTimers, 1);
    equal(meta.dataset.state, "paused");
  } finally {
    clearRefreshTimer();
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});
