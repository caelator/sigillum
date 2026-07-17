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
  const api = {
    getStatus: async () => {
      statusFetches += 1;
      return status;
    },
    listOperations: async () => ({ operations: [] as Operation[] }),
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

  // Snapshot: operations replaced, resync bumped, full status refetched.
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
  es.emit(
    "operation",
    JSON.stringify({
      v: 1,
      operation: sampleOperation("op-1", "completed"),
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

  // Lag resync: a second snapshot bumps resync again.
  es.emit("snapshot", JSON.stringify({ v: 1, locked: true, operations: [] }));
  await tick();
  equal(store.get("resync"), 2);
  deepEqual(store.get("operations"), []);

  client.stop();
  equal(client.transport(), "off");
  es.emit(
    "operation",
    JSON.stringify({ v: 1, operation: sampleOperation("op-3") }),
  );
  await tick();
  equal(store.get("operations").length, 0); // stopped client ignores frames
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
