import { equal } from "node:assert/strict";
import { test } from "node:test";

import { createRefreshRunner } from "../src/state/refresh";

interface Deferred {
  promise: Promise<void>;
  resolve: () => void;
}

function deferred(): Deferred {
  let resolve!: () => void;
  const promise = new Promise<void>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

test("a request in the completion microtask starts a new refresh drain", async () => {
  const firstCycle = deferred();
  const secondCycleStarted = deferred();
  let cycleCount = 0;
  const runner = createRefreshRunner(() => {
    cycleCount += 1;
    if (cycleCount === 1) return firstCycle.promise;
    secondCycleStarted.resolve();
    return Promise.resolve();
  }, () => undefined);

  const firstRun = runner.run();
  void firstCycle.promise.then(() => runner.run());
  firstCycle.resolve();

  await firstRun;
  await secondCycleStarted.promise;
  equal(cycleCount, 2);
});

test("requests queued during a cycle are coalesced into one follow-up", async () => {
  const firstCycle = deferred();
  let cycleCount = 0;
  const runner = createRefreshRunner(async () => {
    cycleCount += 1;
    if (cycleCount === 1) await firstCycle.promise;
  }, () => undefined);

  const refresh = runner.run();
  runner.queue();
  runner.queue();
  firstCycle.resolve();

  await refresh;
  equal(cycleCount, 2);
});
