import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  DaemonHttpError,
  SessionContextChangedError,
  withDaemonHttpStatus,
} from "../src/api/session";
import type {
  LockLatchStorage,
  LockLatchSubscriber,
} from "../src/api/lockUnconfirmed";
import {
  LOCK_UNCONFIRMED_LATCH_KEY,
  createLockUnconfirmedState,
} from "../src/api/lockUnconfirmed";
import type { SessionBoundaryChannel } from "../src/api/sessionBoundary";
import { createSessionCoordinator } from "../src/api/sessionCoordinator";

const TOKEN_A = "a".repeat(64);
const TOKEN_B = "b".repeat(64);

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function memoryStorage(initial: Record<string, string> = {}): LockLatchStorage {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
    removeItem: (key) => {
      values.delete(key);
    },
  };
}

function isolatedBoundaryChannel(): SessionBoundaryChannel {
  let seen: string | null = null;
  let serial = 0;
  return {
    current: () => null,
    seen: () => seen,
    markSeen: (value) => { seen = value; },
    publish: (reason, phase) => JSON.stringify({
      version: 1, id: String(++serial), phase, reason,
    }),
    subscribe: () => () => undefined,
  };
}

function sharedBoundaryBus() {
  let current: string | null = null;
  let eventSerial = 0;
  let endpointSerial = 0;
  const listeners = new Map<number, (value: string) => void>();
  return {
    current: () => current,
    endpoint(initialSeen: string | null = null): SessionBoundaryChannel {
      const endpointId = ++endpointSerial;
      let seen = initialSeen;
      return {
        current: () => current,
        seen: () => seen,
        markSeen: (value) => { seen = value; },
        publish: (reason, phase) => {
          const value = JSON.stringify({
            version: 1, id: String(++eventSerial), phase, reason,
          });
          seen = value;
          current = value;
          listeners.forEach((listener, id) => {
            if (id !== endpointId) listener(value);
          });
          return value;
        },
        subscribe: (listener) => {
          listeners.set(endpointId, listener);
          return () => { listeners.delete(endpointId); };
        },
      };
    },
  };
}

async function captureError(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  return null;
}

function coordinatorHarness(
  request: (
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ) => Promise<any>,
  failClosedLock?: (token: string) => Promise<boolean>,
  lockOptions: {
    storage?: LockLatchStorage | null;
    subscribe?: LockLatchSubscriber;
    boundaryChannel?: SessionBoundaryChannel;
  } = {},
) {
  let token: string | null = TOKEN_A;
  let privacyGeneration = 0;
  let refreshCount = 0;
  let closeCount = 0;
  let forcedCloseCount = 0;
  let queuedRefreshCount = 0;
  let refreshHook = async () => undefined;
  const transitionUi: boolean[] = [];
  const warningStates: Array<boolean | "clear"> = [];
  const warningCapabilities: Array<{ retry: boolean; acknowledge: boolean }> = [];
  const coordinator = createSessionCoordinator({
    privacyGeneration: () => privacyGeneration,
    scrubPrivateWorkspace: () => {
      privacyGeneration += 1;
    },
    resetPrivateState: () => undefined,
    setTransitionUi: (active) => {
      transitionUi.push(active);
    },
    renderTransitionState: () => undefined,
    closeSessionUi: (force) => {
      closeCount += 1;
      if (force) forcedCloseCount += 1;
    },
    showLockUnconfirmed: (canRetry, canAcknowledge) => {
      warningStates.push(canRetry);
      warningCapabilities.push({ retry: canRetry, acknowledge: canAcknowledge });
    },
    clearLockUnconfirmed: () => warningStates.push("clear"),
    isUnlockedUi: () => true,
    markRefreshQueued: () => { queuedRefreshCount += 1; },
    refresh: async () => {
      refreshCount += 1;
      await refreshHook();
    },
    readToken: () => token,
    writeToken: (nextToken) => {
      token = nextToken;
    },
    clearToken: () => {
      token = null;
    },
    failClosedLock,
    lockStorage: lockOptions.storage ?? null,
    lockSubscribe: lockOptions.subscribe,
    boundaryChannel: lockOptions.boundaryChannel || isolatedBoundaryChannel(),
    request,
  });
  return {
    coordinator,
    bumpPrivacy: () => { privacyGeneration += 1; },
    closeCount: () => closeCount,
    forcedCloseCount: () => forcedCloseCount,
    privacyGeneration: () => privacyGeneration,
    queuedRefreshCount: () => queuedRefreshCount,
    refreshCount: () => refreshCount,
    setRefreshHook: (hook: () => Promise<void>) => {
      refreshHook = hook;
    },
    setToken: (nextToken: string | null) => {
      token = nextToken;
    },
    token: () => token,
    transitionUi,
    warningCapabilities,
    warningStates,
  };
}

test("running peers invalidate on pending switch and Lock before late reads paint", async () => {
  for (const operation of ["switch", "lock"] as const) {
    const bus = sharedBoundaryBus();
    const heldRead = deferred<any>();
    const observer = coordinatorHarness(async (_method, path) =>
      path === "/api/held-private-read" ? heldRead.promise : {}, undefined,
    { boundaryChannel: bus.endpoint() });
    const initiator = coordinatorHarness(async (_method, path) =>
      path === "/api/compartment/switch"
        ? withDaemonHttpStatus({ status: "switched", compartment_id: 2,
            session_token: TOKEN_B }, 200)
        : withDaemonHttpStatus({ status: "locked" }, 200), undefined,
    { boundaryChannel: bus.endpoint() });
    observer.coordinator.initializeLockUnconfirmedState();
    initiator.coordinator.initializeLockUnconfirmedState();
    const staleRead = observer.coordinator.api("GET", "/api/held-private-read");

    const path = operation === "switch" ? "/api/compartment/switch" : "/api/lock";
    const context = operation === "switch"
      ? await initiator.coordinator.beginTransition(path, "Switching")
      : await initiator.coordinator.beginEmergencyLockTransition(path, "Locking");
    equal(observer.token(), null);
    equal(observer.forcedCloseCount(), 1);
    equal(observer.privacyGeneration(), 1);
    equal(initiator.token(), TOKEN_A);
    equal(initiator.coordinator.isContextCurrent(context), true);

    heldRead.resolve(withDaemonHttpStatus({ status: "private" }, 200));
    ok(await captureError(staleRead) instanceof SessionContextChangedError);
    await initiator.coordinator.api(
      "POST", path, operation === "switch" ? { id: 2 } : undefined, context,
    );
    equal(initiator.token(), operation === "switch" ? TOKEN_B : null);
    await initiator.coordinator.endTransition(context);
  }
});

test("a cloned tab starting in the post-commit response window honors pending", async () => {
  const bus = sharedBoundaryBus();
  const heldResponse = deferred<any>();
  const initiator = coordinatorHarness(async () => heldResponse.promise, undefined,
    { boundaryChannel: bus.endpoint() });
  initiator.coordinator.initializeLockUnconfirmedState();
  const context = await initiator.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const request = initiator.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  );
  const pending = bus.current();
  ok(pending);
  equal(JSON.parse(pending).phase, "pending");

  // Browser tab cloning copies sessionStorage, including both token T and the
  // already-seen marker. Pending must still invalidate synchronously at start.
  const clone = coordinatorHarness(async () => {
    throw new Error("startup must invalidate before private loading");
  }, undefined, { boundaryChannel: bus.endpoint(pending) });
  clone.coordinator.initializeLockUnconfirmedState();
  equal(clone.token(), null);
  equal(clone.forcedCloseCount(), 1);
  equal(clone.queuedRefreshCount(), 1);

  heldResponse.resolve(withDaemonHttpStatus({
    status: "switched", compartment_id: 2, session_token: TOKEN_B,
  }, 200));
  await request;
  equal(initiator.token(), TOKEN_B);
  equal(initiator.forcedCloseCount(), 0);
  const settled = bus.current();
  ok(settled);
  equal(JSON.parse(settled).phase, "settled");
  await initiator.coordinator.endTransition(context);
});

test("only a matching settled startup marker avoids false invalidation", () => {
  const bus = sharedBoundaryBus();
  const publisher = bus.endpoint();
  const settled = publisher.publish("/api/lock", "settled");
  ok(settled);

  const current = coordinatorHarness(async () => ({}), undefined,
    { boundaryChannel: bus.endpoint(settled) });
  current.setToken(TOKEN_B);
  current.coordinator.initializeLockUnconfirmedState();
  equal(current.token(), TOKEN_B);
  equal(current.forcedCloseCount(), 0);

  const staleMarker = JSON.stringify({
    version: 1, id: "older", phase: "settled", reason: "/api/lock",
  });
  const stale = coordinatorHarness(async () => ({}), undefined,
    { boundaryChannel: bus.endpoint(staleMarker) });
  stale.coordinator.initializeLockUnconfirmedState();
  equal(stale.token(), null);
  equal(stale.forcedCloseCount(), 1);

  const malformedSettled = '{"phase":"settled"}';
  let malformedSeen: string | null = malformedSettled;
  const malformed = coordinatorHarness(async () => ({}), undefined, {
    boundaryChannel: {
      current: () => malformedSettled,
      seen: () => malformedSeen,
      markSeen: (value) => { malformedSeen = value; },
      publish: () => null,
      subscribe: () => () => undefined,
    },
  });
  malformed.coordinator.initializeLockUnconfirmedState();
  equal(malformed.token(), null);
  equal(malformed.forcedCloseCount(), 1);
});

test("reset, restore, and revoke publish pending then only settle validated success", async () => {
  const cases = [
    ["/api/setup/reset", { confirmation: "RESET SIGILLUM" },
      { status: "reset", archived_to: null }],
    ["/api/backup/restore", { passphrase: "long enough", snapshot_hex: "00" },
      { status: "restored", requires_reauth: true, summary: { file_count: 1 } }],
    ["/api/session/revoke", undefined,
      { status: "revoked", requires_reauth: true }],
  ] as const;
  for (const [path, body, payload] of cases) {
    const bus = sharedBoundaryBus();
    const observer = coordinatorHarness(async () => ({}), undefined,
      { boundaryChannel: bus.endpoint() });
    const initiator = coordinatorHarness(async () =>
      withDaemonHttpStatus({ ...payload }, 200), undefined,
    { boundaryChannel: bus.endpoint() });
    observer.coordinator.initializeLockUnconfirmedState();
    initiator.coordinator.initializeLockUnconfirmedState();
    const context = await initiator.coordinator.beginTransition(path, "Boundary");
    equal(JSON.parse(bus.current() || "{}").phase, "pending");
    equal(observer.token(), null);
    equal(initiator.coordinator.isContextCurrent(context), true);
    await initiator.coordinator.api("POST", path, body, context);
    equal(initiator.token(), null);
    equal(JSON.parse(bus.current() || "{}").phase, "settled");
    await initiator.coordinator.endTransition(context);
  }
});

test("boundary endpoints reject requests without an owning transition", async () => {
  let requestCount = 0;
  const harness = coordinatorHarness(async () => {
    requestCount += 1;
    return withDaemonHttpStatus({ status: "unexpected" }, 200);
  });
  for (const path of [
    "/api/compartment/switch", "/api/lock", "/api/setup/reset",
    "/api/backup/restore", "/api/session/revoke",
  ]) {
    ok(await captureError(harness.coordinator.api("POST", path, {}))
      instanceof SessionContextChangedError);
  }
  equal(requestCount, 0);

  const unavailable = coordinatorHarness(async () => {
    requestCount += 1;
    return {};
  }, undefined, { boundaryChannel: {
    current: () => null, seen: () => null, markSeen: () => undefined,
    publish: () => null, subscribe: () => () => undefined,
  } });
  ok(await captureError(unavailable.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  )) instanceof SessionContextChangedError);
  equal(requestCount, 0);
});

test("lost generic boundary outcomes fallback-Lock and cannot repaint pending state", async () => {
  for (const path of [
    "/api/setup/reset", "/api/backup/restore", "/api/session/revoke",
  ]) {
    const response = deferred<any>();
    const fallbackTokens: string[] = [];
    const harness = coordinatorHarness(async () => response.promise,
      async (token) => { fallbackTokens.push(token); return true; });
    const context = await harness.coordinator.beginTransition(path, "Boundary");
    const request = harness.coordinator.api("POST", path, {}, context);
    response.reject(new Error("response lost after daemon commit"));
    const error = await captureError(request);
    equal(fallbackTokens[0], TOKEN_A);
    equal(harness.token(), null);
    equal(
      (error as Error).message.includes("fallback Lock confirmed"),
      true,
      String((error as Error).message),
    );
    await harness.coordinator.endTransition(context);
    equal(harness.refreshCount(), 1); // Safe only after fallback Lock proof.
  }
});

test("confirmed switch fallback settles before a fresh startup token is used", async () => {
  const bus = sharedBoundaryBus();
  const initiator = coordinatorHarness(async () =>
    withDaemonHttpStatus({ status: "malformed" }, 200),
  async () => true, { boundaryChannel: bus.endpoint() });
  initiator.coordinator.initializeLockUnconfirmedState();
  const context = await initiator.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const error = await captureError(initiator.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));
  equal((error as Error).message.includes("fallback Lock confirmed"), true);
  const settled = bus.current();
  ok(settled);
  equal(JSON.parse(settled).phase, "settled");

  const fresh = coordinatorHarness(async () => ({}), undefined,
    { boundaryChannel: bus.endpoint(settled) });
  fresh.setToken(TOKEN_B);
  fresh.coordinator.initializeLockUnconfirmedState();
  equal(fresh.token(), TOKEN_B);
  equal(fresh.forcedCloseCount(), 0);
});

test("strict establishment contracts adopt tokens only for five exact paths", async () => {
  const cases = [
    ["/api/unlock", { passphrase: "secret" }, {
      status: "unlocked", method: "passphrase", unlocked_compartments: [{ id: 1 }],
      active_compartment_id: 1, session_token: TOKEN_B,
    }],
    ["/api/fido2/unlock", { credential: "ok" }, {
      status: "unlocked", method: "fido2", unlocked_compartments: [{ id: 2 }],
      active_compartment_id: 2, session_token: TOKEN_B,
    }],
    ["/api/biometric/unlock", { credential: "ok" }, {
      status: "unlocked", method: "biometric", unlocked_compartments: [{ id: 3 }],
      active_compartment_id: 3, session_token: TOKEN_B,
    }],
    ["/api/compartment/init", { id: 4, label: "cold" }, {
      status: "initialized", compartment_id: 4, compartment_label: "cold",
      session_token: TOKEN_B,
    }],
    ["/api/fido2/setup", { compartments: [{ id: 0 }, { id: 1 }] }, {
      status: "setup_complete", unlocked: true, total_keys: 1, compartments: 2,
      session_token: TOKEN_B,
    }],
  ] as const;
  for (const [path, body, payload] of cases) {
    const harness = coordinatorHarness(async () =>
      withDaemonHttpStatus({ ...payload }, 200));
    harness.setToken(null);
    await harness.coordinator.api("POST", path, body);
    equal(harness.token(), TOKEN_B, path);
  }
});

test("malformed establishment success and 5xx fail closed", async () => {
  const outcomes: Array<() => Promise<any>> = [
    async () => withDaemonHttpStatus({ status: "unlocked", method: "passphrase",
      session_token: TOKEN_B, unlocked_compartments: [{ id: 1 }],
      active_compartment_id: 1 }, 500),
    async () => withDaemonHttpStatus({ status: "unlocked", method: "wrong",
      session_token: TOKEN_B, unlocked_compartments: [{ id: 1 }],
      active_compartment_id: 1 }, 200),
    async () => { throw new DaemonHttpError(200); },
    async () => { throw new Error("transport lost"); },
  ];
  for (const request of outcomes) {
    const fallbackTokens: string[] = [];
    const harness = coordinatorHarness(request, async (token) => {
      fallbackTokens.push(token);
      return true;
    });
    const error = await captureError(harness.coordinator.api(
      "POST", "/api/unlock", { passphrase: "secret" },
    ));
    equal(fallbackTokens[0], TOKEN_A);
    equal(harness.token(), null);
    equal(
      (error as Error).message.includes("fallback Lock confirmed"), true,
      String((error as Error).message),
    );
  }
});

test("no-token establishment ambiguity persists without offering unsafe retry", async () => {
  const storage = memoryStorage();
  const harness = coordinatorHarness(async () => {
    throw new DaemonHttpError(500);
  }, undefined, { storage });
  harness.setToken(null);
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "secret" },
  ));
  equal((error as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(harness.warningCapabilities.at(-1)?.retry, false);
  equal(harness.warningCapabilities.at(-1)?.acknowledge, true);
  equal(await harness.coordinator.retryUnconfirmedLock(), false);
});

test("establishment 401 clears only its current predecessor and returns rejection", async () => {
  const rejection = withDaemonHttpStatus({ error: "unauthorized" }, 401);
  const harness = coordinatorHarness(async () => rejection);
  const result = await harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "wrong" },
  );
  equal(result, rejection);
  equal(harness.token(), null);
  equal(harness.forcedCloseCount(), 1);
  equal(harness.queuedRefreshCount(), 1);
});

test("valid session establishment aborts predecessor reads before publishing T2", async () => {
  let harness!: ReturnType<typeof coordinatorHarness>;
  let tokenObservedAtAbort: string | null | undefined;
  harness = coordinatorHarness((_method, path, _body, signal) => {
    if (path === "/api/unlock") {
      return Promise.resolve(withDaemonHttpStatus({
        status: "unlocked",
        method: "passphrase",
        session_token: TOKEN_B,
        unlocked_compartments: [{ id: 1 }],
        active_compartment_id: 1,
      }, 200));
    }
    if (path === "/api/held-old-read") {
      return new Promise((_resolve, reject) => {
        signal?.addEventListener("abort", () => {
          tokenObservedAtAbort = harness.token();
          reject(new Error("aborted"));
        }, { once: true });
      });
    }
    return Promise.resolve({});
  });
  harness.setToken(null);

  const oldRead = harness.coordinator.api("GET", "/api/held-old-read");
  const unlocked = await harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "secret" },
  );

  equal(unlocked.session_token, TOKEN_B);
  equal(tokenObservedAtAbort, null);
  equal(harness.token(), TOKEN_B);
  ok(await captureError(oldRead) instanceof SessionContextChangedError);
  equal(harness.queuedRefreshCount(), 1);
});

test("abort-time token replacement makes establishment obsolete without fallback Lock", async () => {
  let harness!: ReturnType<typeof coordinatorHarness>;
  let fallbackLockCount = 0;
  harness = coordinatorHarness((_method, path, _body, signal) => {
    if (path === "/api/unlock") {
      return Promise.resolve(withDaemonHttpStatus({
        status: "unlocked",
        method: "passphrase",
        session_token: TOKEN_B,
        unlocked_compartments: [{ id: 1 }],
        active_compartment_id: 1,
      }, 200));
    }
    if (path === "/api/held-old-read") {
      return new Promise((_resolve, reject) => {
        signal?.addEventListener("abort", () => {
          harness.setToken(TOKEN_A);
          reject(new Error("aborted"));
        }, { once: true });
      });
    }
    return Promise.resolve({});
  }, async () => {
    fallbackLockCount += 1;
    return true;
  });
  harness.setToken(null);

  const oldRead = harness.coordinator.api("GET", "/api/held-old-read");
  const unlockError = await captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "secret" },
  ));

  ok(unlockError instanceof SessionContextChangedError);
  ok(await captureError(oldRead) instanceof SessionContextChangedError);
  equal(harness.token(), TOKEN_A);
  equal(fallbackLockCount, 0);
  equal(harness.forcedCloseCount(), 0);
});

test("capability and unknown token-bearing replies are never adopted", async () => {
  for (const path of ["/api/session/capability", "/api/unknown"]) {
    const harness = coordinatorHarness(async () => withDaemonHttpStatus({
      status: "ok", session_token: TOKEN_B,
    }, 200));
    await harness.coordinator.api("POST", path, {});
    equal(harness.token(), TOKEN_A);
  }
});

test("stale privacy and boundary generations reject late establishment adoption", async () => {
  const valid = () => withDaemonHttpStatus({
    status: "unlocked", method: "passphrase", session_token: TOKEN_B,
    unlocked_compartments: [{ id: 1 }], active_compartment_id: 1,
  }, 200);
  const privacyResponse = deferred<any>();
  const privacy = coordinatorHarness(async () => privacyResponse.promise);
  privacy.setToken(null);
  const stalePrivacy = privacy.coordinator.api(
    "POST", "/api/unlock", { passphrase: "secret" },
  );
  privacy.bumpPrivacy();
  privacyResponse.resolve(valid());
  ok(await captureError(stalePrivacy) instanceof SessionContextChangedError);
  equal(privacy.token(), null);

  const bus = sharedBoundaryBus();
  const boundaryResponse = deferred<any>();
  const boundary = coordinatorHarness(async () => boundaryResponse.promise,
    undefined, { boundaryChannel: bus.endpoint() });
  boundary.setToken(null);
  boundary.coordinator.initializeLockUnconfirmedState();
  const staleBoundary = boundary.coordinator.api(
    "POST", "/api/unlock", { passphrase: "secret" },
  );
  bus.endpoint().publish("/api/lock", "pending");
  boundaryResponse.resolve(valid());
  ok(await captureError(staleBoundary) instanceof SessionContextChangedError);
  equal(boundary.token(), null);
});

test("a valid late unlock cannot adopt while another unlock owns containment", async () => {
  const firstResponse = deferred<any>();
  const secondResponse = deferred<any>();
  let requestCount = 0;
  const storage = memoryStorage();
  const harness = coordinatorHarness(async () => {
    requestCount += 1;
    return requestCount === 1 ? firstResponse.promise : secondResponse.promise;
  }, undefined, { storage });
  harness.setToken(null);
  const first = captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "first" },
  ));
  const second = captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "second" },
  ));
  firstResponse.resolve(withDaemonHttpStatus({ status: "malformed" }, 200));
  await new Promise((resolve) => setTimeout(resolve, 0));
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");

  secondResponse.resolve(withDaemonHttpStatus({
    status: "unlocked", method: "passphrase", session_token: TOKEN_B,
    unlocked_compartments: [{ id: 1 }], active_compartment_id: 1,
  }, 200));
  const firstError = await first;
  const secondError = await second;
  equal((firstError as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  ok(secondError instanceof SessionContextChangedError);
  equal(harness.token(), null);
  equal(harness.warningCapabilities.length, 1);
});

test("restart acknowledgment invalidates pre-latch establishment responses", async () => {
  const firstResponse = deferred<any>();
  const secondResponse = deferred<any>();
  let requestCount = 0;
  const storage = memoryStorage();
  const harness = coordinatorHarness(async () =>
    ++requestCount === 1 ? firstResponse.promise : secondResponse.promise,
  undefined, { storage });
  harness.setToken(null);
  const first = captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "ambiguous" },
  ));
  const late = captureError(harness.coordinator.api(
    "POST", "/api/unlock", { passphrase: "late" },
  ));
  firstResponse.resolve(withDaemonHttpStatus({ status: "malformed" }, 200));
  equal((await first as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  await harness.coordinator.acknowledgeDaemonRestart("I STOPPED SIGILLUM");
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);

  secondResponse.resolve(withDaemonHttpStatus({
    status: "unlocked", method: "passphrase", session_token: TOKEN_B,
    unlocked_compartments: [{ id: 1 }], active_compartment_id: 1,
  }, 200));
  ok(await late instanceof SessionContextChangedError);
  equal(harness.token(), null);
});

test("successful retry settles the original pending boundary marker", async () => {
  const bus = sharedBoundaryBus();
  let lockAttempts = 0;
  const harness = coordinatorHarness(async () => {
    throw new Error("response lost");
  }, async () => ++lockAttempts === 2,
  { boundaryChannel: bus.endpoint() });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));
  equal((error as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(JSON.parse(bus.current() || "{}").phase, "pending");
  equal(await harness.coordinator.retryUnconfirmedLock(), true);
  equal(JSON.parse(bus.current() || "{}").phase, "settled");
});

test("overlapping containment attempts never share or replace latch authority", async () => {
  const attempt = deferred<boolean>();
  let token: string | null = TOKEN_A;
  let lockCalls = 0;
  const state = createLockUnconfirmedState({
    lockWithToken: async () => { lockCalls += 1; return attempt.promise; },
    clearGeneralToken: () => { token = null; },
    closeSessionUi: () => undefined,
    clearWarning: () => undefined,
    showWarning: () => undefined,
    storage: memoryStorage(),
  });
  const first = state.contain(TOKEN_A, () => true);
  equal(token, null);
  equal(await state.contain(TOKEN_A, () => true), "obsolete");
  equal(lockCalls, 1);
  attempt.resolve(true);
  equal(await first, "confirmed");
});

test("malformed ordinary 401 and daemon 423 suppress the stale action", async () => {
  const unauthorized = coordinatorHarness(async () => {
    throw new DaemonHttpError(401);
  });
  ok(await captureError(unauthorized.coordinator.api("GET", "/api/private"))
    instanceof SessionContextChangedError);
  equal(unauthorized.token(), null);
  equal(unauthorized.forcedCloseCount(), 1);
  equal(unauthorized.queuedRefreshCount(), 1);

  for (const request of [
    async () => withDaemonHttpStatus({ error: "locked" }, 423),
    async () => { throw new DaemonHttpError(423); },
  ]) {
    const harness = coordinatorHarness(request);
    ok(await captureError(harness.coordinator.api("GET", "/api/private"))
      instanceof SessionContextChangedError);
    equal(harness.token(), null);
    equal(harness.forcedCloseCount(), 1);
    equal(harness.privacyGeneration(), 1);
    equal(harness.queuedRefreshCount() > 0, true);
  }
});

test("ordinary switch drains old mutation and blocks new requests", async () => {
  const oldMutation = deferred<any>();
  const calls: string[] = [];
  const harness = coordinatorHarness(async (_method, path) => {
    calls.push(path);
    if (path === "/api/old-mutation") return oldMutation.promise;
    if (path === "/api/compartment/switch") {
      return withDaemonHttpStatus({
        status: "switched",
        compartment_id: 1,
        session_token: TOKEN_B,
      }, 200);
    }
    return {};
  });

  const staleMutation = harness.coordinator.api("POST", "/api/old-mutation");
  const begin = harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  let transitionStarted = false;
  void begin.then(() => {
    transitionStarted = true;
  });
  await Promise.resolve();

  equal(transitionStarted, false);
  const blockedError = await captureError(
    harness.coordinator.api("GET", "/api/new-read"),
  );
  ok(blockedError instanceof SessionContextChangedError);
  equal(calls.includes("/api/new-read"), false);

  oldMutation.resolve({ status: "ok" });
  const staleError = await captureError(staleMutation);
  ok(staleError instanceof SessionContextChangedError);
  const context = await begin;
  equal(transitionStarted, true);
  await harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 1 },
    context,
  );
  await harness.coordinator.endTransition(context);
  equal(harness.refreshCount(), 1);
});

test("compartment switch may atomically rotate the current session token", async () => {
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness(async (_method, path) => {
    equal(path, "/api/compartment/switch");
    return withDaemonHttpStatus({
      status: "switched",
      compartment_id: 2,
      compartment_label: "secure",
      session_token: TOKEN_B,
    }, 200);
  });

  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  const response = await harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 2 },
    context,
  );
  equal(response.session_token, TOKEN_B);
  equal(harness.token(), TOKEN_B);
  await harness.coordinator.endTransition(context);
});

test("every malformed current-owner switch outcome fail-closes with captured T", async () => {
  const invalidPayloads = [
    withDaemonHttpStatus({ error: "rejected" }, 200),
    withDaemonHttpStatus(
      { status: "ok", compartment_id: 2, session_token: TOKEN_B }, 200,
    ),
    withDaemonHttpStatus({ status: "switched", compartment_id: 2 }, 200),
    withDaemonHttpStatus(
      { status: "switched", compartment_id: 2, session_token: TOKEN_A }, 200,
    ),
    withDaemonHttpStatus(
      { status: "switched", compartment_id: 3, session_token: TOKEN_B }, 200,
    ),
    withDaemonHttpStatus({
      status: "switched", compartment_id: 2, session_token: "not-a-token",
    }, 200),
    { status: "switched", compartment_id: 2, session_token: TOKEN_B },
    withDaemonHttpStatus({
      status: "switched", compartment_id: 2, session_token: TOKEN_B,
    }, 401),
  ];
  for (const payload of invalidPayloads) {
    const fallbackTokens: string[] = [];
    const harness = coordinatorHarness(
      async () => payload,
      async (token) => {
        fallbackTokens.push(token);
        return true;
      },
    );
    const context = await harness.coordinator.beginTransition(
      "/api/compartment/switch",
      "Switching",
    );
    const error = await captureError(harness.coordinator.api(
      "POST", "/api/compartment/switch", { id: 2 }, context,
    ));
    equal(fallbackTokens[0], TOKEN_A);
    equal(harness.token(), null);
    equal((error as Error).message.includes("fallback Lock confirmed"), true);
    await harness.coordinator.endTransition(context);
  }
});

test("stored-token mismatch is contained instead of accepted as switch success", async () => {
  const fallbackTokens: string[] = [];
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness(async () => {
    harness.setToken(TOKEN_B);
    return withDaemonHttpStatus({
      status: "switched",
      compartment_id: 2,
      session_token: TOKEN_B,
    }, 200);
  }, async (token) => {
    fallbackTokens.push(token);
    return true;
  });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));
  equal(fallbackTokens[0], TOKEN_A);
  equal(harness.token(), null);
});

test("immediate switch transport rejection fail-closes before reconciliation", async () => {
  const events: string[] = [];
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness(async () => {
    throw new Error("connection reset");
  }, async (token) => {
    events.push(`lock:${token}:browser=${harness.token()}`);
    return true;
  });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));
  equal(events[0], `lock:${TOKEN_A}:browser=${TOKEN_A}`);
  equal(harness.token(), null);
});

test("an obsolete switch timeout cannot clear a newer session token", async () => {
  const fallbackTokens: string[] = [];
  const harness = coordinatorHarness((_method, path, _body, signal) => {
    if (path !== "/api/compartment/switch") return Promise.resolve({});
    return new Promise((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(new Error("aborted")), {
        once: true,
      });
    });
  }, async (token) => {
    fallbackTokens.push(token);
    return true;
  });
  const switchContext = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  switchContext.requestTimeoutMs = 5;
  const staleSwitch = harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 2 },
    switchContext,
  );
  const lockContext = await harness.coordinator.beginEmergencyLockTransition(
    "/api/lock",
    "Locking",
  );
  harness.setToken(TOKEN_B);

  const timeout = await captureError(staleSwitch);
  ok(timeout instanceof SessionContextChangedError);
  equal(harness.token(), TOKEN_B);
  equal(harness.closeCount(), 0);
  equal(fallbackTokens.length, 0);
  await harness.coordinator.endTransition(lockContext);
});

test("a current switch timeout Locks with the captured token before clearing it", async () => {
  const events: string[] = [];
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness((_method, _path, _body, signal) =>
    new Promise((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(new Error("aborted")), {
        once: true,
      });
    }), async (token) => {
    events.push(`lock:${token}:browser=${harness.token()}`);
    return true;
  });

  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  context.requestTimeoutMs = 5;
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));

  equal(events[0], `lock:${TOKEN_A}:browser=${TOKEN_A}`);
  equal(harness.token(), null);
  equal(harness.closeCount(), 1);
  equal((error as Error).message.includes("fallback Lock confirmed"), true);
});

test("Lock accepts only structural locked success or HTTP 423", async () => {
  for (const payload of [
    withDaemonHttpStatus({ status: "locked" }, 200),
    withDaemonHttpStatus({ error: "already locked" }, 423),
  ]) {
    const fallbackTokens: string[] = [];
    const harness = coordinatorHarness(async () => payload, async (token) => {
      fallbackTokens.push(token);
      return true;
    });
    const context = await harness.coordinator.beginEmergencyLockTransition(
      "/api/lock", "Locking",
    );
    const response = await harness.coordinator.api(
      "POST", "/api/lock", undefined, context,
    );
    equal(response.status, "locked");
    equal(fallbackTokens.length, 0);
    await harness.coordinator.endTransition(context);
  }
});

test("resolved malformed or unauthorized Lock outcomes are contained with T", async () => {
  const outcomes = [
    withDaemonHttpStatus({ status: "ok" }, 200),
    withDaemonHttpStatus({ status: "locked", error: "rejected" }, 200),
    withDaemonHttpStatus({ error: "unauthorized" }, 401),
  ];
  for (const payload of outcomes) {
    const fallbackTokens: string[] = [];
    const harness = coordinatorHarness(async () => payload, async (token) => {
      fallbackTokens.push(token);
      return true;
    });
    const context = await harness.coordinator.beginEmergencyLockTransition(
      "/api/lock", "Locking",
    );
    const error = await captureError(harness.coordinator.api(
      "POST", "/api/lock", undefined, context,
    ));
    equal(fallbackTokens[0], TOKEN_A);
    equal(harness.token(), null);
    equal((error as Error).message.includes("fallback Lock confirmed"), true);
  }
});

test("an unresolved Lock transport outcome enters persistent containment", async () => {
  const storage = memoryStorage();
  const fallbackTokens: string[] = [];
  const harness = coordinatorHarness(async () => {
    throw new Error("connection reset");
  }, async (token) => {
    fallbackTokens.push(token);
    return false;
  }, { storage });
  const context = await harness.coordinator.beginEmergencyLockTransition(
    "/api/lock", "Locking",
  );
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/lock", undefined, context,
  ));
  equal((error as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(fallbackTokens[0], TOKEN_A);
  equal(harness.token(), null);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(harness.warningStates.at(-1), true);
});

test("a synchronous fallback failure still isolates T and blocks the session", async () => {
  const storage = memoryStorage();
  const harness = coordinatorHarness(async () => {
    throw new Error("connection reset");
  }, () => {
    throw new Error("fallback could not start");
  }, { storage });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));
  equal((error as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(harness.token(), null);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(harness.warningStates.at(-1), true);
});

test("unconfirmed fallback persists, blocks reconciliation, and retries only isolated T", async () => {
  const storage = memoryStorage();
  const lockTokens: string[] = [];
  let lockAttempt = 0;
  const requestCalls: string[] = [];
  const harness = coordinatorHarness(async (_method, path) => {
    requestCalls.push(path);
    throw new Error("transport lost");
  }, async (token) => {
    lockTokens.push(token);
    lockAttempt += 1;
    return lockAttempt === 2;
  }, { storage });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const error = await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));

  equal((error as Error).message.startsWith("LOCK NOT CONFIRMED"), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(harness.token(), null);
  equal(harness.warningStates.at(-1), true);
  await harness.coordinator.endTransition(context);
  equal(harness.refreshCount(), 0);
  const blocked = await captureError(
    harness.coordinator.api("GET", "/api/status"),
  );
  ok(blocked instanceof SessionContextChangedError);
  equal(requestCalls.includes("/api/status"), false);

  await harness.coordinator.retryUnconfirmedLock();
  equal(lockTokens.length, 2);
  equal(lockTokens.every((token) => token === TOKEN_A), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);
  equal(harness.warningStates.at(-1), "clear");
  equal(harness.refreshCount(), 1);
  equal(harness.transitionUi.at(-1), false);
});

test("a failed retry remains retryable with the same isolated token", async () => {
  const storage = memoryStorage();
  const lockTokens: string[] = [];
  const harness = coordinatorHarness(async () => {
    throw new Error("transport lost");
  }, async (token) => {
    lockTokens.push(token);
    return lockTokens.length === 3;
  }, { storage });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  await captureError(harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  ));

  equal(await harness.coordinator.retryUnconfirmedLock(), false);
  equal(harness.warningStates.at(-1), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(await harness.coordinator.retryUnconfirmedLock(), true);
  equal(lockTokens.length, 3);
  equal(lockTokens.every((token) => token === TOKEN_A), true);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);
});

test("late same-switch token adoption during fallback is cleared, not promoted", async () => {
  const fallbackStarted = deferred<void>();
  const fallbackResult = deferred<boolean>();
  const storage = memoryStorage();
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness((_method, _path, _body, signal) =>
    new Promise((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(new Error("aborted")), {
        once: true,
      });
    }), async () => {
    fallbackStarted.resolve();
    return fallbackResult.promise;
  }, { storage });
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  context.requestTimeoutMs = 5;
  const request = harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  );
  await fallbackStarted.promise;
  equal(harness.token(), null);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  harness.setToken(TOKEN_B);
  fallbackResult.resolve(true);
  await captureError(request);
  equal(harness.token(), null);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);
});

test("startup restores the non-secret latch and requires exact daemon-stop acknowledgment", async () => {
  const storage = memoryStorage({ [LOCK_UNCONFIRMED_LATCH_KEY]: "1" });
  const harness = coordinatorHarness(async () => ({}), undefined, { storage });

  equal(harness.coordinator.initializeLockUnconfirmedState(), true);
  equal(harness.token(), null);
  equal(harness.warningStates.at(-1), false);
  ok(await captureError(harness.coordinator.api("GET", "/api/status"))
    instanceof SessionContextChangedError);
  await harness.coordinator.acknowledgeDaemonRestart("I stopped Sigillum");
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");
  equal(harness.refreshCount(), 0);

  await harness.coordinator.acknowledgeDaemonRestart("I STOPPED SIGILLUM");
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);
  equal(harness.warningStates.at(-1), "clear");
  equal(harness.refreshCount(), 1);
  equal(harness.transitionUi.at(-1), false);
});

test("cross-tab latch events immediately block and later reconcile this tab", async () => {
  const storage = memoryStorage();
  let storageListener: ((latched: boolean) => void) | null = null;
  const harness = coordinatorHarness(async () => ({}), undefined, {
    storage,
    subscribe: (listener) => {
      storageListener = listener;
      return () => undefined;
    },
  });
  equal(harness.coordinator.initializeLockUnconfirmedState(), false);

  storage.setItem(LOCK_UNCONFIRMED_LATCH_KEY, "1");
  storageListener?.(true);
  equal(harness.token(), null);
  equal(harness.warningStates.at(-1), false);
  ok(await captureError(harness.coordinator.api("GET", "/api/status"))
    instanceof SessionContextChangedError);

  storage.removeItem(LOCK_UNCONFIRMED_LATCH_KEY);
  storageListener?.(false);
  await new Promise((resolve) => setTimeout(resolve, 0));
  equal(harness.warningStates.at(-1), "clear");
  equal(harness.refreshCount(), 1);
  equal(harness.transitionUi.at(-1), false);
});

test("external latch clear supersedes a held fallback without repainting warning", async () => {
  const storage = memoryStorage();
  const fallbackStarted = deferred<void>();
  const fallbackResult = deferred<boolean>();
  let storageListener: ((latched: boolean) => void) | null = null;
  const harness = coordinatorHarness(async () => {
    throw new Error("transport lost");
  }, async () => {
    fallbackStarted.resolve();
    return fallbackResult.promise;
  }, {
    storage,
    subscribe: (listener) => {
      storageListener = listener;
      return () => undefined;
    },
  });
  harness.coordinator.initializeLockUnconfirmedState();
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch", "Switching",
  );
  const request = harness.coordinator.api(
    "POST", "/api/compartment/switch", { id: 2 }, context,
  );
  await fallbackStarted.promise;
  equal(harness.token(), null);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), "1");

  storage.removeItem(LOCK_UNCONFIRMED_LATCH_KEY);
  storageListener?.(false);
  await new Promise((resolve) => setTimeout(resolve, 0));
  fallbackResult.resolve(false);
  const error = await captureError(request);
  ok(error instanceof SessionContextChangedError);
  equal(storage.getItem(LOCK_UNCONFIRMED_LATCH_KEY), null);
  equal(harness.warningStates.at(-1), "clear");
  equal(harness.warningStates.includes(true), false);
  equal(harness.refreshCount(), 1);
  equal(harness.transitionUi.at(-1), false);
});

test("reconciliation owns the coordinator until its refresh settles", async () => {
  const heldStatus = deferred<any>();
  const heldFinalStatus = deferred<any>();
  const finalStatusStarted = deferred<void>();
  const calls: string[] = [];
  let harness!: ReturnType<typeof coordinatorHarness>;
  harness = coordinatorHarness(async (_method, path) => {
    calls.push(path);
    if (path === "/api/compartment/switch") {
      return withDaemonHttpStatus({
        status: "switched",
        compartment_id: 2,
        session_token: TOKEN_B,
      }, 200);
    }
    if (path === "/api/status") return heldStatus.promise;
    if (path === "/api/final-status") {
      finalStatusStarted.resolve();
      return heldFinalStatus.promise;
    }
    return {};
  });
  harness.setRefreshHook(async () => {
    await harness.coordinator.runRefreshRequests(() =>
      harness.coordinator.api("GET", "/api/status"),
    );
    await harness.coordinator.runRefreshRequests(() =>
      harness.coordinator.api("GET", "/api/final-status"),
    );
  });

  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  await harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 2 },
    context,
  );
  const reconciliation = harness.coordinator.endTransition(context);
  equal(calls.includes("/api/status"), true);

  const blocked = await captureError(
    harness.coordinator.api("GET", "/api/ordinary-read"),
  );
  ok(blocked instanceof SessionContextChangedError);
  equal(calls.includes("/api/ordinary-read"), false);

  heldStatus.resolve({ status: "ok" });
  await finalStatusStarted.promise;
  equal(calls.includes("/api/final-status"), true);
  const stillBlocked = await captureError(
    harness.coordinator.api("GET", "/api/ordinary-read-after-await"),
  );
  ok(stillBlocked instanceof SessionContextChangedError);
  equal(calls.includes("/api/ordinary-read-after-await"), false);
  heldFinalStatus.resolve({ status: "ok" });
  await reconciliation;
  equal(harness.transitionUi.at(-1), false);
});

test("ordinary transition aborts a held old read without deadlock", async () => {
  const calls: string[] = [];
  const harness = coordinatorHarness((_method, path, _body, signal) => {
    calls.push(path);
    if (path !== "/api/held-read") return Promise.resolve({});
    return new Promise((_resolve, reject) => {
      signal?.addEventListener("abort", () => reject(new Error("aborted")), {
        once: true,
      });
    });
  });

  const heldRead = harness.coordinator.api("GET", "/api/held-read");
  const context = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  const readError = await captureError(heldRead);
  ok(readError instanceof SessionContextChangedError);
  equal(calls.length, 1);
  await harness.coordinator.endTransition(context);
});

test("emergency Lock supersedes a held switch and rejects its late response", async () => {
  const heldSwitch = deferred<any>();
  const harness = coordinatorHarness(async (_method, path) => {
    if (path === "/api/compartment/switch") return heldSwitch.promise;
    if (path === "/api/lock") {
      return withDaemonHttpStatus({ status: "locked" }, 200);
    }
    return {};
  });

  const switchContext = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  const switchRequest = harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 1 },
    switchContext,
  );
  const lockContext = await harness.coordinator.beginEmergencyLockTransition(
    "/api/lock",
    "Locking",
  );
  const lockResponse = await harness.coordinator.api(
    "POST",
    "/api/lock",
    undefined,
    lockContext,
  );
  equal(lockResponse.status, "locked");
  await harness.coordinator.endTransition(lockContext);

  heldSwitch.resolve({ status: "switched", compartment_id: 1 });
  const switchError = await captureError(switchRequest);
  ok(switchError instanceof SessionContextChangedError);
  await harness.coordinator.endTransition(switchContext);
  equal(harness.refreshCount(), 1);
  equal(harness.transitionUi.at(-1), false);
});

test("same-path transition loser cannot tear down the winning owner", async () => {
  const harness = coordinatorHarness(async (_method, path, body: any) =>
    path === "/api/compartment/switch"
      ? withDaemonHttpStatus({
          status: "switched",
          compartment_id: body.id,
          session_token: TOKEN_B,
        }, 200)
      : { status: "ok" },
  );
  const winner = await harness.coordinator.beginTransition(
    "/api/compartment/switch",
    "Switching",
  );
  const loserError = await captureError(
    harness.coordinator.beginTransition(
      "/api/compartment/switch",
      "Switching again",
    ),
  );
  ok(loserError instanceof SessionContextChangedError);
  await harness.coordinator.endTransition(null);

  const response = await harness.coordinator.api(
    "POST",
    "/api/compartment/switch",
    { id: 1 },
    winner,
  );
  equal(response.status, "switched");
  await harness.coordinator.endTransition(winner);
  equal(harness.refreshCount(), 1);
});
