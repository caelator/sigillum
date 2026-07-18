import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import {
  createEventsClient,
  parseDaemonEvent,
  QUEUE_EVENTS_CAP,
} from "../src/core/events";
import { createCoreStore } from "../src/core/state";
import type { Operation } from "../src/contracts";
import { installDom } from "./dom-fixture";
import {
  BOOT_ROUTE,
  MockEventSource,
  sampleOperation,
  sampleStatus,
  sleep,
  tick,
} from "./core-helpers";

interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: Error): void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

test("parseDaemonEvent ignores unknown names, junk, and wrong versions", () => {
  equal(parseDaemonEvent("telemetry", "{}"), null);
  equal(parseDaemonEvent("snapshot", "not json"), null);
  equal(parseDaemonEvent("snapshot", JSON.stringify({ v: 2 })), null);
  equal(parseDaemonEvent("status", JSON.stringify({ v: 0 })), null);
  const parsed = parseDaemonEvent(
    "queue",
    JSON.stringify({ v: 1, job_id: "j1", state: "sent" }),
  );
  equal(parsed?.name, "queue");
});

test("events client feeds store slices from SSE frames", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  const status = sampleStatus(false);
  let statusFetches = 0;
  let listedOperations: Operation[] = [];
  const api = {
    getStatus: async () => {
      statusFetches += 1;
      return status;
    },
    listOperations: async () => ({ operations: listedOperations }),
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  equal(MockEventSource.instances.length, 1);
  equal(MockEventSource.instances[0].url, "/api/events?session=tok");
  equal(client.transport(), "connecting");

  const es = MockEventSource.instances[0];
  es.emit("open");
  equal(client.transport(), "sse");

  // Snapshot: live operations reconciled, resync bumped, full status refetched.
  es.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("op-1")] }),
  );
  await tick();
  deepEqual(
    store.get("operations").map((op) => op.id),
    ["op-1"],
  );
  equal(store.get("resync"), 1);
  equal(statusFetches, 1);
  deepEqual(store.get("status"), status);

  // Operation event upserts by id.
  const completed = sampleOperation("op-1", "completed");
  es.emit(
    "operation",
    JSON.stringify({
      v: 1,
      operation: completed,
    }),
  );
  es.emit(
    "operation",
    JSON.stringify({ v: 1, operation: sampleOperation("op-2") }),
  );
  await tick();
  const operations = store.get("operations");
  equal(operations.length, 2);
  equal(operations[0].state, "completed");
  equal(operations[1].id, "op-2");

  // Queue events accumulate newest-first, capped.
  for (let i = 0; i < QUEUE_EVENTS_CAP + 10; i++) {
    es.emit("queue", JSON.stringify({ v: 1, job_id: `job-${i}`, state: "sent" }));
  }
  await tick();
  const queueEvents = store.get("queueEvents");
  equal(queueEvents.length, QUEUE_EVENTS_CAP);
  equal(queueEvents[0].job_id, `job-${QUEUE_EVENTS_CAP + 9}`);

  // Status event triggers a full status refetch.
  es.emit("status", JSON.stringify({ v: 1, kind: "locked" }));
  await tick();
  equal(statusFetches, 2);

  // Lag resync: a second snapshot drops stale live work but preserves the
  // terminal record that remains in the daemon's bounded list.
  listedOperations = [completed];
  es.emit("snapshot", JSON.stringify({ v: 1, locked: true, operations: [] }));
  await tick();
  equal(store.get("resync"), 2);
  deepEqual(
    store.get("operations").map((operation) => [operation.id, operation.state]),
    [["op-1", "completed"]],
  );

  client.stop();
  equal(client.transport(), "off");
  es.emit(
    "operation",
    JSON.stringify({ v: 1, operation: sampleOperation("op-3") }),
  );
  await tick();
  equal(store.get("operations").length, 1); // stopped client ignores frames
});

test("session token changes retire stale SSE authorization and reconnect", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  let token: string | null = "old-token";
  let onTokenChange: ((token: string | null) => void) | null = null;
  let unsubscribed = false;
  const client = createEventsClient({
    store,
    api: {
      getStatus: async () => sampleStatus(token === null),
      listOperations: async () => ({ operations: [] as Operation[] }),
    },
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => token,
    sessionTokenChanges: (listener) => {
      onTokenChange = listener;
      return () => {
        unsubscribed = true;
        onTokenChange = null;
      };
    },
    pollIntervalMs: 60_000,
  });

  client.start();
  const oldSource = MockEventSource.instances[0];
  equal(oldSource.url, "/api/events?session=old-token");
  oldSource.emit("open");

  token = null;
  onTokenChange?.(null);
  equal(oldSource.closed, true);
  equal(client.transport(), "poll");
  oldSource.emit(
    "operation",
    JSON.stringify({ v: 1, operation: sampleOperation("stale") }),
  );
  await tick();
  equal(store.get("operations").length, 0);

  token = "new-token";
  onTokenChange?.(token);
  equal(MockEventSource.instances.length, 2);
  equal(MockEventSource.instances[1].url, "/api/events?session=new-token");
  equal(client.transport(), "connecting");

  client.stop();
  equal(MockEventSource.instances[1].closed, true);
  equal(unsubscribed, true);
});

test("snapshot merges terminal history while live SSE records take precedence", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  const live = sampleOperation("op-live");
  const failed = { ...sampleOperation("op-failed", "failed"), error: "boom" };
  const staleDuplicate = {
    ...sampleOperation("op-live", "failed"),
    error: "stale list result",
  };
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: async () => ({
      operations: [staleDuplicate, failed, failed],
    }),
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("open");
  es.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [live] }),
  );
  await tick();

  deepEqual(
    store.get("operations").map((operation) => [operation.id, operation.state]),
    [
      ["op-live", "running"],
      ["op-failed", "failed"],
    ],
  );
  client.stop();
});

test("newer operation events are not overwritten by in-flight history", async () => {
  installDom();
  MockEventSource.instances = [];
  const history = deferred<{ operations: Operation[] }>();
  const store = createCoreStore(BOOT_ROUTE);
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: () => history.promise,
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("op-1")] }),
  );
  const completed = {
    ...sampleOperation("op-1", "completed"),
    updated_at_unix: 20,
  };
  es.emit(
    "operation",
    JSON.stringify({ v: 1, operation: completed }),
  );
  history.resolve({
    operations: [
      { ...sampleOperation("op-1", "failed"), error: "stale" },
      sampleOperation("op-history", "failed"),
    ],
  });
  await tick();

  deepEqual(
    store.get("operations").map((operation) => [operation.id, operation.state]),
    [
      ["op-1", "completed"],
      ["op-history", "failed"],
    ],
  );
  client.stop();
});

test("terminal-history failure leaves snapshot state and SSE transport intact", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  store.set("operations", [sampleOperation("op-known", "failed")]);
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: async (): Promise<{ operations: Operation[] }> => {
      throw new Error("history unavailable");
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("open");
  es.emit(
    "snapshot",
    JSON.stringify({
      v: 1,
      locked: false,
      operations: [sampleOperation("op-live")],
    }),
  );
  await tick();

  equal(client.transport(), "sse");
  equal(store.get("resync"), 1);
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["op-live", "op-known"],
  );
  client.stop();
});

test("stale history is ignored after a newer snapshot and across stop/start", async () => {
  installDom();
  MockEventSource.instances = [];
  const requests: Array<Deferred<{ operations: Operation[] }>> = [];
  const store = createCoreStore(BOOT_ROUTE);
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: () => {
      const request = deferred<{ operations: Operation[] }>();
      requests.push(request);
      return request.promise;
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const firstSource = MockEventSource.instances[0];
  firstSource.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("live-1")] }),
  );
  firstSource.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("live-2")] }),
  );
  equal(requests.length, 2);

  requests[0].resolve({ operations: [sampleOperation("stale-snapshot", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["live-2"],
  );

  requests[1].resolve({ operations: [sampleOperation("current-history", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["live-2", "current-history"],
  );

  firstSource.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("live-3")] }),
  );
  equal(requests.length, 3);
  client.stop();
  client.start();
  const restartedSource = MockEventSource.instances[1];
  restartedSource.emit(
    "snapshot",
    JSON.stringify({ v: 1, locked: false, operations: [sampleOperation("live-4")] }),
  );
  equal(requests.length, 4);

  requests[2].resolve({ operations: [sampleOperation("stale-generation", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["live-4", "current-history"],
  );

  requests[3].resolve({ operations: [sampleOperation("fresh-generation", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["live-4", "fresh-generation"],
  );
  client.stop();
});

test("history reconciliation honors daemon eviction and keeps in-flight SSE transitions", async () => {
  installDom();
  MockEventSource.instances = [];
  const requests: Array<Deferred<{ operations: Operation[] }>> = [];
  const store = createCoreStore(BOOT_ROUTE);
  const prior = Array.from({ length: 60 }, (_, index) =>
    sampleOperation(`old-${index}`, "completed"),
  );
  store.set("operations", prior);
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: () => {
      const request = deferred<{ operations: Operation[] }>();
      requests.push(request);
      return request.promise;
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("snapshot", JSON.stringify({ v: 1, locked: false, operations: [] }));
  equal(requests.length, 1);
  const duringRequest = sampleOperation("during-request", "failed");
  es.emit(
    "operation",
    JSON.stringify({ v: 1, operation: duringRequest }),
  );
  requests[0].resolve({ operations: prior.slice(10).reverse() });
  await tick();

  // The response is bounded to 50, while the transition that arrived after
  // the request began is retained until the next authoritative reconciliation.
  equal(store.get("operations").length, 51);
  equal(store.get("operations").some((operation) => operation.id === "old-0"), false);
  equal(
    store.get("operations").some((operation) => operation.id === duringRequest.id),
    true,
  );

  es.emit("snapshot", JSON.stringify({ v: 1, locked: false, operations: [] }));
  equal(requests.length, 2);
  requests[1].resolve({
    operations: [duringRequest, ...prior.slice(11).reverse()],
  });
  await tick();

  equal(store.get("operations").length, 50);
  equal(store.get("operations").some((operation) => operation.id === "old-10"), false);
  equal(
    store.get("operations").some((operation) => operation.id === duringRequest.id),
    true,
  );
  client.stop();
});

test("history reconciliation accepts opaque listed states and only preserves known live state", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  store.set("operations", [
    sampleOperation("known-live", "cancel_requested"),
    sampleOperation("opaque-stale", "aborted"),
  ]);
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: async () => ({
      operations: [sampleOperation("opaque-listed", "aborted")],
    }),
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit(
    "snapshot",
    JSON.stringify({
      v: 1,
      locked: false,
      operations: [sampleOperation("known-live", "cancel_requested")],
    }),
  );
  await tick();

  deepEqual(
    store.get("operations").map((operation) => [operation.id, operation.state]),
    [
      ["known-live", "cancel_requested"],
      ["opaque-listed", "aborted"],
    ],
  );
  client.stop();
});

test("terminal-history failures retry without degrading SSE", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  let historyCalls = 0;
  let retry: (() => void) | null = null;
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: async () => {
      historyCalls += 1;
      if (historyCalls === 1) throw new Error("temporary history failure");
      return { operations: [sampleOperation("recovered-history", "failed")] };
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
    setTimeoutFn: (callback) => {
      retry = callback;
      return "history-retry";
    },
    clearTimeoutFn: () => {
      retry = null;
    },
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("open");
  es.emit("snapshot", JSON.stringify({ v: 1, locked: false, operations: [] }));
  await tick();
  equal(historyCalls, 1);
  equal(client.transport(), "sse");
  if (!retry) throw new Error("history retry was not scheduled");

  retry();
  await tick();
  equal(historyCalls, 2);
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["recovered-history"],
  );
  equal(client.transport(), "sse");
  client.stop();
});

test("poll fallback invalidates snapshot history still in flight", async () => {
  installDom();
  MockEventSource.instances = [];
  const snapshotHistory = deferred<{ operations: Operation[] }>();
  const pollHistory = deferred<{ operations: Operation[] }>();
  const store = createCoreStore(BOOT_ROUTE);
  let operationsCalls = 0;
  const api = {
    getStatus: async () => sampleStatus(false),
    listOperations: () => {
      operationsCalls += 1;
      return operationsCalls === 1 ? snapshotHistory.promise : pollHistory.promise;
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
    maxConsecutiveErrors: 1,
    pollIntervalMs: 60_000,
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("snapshot", JSON.stringify({ v: 1, locked: false, operations: [] }));
  es.emit("error");
  equal(operationsCalls, 2);

  pollHistory.resolve({ operations: [sampleOperation("poll-current", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["poll-current"],
  );
  snapshotHistory.resolve({ operations: [sampleOperation("snapshot-stale", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["poll-current"],
  );
  client.stop();
});

test("in-flight locked-mode poll cannot overwrite SSE after unlock upgrade", async () => {
  installDom();
  MockEventSource.instances = [];
  const stalePoll = deferred<{ operations: Operation[] }>();
  const upgradePoll = deferred<{ operations: Operation[] }>();
  const store = createCoreStore(BOOT_ROUTE);
  let token: string | null = null;
  let intervalTick: (() => void) | null = null;
  let operationsCalls = 0;
  const api = {
    getStatus: async () => sampleStatus(token === null),
    listOperations: () => {
      operationsCalls += 1;
      if (operationsCalls === 1) {
        return Promise.resolve({ operations: [sampleOperation("poll-initial")] });
      }
      if (operationsCalls === 2) return stalePoll.promise;
      if (operationsCalls === 3) return upgradePoll.promise;
      return Promise.resolve({ operations: [] as Operation[] });
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => token,
    setIntervalFn: (callback) => {
      intervalTick = callback;
      return "poll-interval";
    },
    clearIntervalFn: () => {
      intervalTick = null;
    },
  });
  client.start();
  await tick();
  equal(client.transport(), "poll");
  equal(MockEventSource.instances.length, 0);
  if (!intervalTick) throw new Error("poll interval was not scheduled");

  intervalTick();
  intervalTick();
  equal(operationsCalls, 3);
  token = "fresh-token";
  upgradePoll.resolve({ operations: [sampleOperation("upgrade-poll")] });
  await tick();
  equal(MockEventSource.instances.length, 1);

  const es = MockEventSource.instances[0];
  es.emit("open");
  es.emit(
    "snapshot",
    JSON.stringify({
      v: 1,
      locked: false,
      operations: [sampleOperation("sse-live")],
    }),
  );
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["sse-live"],
  );

  stalePoll.resolve({ operations: [sampleOperation("stale-poll", "failed")] });
  await tick();
  deepEqual(
    store.get("operations").map((operation) => operation.id),
    ["sse-live"],
  );
  client.stop();
});

test("newer same-generation status fetch wins when responses arrive out of order", async () => {
  installDom();
  MockEventSource.instances = [];
  const older = deferred<ReturnType<typeof sampleStatus>>();
  const newer = deferred<ReturnType<typeof sampleStatus>>();
  const statuses = [older, newer];
  const store = createCoreStore(BOOT_ROUTE);
  const api = {
    getStatus: () => statuses.shift()!.promise,
    listOperations: async () => ({ operations: [] as Operation[] }),
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
  });
  client.start();

  const es = MockEventSource.instances[0];
  es.emit("snapshot", JSON.stringify({ v: 1, locked: false, operations: [] }));
  es.emit("status", JSON.stringify({ v: 1, kind: "unlocked" }));
  newer.resolve(sampleStatus(false));
  await tick();
  equal(store.get("status")?.locked, false);

  older.resolve(sampleStatus(true));
  await tick();
  equal(store.get("status")?.locked, false);
  client.stop();
});

test("events client reconnects with backoff, then falls back to passive polling", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  const polls: string[] = [];
  const api = {
    getStatus: async () => {
      polls.push("status");
      return sampleStatus(false);
    },
    listOperations: async () => {
      polls.push("operations");
      return { operations: [sampleOperation("op-poll")] };
    },
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => "tok",
    maxConsecutiveErrors: 3,
    pollIntervalMs: 1000,
    reconnectDelayMs: () => 1,
  });
  client.start();

  // Three consecutive transport errors: two reconnects, then the fallback.
  MockEventSource.instances[0].emit("error");
  await sleep(10);
  equal(MockEventSource.instances.length, 2);
  MockEventSource.instances[1].emit("error");
  await sleep(10);
  equal(MockEventSource.instances.length, 3);
  MockEventSource.instances[2].emit("error");
  await tick();

  equal(client.transport(), "poll");
  deepEqual(polls, ["status", "operations"]);
  await tick();
  deepEqual(
    store.get("operations").map((op) => op.id),
    ["op-poll"],
  );
  deepEqual(store.get("status")?.locked, false);

  client.stop();
});

test("events client starts polling while locked and upgrades to SSE after unlock", async () => {
  installDom();
  MockEventSource.instances = [];
  const store = createCoreStore(BOOT_ROUTE);
  let token: string | null = null;
  const api = {
    getStatus: async () => sampleStatus(token === null),
    listOperations: async () => ({ operations: [] as Operation[] }),
  };
  const client = createEventsClient({
    store,
    api,
    eventSourceFactory: (url) => new MockEventSource(url),
    sessionToken: () => token,
    pollIntervalMs: 5,
  });
  client.start();
  await sleep(20);

  // No session token: straight to the passive poll, no EventSource yet.
  equal(MockEventSource.instances.length, 0);
  equal(client.transport(), "poll");

  // Unlock: a token appears; the next successful poll upgrades to SSE.
  token = "fresh-token";
  await sleep(30);
  equal(MockEventSource.instances.length, 1);
  equal(MockEventSource.instances[0].url, "/api/events?session=fresh-token");
  MockEventSource.instances[0].emit("open");
  equal(client.transport(), "sse");

  client.stop();
});
