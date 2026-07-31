import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import { createStore } from "../src/core/store";
import { tick } from "./core-helpers";

test("store notifies subscribers with next/prev after a microtask batch", async () => {
  const store = createStore({ count: 0, flag: false });
  const seen: Array<[number, number]> = [];
  store.subscribe("count", (next, prev) => seen.push([next, prev]));

  store.set("count", 1);
  store.set("count", 2); // batched: one notification with the latest value
  store.set("flag", true); // different slice: must not reach count listeners
  await tick();

  deepEqual(seen, [[2, 0]]);
  equal(store.get("count"), 2);
  equal(store.get("flag"), true);
});

test("store skips notifications for unchanged references (structural sharing)", async () => {
  const store = createStore({ items: [1, 2], other: "x" });
  let fired = 0;
  store.subscribe("items", () => {
    fired += 1;
  });

  store.set("items", store.get("items")); // same reference
  store.update("items", (prev) => prev); // idiomatic no-change
  await tick();
  equal(fired, 0);

  store.update("items", (prev) => [...prev, 3]); // new reference
  await tick();
  equal(fired, 1);
  deepEqual(store.get("items"), [1, 2, 3]);
});

test("store unsubscribe stops delivery and listeners stay slice-isolated", async () => {
  const store = createStore({ a: 0, b: 0 });
  const seenA: number[] = [];
  const seenB: number[] = [];
  const offA = store.subscribe("a", (next) => seenA.push(next));
  store.subscribe("b", (next) => seenB.push(next));

  store.set("a", 1);
  await tick(); // delivered: the listener was subscribed at flush time
  offA(); // pending AND future deliveries stop here
  store.set("a", 2);
  store.set("b", 9);
  await tick();

  deepEqual(seenA, [1]);
  deepEqual(seenB, [9]);
});

test("store unsubscribe before the flush drops the pending notification", async () => {
  const store = createStore({ a: 0 });
  const seen: number[] = [];
  const off = store.subscribe("a", (next) => seen.push(next));
  store.set("a", 1);
  off(); // removed before the microtask flush: nothing is delivered
  await tick();
  deepEqual(seen, []);
});
