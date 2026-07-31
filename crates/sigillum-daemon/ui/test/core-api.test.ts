import { deepEqual, equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  ApiError,
  apiFailure,
  createDaemonApi,
  isApiFailure,
} from "../src/core/api";
import { installDom } from "./dom-fixture";
import { mockFetchJson } from "./core-helpers";

test("api client maps typed methods to paths with plan-1.5 query params", async () => {
  installDom();
  const api = createDaemonApi();
  const paths: string[] = [];
  mockFetchJson((path: string) => {
    paths.push(path);
    if (path.startsWith("/api/queue/jobs")) return { jobs: [] };
    if (path.startsWith("/api/receiving/overview")) return { groups: [] };
    if (path.startsWith("/api/operations/")) return { operation: {} };
    return {};
  });

  await api.listQueueJobs({
    limit: 25,
    offset: 0,
    state: "queued",
    kind: "plan_step_execution",
    chain_id: 1,
    sort: "created",
    order: "asc",
  });
  equal(
    paths[0],
    "/api/queue/jobs?limit=25&offset=0&state=queued&kind=plan_step_execution&chain_id=1&sort=created&order=asc",
  );

  await api.listQueueJobs(); // parameterless keeps the legacy response
  equal(paths[1], "/api/queue/jobs");

  await api.getReceivingOverview({ includeRetired: true });
  equal(paths[2], "/api/receiving/overview?include_retired=true");

  await api.getOperation("op 1");
  equal(paths[3], "/api/operations/op%201");
});

test("api client throws ApiError with the discriminated failure union", async () => {
  installDom();
  const api = createDaemonApi();

  mockFetchJson(() => ({
    code: "vault_locked",
    error: "The vault is locked.",
    action: "unlock",
  }));
  try {
    await api.getTreasuryOverview();
    ok(false, "expected ApiError");
  } catch (error) {
    ok(error instanceof ApiError);
    const failure = apiFailure(error);
    ok(failure);
    equal(failure.code, "vault_locked");
    equal(failure.action, "unlock");
    ok(isApiFailure(error, "vault_locked"));
    ok(!isApiFailure(error, "not_found"));
  }

  mockFetchJson(() => ({
    code: "validation_failed",
    error: "limit exceeds maximum",
    fields: [{ field: "limit", message: "limit exceeds maximum" }],
  }));
  try {
    await api.listQueueJobs({ limit: 99999 });
    ok(false, "expected ApiError");
  } catch (error) {
    const failure = apiFailure(error);
    equal(failure?.code, "validation_failed");
    deepEqual(failure?.fields, [
      { field: "limit", message: "limit exceeds maximum" },
    ]);
  }
});

test("api client maps network failures to the unavailable code", async () => {
  installDom();
  const api = createDaemonApi();
  (globalThis as { fetch?: unknown }).fetch = async () => {
    throw new TypeError("fetch failed");
  };
  try {
    await api.getStatus();
    ok(false, "expected ApiError");
  } catch (error) {
    const failure = apiFailure(error);
    equal(failure?.code, "unavailable");
    equal(failure?.error, "fetch failed");
  }
});
