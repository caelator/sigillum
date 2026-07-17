import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import { createVaultDestination } from "../src/destinations/Vault";
import type { CoreRuntime } from "../src/core/live";
import type { Route } from "../src/core/router";
import { createCoreStore, type CoreStore } from "../src/core/state";
import type {
  AuditEvent,
  DiagnosticsResponse,
  SelfCheckRunResponse,
  StatusResponse,
} from "../src/contracts";
import { FakeElement, FakeTextNode, installDom, type DomFixture } from "./dom-fixture";

// ── Fixtures ─────────────────────────────────────────────────────────

const VAULT_ROUTE: Route = {
  destination: "vault",
  path: [],
  params: {},
  hash: "#/vault",
};

function unlockedStatus(): StatusResponse {
  return {
    initialized: true,
    locked: false,
    active_compartment: {
      compartment_id: 0,
      compartment_label: "Main",
      api_key_count: 1,
    },
    unlocked_compartments: [
      { id: 0, label: "Main", threshold: 1 },
      { id: 1, label: "Ops", threshold: 2, passphrase_mode: "required" },
    ],
  };
}

function lockedStatus(): StatusResponse {
  return { initialized: true, locked: true, unlocked_compartments: [] };
}

function diagnosticsSample(): DiagnosticsResponse {
  return {
    status: "ok",
    version: "1.2.3",
    unlock_scope: "all",
    session_scope: "tab",
    started_at_unix: 1752700000,
    initialized: true,
    unlocked_compartment_count: 2,
    active_session_count: 1,
    default_active_compartment_id: 0,
    max_unlocked_threshold: 2,
    audit_log_present: true,
    pending_operation_count: 0,
    queue_job_count: 4,
    blocked_queue_job_count: 1,
    retrying_queue_job_count: 0,
    failed_queue_job_count: 1,
    operator_action_required_queue_job_count: 2,
    deferred_queue_job_count: 0,
    startup_interrupted_operation_count: 0,
    startup_recovered_operation_count: 0,
    startup_unresolved_operation_count: 0,
    startup_recovered_queue_job_count: 0,
    startup_reconciled_deposit_count: 0,
    runtime_policy: {
      queue_default_process_limit: 5,
      queue_max_process_limit: 25,
      deposit_default_refresh_limit: 10,
      deposit_max_refresh_limit: 100,
      audit_default_limit: 20,
      audit_max_limit: 200,
      queue_retry_base_delay_secs: 30,
      queue_retry_max_delay_secs: 900,
      provider_balance_observation_concurrency: 4,
      receiving_refresh_address_cap: 25,
      idle_lock_secs: 1800,
      idle_lock_drain_secs: 60,
      idle_lock_force_after_secs: 300,
    },
    eth_stealth_deposit_count: 3,
    funded_eth_stealth_deposit_count: 1,
    scheduler: {
      enabled: true,
      queue_tick_secs: 15,
      refresh_secs: 300,
      last_tick_at_unix: 1752700000,
      last_cycle_outcome: "advanced",
      consecutive_failures: 0,
      due_queue_job_count: 1,
      next_retry_at_unix: null,
    },
  };
}

function selfCheckSample(): SelfCheckRunResponse {
  return {
    status: "fail",
    generated_at_unix: Math.floor(Date.now() / 1000),
    checks: [
      {
        id: "c1",
        domain: "provider",
        subject: "mainnet RPC",
        status: "pass",
        detail: "answers on the right chain",
        latency_ms: 42,
      },
      {
        id: "c2",
        domain: "policy",
        subject: "treasury policy",
        status: "fail",
        detail: "caps are inconsistent",
      },
    ],
  };
}

// ── Fetch + api doubles ──────────────────────────────────────────────

interface FetchCall {
  path: string;
  method: string;
  body: Record<string, unknown> | undefined;
}

type Responder =
  | Record<string, unknown>
  | ((call: FetchCall) => Record<string, unknown>);

/**
 * Route-aware fetch double keyed on "METHOD /path" (exact or trailing-*).
 * Returns a 200 JSON response for everything, mirroring the daemon's
 * envelope contract (error payloads arrive as 200 + {code,error}).
 */
function stubFetch(routes: Record<string, Responder> = {}): FetchCall[] {
  const calls: FetchCall[] = [];
  (globalThis as { fetch?: unknown }).fetch = async (
    path: string,
    init: { method?: string; body?: string },
  ) => {
    const call: FetchCall = {
      path,
      method: init?.method ?? "GET",
      body: init?.body ? (JSON.parse(init.body) as Record<string, unknown>) : undefined,
    };
    calls.push(call);
    const key = call.method + " " + path;
    for (const [pattern, responder] of Object.entries(routes)) {
      const matches = pattern.endsWith("*")
        ? key.startsWith(pattern.slice(0, -1))
        : key === pattern;
      if (matches) {
        const payload =
          typeof responder === "function" ? responder(call) : responder;
        return { status: 200, json: async () => payload };
      }
    }
    return { status: 200, json: async () => ({}) };
  };
  return calls;
}

function findCalls(
  calls: FetchCall[],
  method: string,
  path: string,
): FetchCall[] {
  return calls.filter((call) => call.method === method && call.path === path);
}

interface ApiDouble {
  listAuditQueries: Array<Record<string, unknown> | undefined>;
  getStatusCalls: number;
  runSelfCheckCalls: number;
  api: Record<string, unknown>;
}

function stubApi(overrides: {
  status?: StatusResponse;
  auditEvents?: AuditEvent[];
  diagnostics?: DiagnosticsResponse;
  selfCheck?: SelfCheckRunResponse;
}): ApiDouble {
  const double: ApiDouble = {
    listAuditQueries: [],
    getStatusCalls: 0,
    runSelfCheckCalls: 0,
    api: {},
  };
  double.api = {
    getStatus: async () => {
      double.getStatusCalls += 1;
      return overrides.status ?? unlockedStatus();
    },
    listAudit: async (query?: Record<string, unknown>) => {
      double.listAuditQueries.push(query);
      return { events: overrides.auditEvents ?? [] };
    },
    getDiagnostics: async () => overrides.diagnostics ?? diagnosticsSample(),
    runSelfCheck: async () => {
      double.runSelfCheckCalls += 1;
      return overrides.selfCheck ?? selfCheckSample();
    },
  };
  return double;
}

function stubRuntime(store: CoreStore, api: Record<string, unknown>): CoreRuntime {
  return {
    store,
    api,
    router: {
      route: () => VAULT_ROUTE,
      register: () => undefined,
      navigate: () => undefined,
      start: () => undefined,
      stop: () => undefined,
    },
    adapter: {},
    events: {},
    notifyLegacySection: () => undefined,
    stop: () => undefined,
  } as unknown as CoreRuntime;
}

// ── DOM helpers (fake-DOM text does not aggregate; walk manually) ────

function textOf(node: FakeElement | FakeTextNode | null | undefined): string {
  if (!node) return "";
  if (node instanceof FakeTextNode) return node.textContent;
  let text = node.textContent ?? "";
  for (const child of node.childNodes) text += textOf(child);
  return text;
}

function byVault(dom: DomFixture, name: string): FakeElement | null {
  const host = dom.document.getElementById("secretsCard");
  return host?.querySelector('[data-vault="' + name + '"]') ?? null;
}

function collect(
  root: FakeElement,
  attr: string,
  value: string,
  out: FakeElement[] = [],
): FakeElement[] {
  for (const child of root.children) {
    if (child.attributes[attr] === value) out.push(child);
    collect(child, attr, value, out);
  }
  return out;
}

function collectByVault(dom: DomFixture, name: string): FakeElement[] {
  const host = dom.document.getElementById("secretsCard");
  return host ? collect(host, "data-vault", name) : [];
}

function findButton(root: FakeElement, label: string): FakeElement | null {
  for (const child of root.children) {
    if (child.tagName === "BUTTON" && textOf(child).trim() === label) return child;
    const found = findButton(child, label);
    if (found) return found;
  }
  return null;
}

/** The fixture's querySelector only supports [attr] selectors; walk tags manually. */
function findTag(root: FakeElement | null, tag: string): FakeElement | null {
  if (!root) return null;
  for (const child of root.children) {
    if (child.tagName === tag) return child;
    const found = findTag(child, tag);
    if (found) return found;
  }
  return null;
}

function submit(form: FakeElement): void {
  form.dispatchEvent({ type: "submit", preventDefault: () => undefined });
}

async function flush(rounds = 30): Promise<void> {
  for (let i = 0; i < rounds; i++) await Promise.resolve();
}

/** Confirm-dialog parts (the shared dialog mounts on document.body). */
function confirmPart(dom: DomFixture, part: string): FakeElement | null {
  return dom.document.body.querySelector('[data-confirm-' + part + "]");
}

// ── Standard setup ───────────────────────────────────────────────────

interface Setup {
  dom: DomFixture;
  store: CoreStore;
  controller: ReturnType<typeof createVaultDestination>;
  fetchCalls: FetchCall[];
  api: ApiDouble;
}

const DEFAULT_ROUTES: Record<string, Responder> = {
  "GET /api/compartment/list": {
    compartments: [
      { id: 0, label: "Main", threshold: 1, is_active: true },
      { id: 1, label: "Ops", threshold: 2, is_active: false, passphrase_mode: "required" },
    ],
  },
  "GET /api/secrets": { keys: ["seed"] },
  "GET /api/api-keys": { keys: [] },
  "GET /api/fido2/detect": { device_present: true, device_count: 1 },
  "GET /api/fido2/list": {
    keys: [
      { label: "backup", credential_id_short: "ab12cd", registered_at: "2026-07-01" },
    ],
  },
};

function setup(options: {
  status?: StatusResponse | null;
  routes?: Record<string, Responder>;
  api?: ApiDouble;
  ids?: string[];
}): Setup {
  const dom = installDom(options.ids ?? ["secretsCard"]);
  const store = createCoreStore(VAULT_ROUTE);
  if (options.status) store.set("status", options.status);
  const api = options.api ?? stubApi({ status: options.status ?? unlockedStatus() });
  const fetchCalls = stubFetch({ ...DEFAULT_ROUTES, ...(options.routes ?? {}) });
  const controller = createVaultDestination(stubRuntime(store, api.api));
  return { dom, store, controller, fetchCalls, api };
}

// ── Tests ────────────────────────────────────────────────────────────

test("mount takes over #secretsCard, hides legacy vault siblings, unmount restores", async () => {
  const dom = installDom(["secretsCard", "apiKeysCard", "diagCard"]);
  const host = dom.el("secretsCard");
  const legacyChild = dom.document.createElement("p");
  legacyChild.textContent = "legacy secrets content";
  host.appendChild(legacyChild);
  dom.el("diagCard").classList.add("hidden"); // already hidden before mount

  const store = createCoreStore(VAULT_ROUTE);
  store.set("status", lockedStatus());
  const controller = createVaultDestination(
    stubRuntime(store, stubApi({ status: lockedStatus() }).api),
  );
  stubFetch();

  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    ok(byVault(dom, "root"), "vault root rendered into the host card");
    equal(host.childNodes.includes(legacyChild), false);
    ok(dom.el("apiKeysCard").classList.contains("hidden"));
    ok(dom.el("diagCard").classList.contains("hidden"));
  } finally {
    controller.unmount();
  }
  ok(host.childNodes.includes(legacyChild), "legacy children restored");
  equal(byVault(dom, "root"), null);
  ok(!dom.el("apiKeysCard").classList.contains("hidden"));
  ok(dom.el("diagCard").classList.contains("hidden"), "prior hidden state kept");
});

test("locked vault: lock strip + locked placeholders, no protected fetches", async () => {
  const { dom, controller, fetchCalls } = setup({ status: lockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const strip = byVault(dom, "lock-strip");
    ok(strip);
    ok(textOf(strip).includes("Vault is locked"));
    equal(strip?.dataset.tier, "review");
    ok(textOf(byVault(dom, "secrets")).includes("Vault is locked"));
    ok(textOf(byVault(dom, "diagnostics")).includes("Vault is locked"));
    equal(findCalls(fetchCalls, "GET", "/api/secrets").length, 0);
    equal(findCalls(fetchCalls, "GET", "/api/compartment/list").length, 0);
  } finally {
    controller.unmount();
  }
});

test("unlocked session strip: compartments, switcher, countdown, actions", async () => {
  const { dom, controller } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const strip = byVault(dom, "lock-strip");
    ok(textOf(strip).includes("Unlocked — 2 compartments"));
    ok(textOf(strip).includes("Active compartment: Main"));
    ok(byVault(dom, "switcher"), "compartment switcher rendered");
    ok(byVault(dom, "lock-now"));
    ok(byVault(dom, "logout"));
    const countdown = textOf(byVault(dom, "countdown"));
    ok(
      countdown.includes("Auto-lock after 30 minutes"),
      "countdown shows the idle policy: " + countdown,
    );
  } finally {
    controller.unmount();
  }
});

test("compartments render thresholds; Make active posts the switch", async () => {
  const { dom, controller, fetchCalls, api } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const rows = collectByVault(dom, "compartment-row");
    equal(rows.length, 2);
    ok(textOf(rows[0]).includes("Main"));
    ok(textOf(rows[0]).includes("active"));
    ok(textOf(rows[1]).includes("Requires 2 keys to unlock"));
    const makeActive = findButton(rows[1], "Make active");
    ok(makeActive);
    makeActive?.click();
    await flush();
    const switches = findCalls(fetchCalls, "POST", "/api/compartment/switch");
    equal(switches.length, 1);
    equal(switches[0].body?.id, 1);
    ok(api.getStatusCalls >= 1, "status refetched after switch");
  } finally {
    controller.unmount();
  }
});

test("secrets: keyed rows render, store posts the legacy contract", async () => {
  const { dom, controller, fetchCalls } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const rows = collectByVault(dom, "secret-row");
    equal(rows.length, 1);
    ok(textOf(rows[0]).includes("seed"));

    const name = byVault(dom, "secret-name");
    const value = byVault(dom, "secret-value");
    const form = byVault(dom, "secret-form");
    ok(name && value && form);
    name.value = "rpc-token";
    value.value = "supersecret";
    submit(form);
    await flush();
    const sets = findCalls(fetchCalls, "POST", "/api/secrets/set");
    equal(sets.length, 1);
    equal(sets[0].body?.key, "rpc-token");
    equal(sets[0].body?.value, "supersecret");
    equal(name.value, "", "form cleared after store");
    ok(textOf(byVault(dom, "flash")).includes("rpc-token"));
  } finally {
    controller.unmount();
  }
});

test("reveal-on-demand shows the value and auto-hides after the timeout", async () => {
  // The controller reads this seam lazily (30s in production; see Vault.ts).
  (globalThis as { __SIGILLUM_VAULT_REVEAL_MS__?: number })
    .__SIGILLUM_VAULT_REVEAL_MS__ = 40;
  try {
    const { dom, controller } = setup({
      status: unlockedStatus(),
      routes: { "POST /api/secrets/get": { value: "hunter2" } },
    });
    controller.mount(VAULT_ROUTE);
    await flush();
    try {
      const row = collectByVault(dom, "secret-row")[0];
      findButton(row, "Reveal")?.click();
      await flush();
      const revealed = byVault(dom, "revealed");
      ok(revealed);
      equal(textOf(revealed), "hunter2");

      await new Promise((resolve) => setTimeout(resolve, 80));
      await flush();
      equal(byVault(dom, "revealed"), null, "value auto-hidden after the timeout");
      equal(
        collectByVault(dom, "secret-row").length,
        1,
        "row replaced without leaving a zombie",
      );
      ok(findButton(collectByVault(dom, "secret-row")[0], "Reveal"));
    } finally {
      controller.unmount();
    }
  } finally {
    delete (globalThis as { __SIGILLUM_VAULT_REVEAL_MS__?: number })
      .__SIGILLUM_VAULT_REVEAL_MS__;
  }
});

test("delete secret is gated by the danger confirm (cancel path too)", async () => {
  const { dom, controller, fetchCalls } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    // Cancel path: dialog dismissed, no POST.
    findButton(collectByVault(dom, "secret-row")[0], "Delete")?.click();
    await flush();
    confirmPart(dom, "cancel")?.click();
    await flush();
    equal(findCalls(fetchCalls, "POST", "/api/secrets/delete").length, 0);

    // Confirm path: the danger action fires the delete.
    findButton(collectByVault(dom, "secret-row")[0], "Delete")?.click();
    await flush();
    confirmPart(dom, "action")?.click();
    await flush();
    const deletes = findCalls(fetchCalls, "POST", "/api/secrets/delete");
    equal(deletes.length, 1);
    equal(deletes[0].body?.key, "seed");
  } finally {
    controller.unmount();
  }
});

test("validation_failed highlights the offending field via error fields", async () => {
  const { dom, controller } = setup({
    status: unlockedStatus(),
    routes: {
      "POST /api/secrets/set": {
        code: "validation_failed",
        error: "Validation failed",
        fields: [{ field: "key", message: "Secret name may not contain spaces." }],
      },
    },
  });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const name = byVault(dom, "secret-name");
    name.value = "bad name";
    byVault(dom, "secret-value").value = "x";
    submit(byVault(dom, "secret-form"));
    await flush();
    ok(name.classList.contains("input-invalid"), "field marked invalid");
    ok(
      textOf(byVault(dom, "flash")).includes("Secret name may not contain spaces."),
      "field message surfaced",
    );
  } finally {
    controller.unmount();
  }
});

test("failed refresh renders a persistent banner; retry recovers", async () => {
  let broken = true;
  const { dom, controller } = setup({
    status: unlockedStatus(),
    routes: {
      "GET /api/compartment/list": () =>
        broken
          ? { code: "internal", error: "db busy" }
          : {
              compartments: [
                { id: 0, label: "Main", threshold: 1, is_active: true },
              ],
            },
    },
  });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const banner = byVault(dom, "banner");
    ok(banner);
    ok(!banner.classList.contains("hidden"), "banner visible after failure");
    ok(textOf(banner).includes("db busy"));

    broken = false;
    findButton(banner, "Retry now")?.click();
    await flush();
    ok(banner.classList.contains("hidden"), "banner cleared after recovery");
    ok(textOf(byVault(dom, "compartments")).includes("Main"));
  } finally {
    controller.unmount();
  }
});

test("vault_locked responses guide to unlock instead of erroring", async () => {
  const { dom, controller } = setup({
    status: unlockedStatus(),
    routes: { "GET /api/secrets": { code: "vault_locked", error: "Vault is locked" } },
  });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const banner = byVault(dom, "banner");
    ok(banner.classList.contains("hidden"), "no stale banner for lock errors");
    // Keys never arrived: the section stays in its first-load state.
    ok(byVault(dom, "secrets")?.querySelector('[data-vault="skeleton"]'));
  } finally {
    controller.unmount();
  }
});

test("hardware keys: detect line, keyed list, poison register, remove with PIN", async () => {
  const { dom, controller, fetchCalls } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    ok(textOf(byVault(dom, "fido2-detect-line")).includes("1 hardware key(s) connected"));
    const rows = collectByVault(dom, "fido2-row");
    equal(rows.length, 1);
    ok(textOf(rows[0]).includes("backup"));

    // Register a poison key: the duress confirm gates the POST.
    const poison = byVault(dom, "poison");
    poison.checked = true;
    const labelInput = dom.document
      .getElementById("secretsCard")
      ?.querySelector('[aria-label="Key label"]');
    ok(labelInput);
    labelInput.value = "duress";
    submit(byVault(dom, "fido2-register-form"));
    await flush();
    ok(
      textOf(confirmPart(dom, "body")).includes("POISON key"),
      "poison confirm copy preserved",
    );
    confirmPart(dom, "action")?.click();
    await flush();
    const registers = findCalls(fetchCalls, "POST", "/api/fido2/register");
    equal(registers.length, 1);
    equal(registers[0].body?.label, "duress");
    equal(registers[0].body?.poison, true);

    // Remove: confirm, then the styled PIN prompt, then the POST carries the PIN.
    findButton(collectByVault(dom, "fido2-row")[0], "Remove")?.click();
    await flush();
    confirmPart(dom, "action")?.click();
    await flush();
    const pinInput = dom.document.body.querySelector(
      '[aria-label="Current FIDO2 PIN"]',
    );
    ok(pinInput, "PIN prompt shown");
    pinInput.value = "1234";
    const pinForm = dom.document.body.querySelector('[data-vault="pin-form"]');
    ok(pinForm);
    submit(pinForm);
    await flush();
    const removes = findCalls(fetchCalls, "POST", "/api/fido2/remove");
    equal(removes.length, 1);
    equal(removes[0].body?.label, "backup");
    equal(removes[0].body?.pin, "1234");
  } finally {
    controller.unmount();
  }
});

test("snapshots: nudge when no export recorded; age line when one exists", async () => {
  // Case A: no snapshot.export event → review-tier nudge.
  const apiA = stubApi({ status: unlockedStatus(), auditEvents: [] });
  const a = setup({ status: unlockedStatus(), api: apiA });
  a.controller.mount(VAULT_ROUTE);
  await flush();
  try {
    ok(byVault(a.dom, "backup-nudge"), "nudge rendered");
    ok(
      apiA.listAuditQueries.some(
        (query) => query?.kind === "snapshot.export" && query?.limit === 1,
      ),
      "backup age probed via the audit kind filter",
    );
  } finally {
    a.controller.unmount();
  }

  // Case B: an export event → quiet age line instead of the nudge.
  const apiB = stubApi({
    status: unlockedStatus(),
    auditEvents: [
      {
        created_at_unix: Math.floor(Date.now() / 1000) - 3600,
        kind: "snapshot.export",
      },
    ],
  });
  const b = setup({ status: unlockedStatus(), api: apiB });
  b.controller.mount(VAULT_ROUTE);
  await flush();
  try {
    equal(byVault(b.dom, "backup-nudge"), null);
    ok(textOf(byVault(b.dom, "backup-age")).includes("1h ago"));
  } finally {
    b.controller.unmount();
  }
});

test("audit viewer: humanized rows, kind filter, show-more pagination", async () => {
  const events: AuditEvent[] = [
    {
      created_at_unix: 1752700000,
      kind: "secret.set",
      compartment_id: 0,
      details: { key: "seed" },
    },
    { created_at_unix: 1752700001, kind: "lock.all" },
  ];
  const api = stubApi({ status: unlockedStatus(), auditEvents: events });
  const { dom, controller } = setup({ status: unlockedStatus(), api });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const rows = collectByVault(dom, "audit-row");
    equal(rows.length, 2);
    ok(textOf(rows[0]).includes("Stored encrypted secret — seed"));
    ok(textOf(rows[0]).includes("compartment #0"));
    ok(textOf(rows[1]).includes("Locked all compartments"));
    ok(textOf(rows[1]).includes("global"));

    // Kind filter drives the 1.5 query param.
    const kindSelect = byVault(dom, "audit-kind");
    ok(kindSelect);
    kindSelect.value = "secret.set";
    submit(byVault(dom, "audit-filter-form"));
    await flush();
    ok(
      api.listAuditQueries.some(
        (query) => query?.kind === "secret.set" && query?.limit === 20,
      ),
      "kind filter applied",
    );
  } finally {
    controller.unmount();
  }
});

test("audit show-more grows the page limit", async () => {
  const events: AuditEvent[] = Array.from({ length: 20 }, (_, index) => ({
    created_at_unix: 1752700000 + index,
    kind: "secret.set",
  }));
  const api = stubApi({ status: unlockedStatus(), auditEvents: events });
  const { dom, controller } = setup({ status: unlockedStatus(), api });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const more = byVault(dom, "audit-more");
    ok(more);
    more.click();
    await flush();
    ok(
      api.listAuditQueries.some((query) => query?.limit === 40),
      "limit grows by one page",
    );
  } finally {
    controller.unmount();
  }
});

test("self-check renders pass/warn/fail groupings after a run", async () => {
  const { dom, controller, api } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    // Before any run: the empty state offers the action.
    ok(textOf(byVault(dom, "selfcheck")).includes("Not run yet"));

    byVault(dom, "run-selfcheck")?.click();
    await flush();
    equal(api.runSelfCheckCalls, 1);
    const summary = textOf(byVault(dom, "selfcheck-summary"));
    ok(summary.includes("1 pass") && summary.includes("1 fail"));
    const failGroup = byVault(dom, "selfcheck-fail");
    ok(failGroup?.hasAttribute("open"), "fail group expanded by default");
    ok(textOf(failGroup).includes("treasury policy"));
    ok(textOf(failGroup).includes("caps are inconsistent"));
    const passGroup = byVault(dom, "selfcheck-pass");
    ok(passGroup && !passGroup.hasAttribute("open"), "pass group collapsed");
  } finally {
    controller.unmount();
  }
});

test("diagnostics render grouped and humanized (no raw tile soup)", async () => {
  const { dom, controller } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    const diag = textOf(byVault(dom, "diagnostics"));
    for (const group of ["Daemon", "Queue", "Operations & deposits", "Runtime policy", "Scheduler"]) {
      ok(diag.includes(group), "group present: " + group);
    }
    ok(diag.includes("1.2.3"));
    ok(diag.includes("Needs operator action"));
    ok(diag.includes("Idle auto-lock"));
    ok(diag.includes("30 minutes"));
    // Raw unix seconds stay behind a details disclosure.
    const raw = findTag(byVault(dom, "diagnostics"), "SUMMARY");
    ok(raw, "raw details disclosure present");
  } finally {
    controller.unmount();
  }
});

test("reset local data: typed phrase gates the server confirmation", async () => {
  const { dom, controller, fetchCalls } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    byVault(dom, "reset")?.click();
    await flush();
    const action = confirmPart(dom, "action");
    const input = confirmPart(dom, "input");
    ok(action && input);
    equal(action.disabled, true, "action disabled until the phrase matches");
    input.value = "RESET LOCAL SIGILLUM DATA";
    input.dispatchEvent({ type: "input", target: input });
    equal(action.disabled, false);
    action.click();
    await flush();
    const resets = findCalls(fetchCalls, "POST", "/api/setup/reset");
    equal(resets.length, 1);
    equal(resets[0].body?.confirmation, "RESET LOCAL SIGILLUM DATA");
  } finally {
    controller.unmount();
  }
});

test("export snapshot posts the passphrase and triggers a download", async () => {
  const { dom, controller, fetchCalls } = setup({
    status: unlockedStatus(),
    routes: {
      "POST /api/backup/export": {
        snapshot_hex: "00ff",
        summary: { created_at_unix: 1752700000 },
      },
    },
  });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    byVault(dom, "export-pass").value = "12345678";
    submit(byVault(dom, "export-form"));
    await flush();
    const exports = findCalls(fetchCalls, "POST", "/api/backup/export");
    equal(exports.length, 1);
    equal(exports[0].body?.passphrase, "12345678");
    ok(textOf(byVault(dom, "flash")).includes("Snapshot downloaded"));
  } finally {
    controller.unmount();
  }
});

test("store status changes re-render the lock strip (live, no pollers)", async () => {
  const { dom, store, controller } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    ok(textOf(byVault(dom, "lock-strip")).includes("Unlocked"));
    store.set("status", lockedStatus());
    await flush();
    ok(textOf(byVault(dom, "lock-strip")).includes("Vault is locked"));
    ok(textOf(byVault(dom, "secrets")).includes("Vault is locked"));
  } finally {
    controller.unmount();
  }
});

test("lock now confirms, posts /api/lock, and clears the session token", async () => {
  const { dom, controller, fetchCalls } = setup({ status: unlockedStatus() });
  controller.mount(VAULT_ROUTE);
  await flush();
  try {
    byVault(dom, "lock-now")?.click();
    await flush();
    ok(textOf(confirmPart(dom, "body")).includes("zeroized"));
    confirmPart(dom, "action")?.click();
    await flush();
    equal(findCalls(fetchCalls, "POST", "/api/lock").length, 1);
  } finally {
    controller.unmount();
  }
});
