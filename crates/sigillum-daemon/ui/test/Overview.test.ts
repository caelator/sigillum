import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import type {
  AuditEvent,
  ConsolidationPlan,
  QueueJob,
  TreasuryOverviewResponse,
} from "../src/contracts";
import type { DaemonApi } from "../src/core/api";
import type { CoreRuntime } from "../src/core/live";
import { createCoreStore, type CoreStore } from "../src/core/state";
import {
  computeAttentionItems,
  createOverviewDestination,
  describeAuditEvent,
  freshnessWatermark,
  relativeAge,
  STALE_SCAN_THRESHOLD_SECS,
  type AttentionInput,
} from "../src/destinations/Overview";
import { BOOT_ROUTE, mockFetchJson, tick } from "./core-helpers";
import { FakeElement, installDom } from "./dom-fixture";

// ── Fixture helpers ───────────────────────────────────────────────────

function walk(node: FakeElement, fn: (element: FakeElement) => void): void {
  fn(node);
  for (const child of node.children) walk(child, fn);
}

function byClass(root: FakeElement, cls: string): FakeElement[] {
  const found: FakeElement[] = [];
  walk(root, (element) => {
    if (element.className.split(/\s+/).includes(cls)) found.push(element);
  });
  return found;
}

function statValues(host: FakeElement): Record<string, string> {
  const values: Record<string, string> = {};
  for (const stat of byClass(host, "stat")) {
    const label = stat.children[1]?.textContent ?? "";
    const value = stat.children[0]?.textContent ?? "";
    if (label) values[label] = value;
  }
  return values;
}

// ── Mock data ─────────────────────────────────────────────────────────

const NOW_SECS = Math.floor(Date.now() / 1000);

function samplePlan(id: string, updated = NOW_SECS - 900): ConsolidationPlan {
  return {
    id,
    status: "review_required",
    chain_id: 1,
    created_at_unix: updated - 3600,
    updated_at_unix: updated,
    summary: {
      total_steps: 4,
      blocked_steps: 1,
      review_required_steps: 3,
      approved_steps: 0,
      executable_steps: 0,
      value_items: 2,
    },
    steps: [],
  } as ConsolidationPlan;
}

function sampleQueueJob(id: string, updated = NOW_SECS - 300): QueueJob {
  return {
    id,
    kind: "plan_step_execution",
    state: "operator_action_required",
    attempts: 1,
    created_at_unix: updated - 600,
    updated_at_unix: updated,
    last_error: "receipt reverted",
  } as QueueJob;
}

function sampleTreasury(): TreasuryOverviewResponse {
  return {
    generated_at_unix: NOW_SECS - 60,
    tracked_address_count: 3,
    funded_address_count: 2,
    watch_only_address_count: 1,
    signer_address_count: 2,
    groups: [
      {
        wallet_family: "eth_seed",
        wallet_profile: "treasury",
        chain_id: 1,
        address_count: 3,
        funded_address_count: 2,
        native_total_wei_hex: "0x0",
        signer_address_count: 2,
        watch_only_address_count: 1,
        erc20_holding_count: 0,
        nft_holding_count: 0,
        defi_holding_count: 0,
        claimable_holding_count: 0,
        approval_exposure_count: 0,
        dormant_candidate_count: 0,
      },
    ],
    risk: {
      total_findings: 0,
      critical_findings: 0,
      high_findings: 0,
      medium_findings: 0,
      low_findings: 0,
    },
    plans: {
      total_plans: 0,
      latest_review_required_steps: 0,
      latest_approved_steps: 0,
      latest_executable_steps: 0,
      latest_blocked_steps: 0,
    },
  } as TreasuryOverviewResponse;
}

function auditEvents(count: number): AuditEvent[] {
  return Array.from({ length: count }, (_, index) => ({
    created_at_unix: NOW_SECS - index * 60,
    kind: "unlock.passphrase",
    compartment_id: 0,
    details: {},
  }));
}

/** Calm-workspace mock api; tests override the pieces they exercise. */
function calmApi(overrides: Partial<DaemonApi> = {}): Partial<DaemonApi> {
  return {
    listPlans: async () => ({ plans: [] }),
    listQueueJobs: async () => ({
      jobs: [],
      pagination: { total: 0, limit: 20, offset: 0, has_more: false },
    }),
    getTreasuryOverview: async () => sampleTreasury(),
    listInventoryWallets: async () => ({
      jobs: [],
      addresses: [
        {
          id: "a1",
          last_checked_at_unix: NOW_SECS - 600,
        },
      ],
      holdings: [],
      pagination: { total: 3, limit: 1, offset: 0, has_more: true },
    }),
    listAudit: async () => ({ events: [] }),
    runSelfCheck: async () => ({
      status: "pass",
      generated_at_unix: NOW_SECS - 30,
      checks: [
        { id: "c1", domain: "provider", subject: "mainnet", status: "pass", detail: "ok" },
      ],
    }),
    ...overrides,
  } as Partial<DaemonApi>;
}

function mockProviderAndChainFetch(): void {
  mockFetchJson((path: string) => {
    if (path.startsWith("/api/profiles/evm")) {
      return { profiles: [{ name: "mainnet" }, { name: "l2" }] };
    }
    if (path.startsWith("/api/chains")) {
      return {
        profiles: [
          {
            name: "ethereum",
            chain_family: "evm",
            chain_id: 1,
            enabled: true,
            native_symbol: "ETH",
            native_decimals: 18,
            finality_blocks: 12,
            capabilities: [],
            source: "builtin",
            builtin: true,
            updated_at_unix: 0,
          },
        ],
      };
    }
    return {};
  });
}

function makeRuntime(api: Partial<DaemonApi>): {
  runtime: CoreRuntime;
  store: CoreStore;
} {
  const store = createCoreStore(BOOT_ROUTE);
  const runtime = { store, api } as unknown as CoreRuntime;
  return { runtime, store };
}

function unlockedStatus() {
  return {
    initialized: true,
    locked: false,
    active_compartment: {
      compartment_id: 0,
      compartment_label: "Main",
      api_key_count: 2,
      secret_count: 3,
    },
    unlocked_compartments: [{ id: 0, label: "Main", threshold: 1 }],
  };
}

async function flush(): Promise<void> {
  await tick();
  await tick();
  await tick();
}

// ── Pure helpers ──────────────────────────────────────────────────────

test("Overview: relativeAge renders human units", () => {
  const now = 1_000_000_000_000;
  equal(relativeAge(null, now), "never");
  equal(relativeAge(0, now), "never");
  equal(relativeAge(now / 1000 - 10, now), "just now");
  equal(relativeAge(now / 1000 - 300, now), "5m ago");
  equal(relativeAge(now / 1000 - 3 * 3600, now), "3h ago");
  equal(relativeAge(now / 1000 - 2 * 86400, now), "2d ago");
});

test("Overview: describeAuditEvent humanizes kinds and detail suffixes", () => {
  equal(
    describeAuditEvent({ created_at_unix: 1, kind: "unlock.passphrase" }),
    "Unlocked with passphrase",
  );
  equal(
    describeAuditEvent({
      created_at_unix: 1,
      kind: "secret.set",
      details: { label: "api key" },
    }),
    "Stored encrypted secret — api key",
  );
  // Unknown kinds (newer daemon) still read as words, not raw enum text.
  equal(
    describeAuditEvent({ created_at_unix: 1, kind: "custom.event_x" }),
    "Custom event x",
  );
});

test("Overview: computeAttentionItems ranks danger first and deep-links plans", () => {
  const input: AttentionInput = {
    locked: false,
    plans: [samplePlan("plan-1")],
    chains: [
      {
        name: "ethereum",
        chain_family: "evm",
        chain_id: 1,
        enabled: true,
        native_symbol: "ETH",
        native_decimals: 18,
        finality_blocks: 12,
        capabilities: [],
        source: "builtin",
        builtin: true,
        updated_at_unix: 0,
      },
    ],
    queueJobs: [sampleQueueJob("j1")],
    queueTotal: 2,
    treasury: sampleTreasury(),
    newestScanUnix: NOW_SECS - 600,
    trackedAddressCount: 3,
    selfCheck: null,
    failedOperations: [],
  };
  const items = computeAttentionItems(input);
  equal(items.length, 2);
  equal(items[0].key, "queue-operator-action");
  equal(items[0].tier, "danger");
  equal(items[0].title, "2 queue jobs need operator action");
  ok(items[0].body.includes("receipt reverted"), items[0].body);
  equal(items[0].href, "#/move");
  equal(items[1].key, "plan-plan-1");
  equal(items[1].tier, "review");
  equal(items[1].title, "Plan on 1 (ethereum) needs review");
  equal(items[1].href, "#/move/plan/plan-1");
  ok(items[1].body.includes("3 of 4 steps"), items[1].body);
  ok(items[1].body.includes("1 blocked"), items[1].body);
});

test("Overview: computeAttentionItems aggregates beyond the plan row cap", () => {
  const plans = Array.from({ length: 7 }, (_, index) =>
    samplePlan("plan-" + index, NOW_SECS - index * 60),
  );
  const items = computeAttentionItems({
    locked: false,
    plans,
    chains: [],
    queueJobs: [],
    queueTotal: 0,
    treasury: null,
    newestScanUnix: NOW_SECS,
    trackedAddressCount: 1,
    selfCheck: null,
    failedOperations: [],
  });
  equal(items.length, 6);
  equal(items[5].key, "plans-aggregate");
  equal(items[5].title, "2 more plans need review");
  equal(items[5].href, "#/move");
});

test("Overview: computeAttentionItems covers self-check, risk, stale scans, failed ops", () => {
  const treasury = sampleTreasury();
  treasury.risk.critical_findings = 1;
  treasury.risk.high_findings = 2;
  const items = computeAttentionItems({
    locked: false,
    plans: [],
    chains: [],
    queueJobs: [],
    queueTotal: 0,
    treasury,
    newestScanUnix: NOW_SECS - STALE_SCAN_THRESHOLD_SECS - 3600,
    trackedAddressCount: 4,
    selfCheck: {
      status: "fail",
      failCount: 2,
      warnCount: 1,
      failDomains: ["provider", "policy"],
      warnDomains: ["watch-book"],
      atUnix: NOW_SECS - 120,
    },
    failedOperations: [
      {
        id: "op1",
        kind: "inventory_scan_evm",
        state: "failed",
        progress: { processed: 3 },
        created_at_unix: NOW_SECS - 500,
        updated_at_unix: NOW_SECS - 400,
        error: "provider timeout",
      },
      {
        id: "op2",
        kind: "inventory_scan_evm",
        state: "failed",
        progress: { processed: 1 },
        created_at_unix: NOW_SECS - 300,
        updated_at_unix: NOW_SECS - 200,
        error: "rate limited",
      },
    ],
  });
  const keys = items.map((item) => item.key);
  deepInclude(keys, [
    "selfcheck-fail",
    "risk-severe",
    "scan-stale",
    "ops-failed",
  ]);
  const selfCheck = items.find((item) => item.key === "selfcheck-fail");
  equal(selfCheck?.tier, "danger");
  ok(selfCheck?.body.includes("Provider, Policy"), selfCheck?.body);
  equal(selfCheck?.href, "#/vault");
  const risk = items.find((item) => item.key === "risk-severe");
  equal(risk?.title, "3 high-severity risk findings");
  equal(risk?.href, "#/portfolio");
  const stale = items.find((item) => item.key === "scan-stale");
  equal(stale?.tier, "review");
  const ops = items.find((item) => item.key === "ops-failed");
  equal(ops?.title, "2 background jobs failed");
  ok(ops?.body.includes("rate limited"), ops?.body);
});

function deepInclude(actual: string[], expected: string[]): void {
  for (const key of expected) {
    ok(actual.includes(key), "missing " + key + " in " + actual.join(","));
  }
}

test("Overview: computeAttentionItems is empty for a calm workspace", () => {
  const items = computeAttentionItems({
    locked: false,
    plans: [],
    chains: [],
    queueJobs: [],
    queueTotal: 0,
    treasury: sampleTreasury(),
    newestScanUnix: NOW_SECS - 600,
    trackedAddressCount: 3,
    selfCheck: {
      status: "pass",
      failCount: 0,
      warnCount: 0,
      failDomains: [],
      warnDomains: [],
      atUnix: NOW_SECS - 30,
    },
    failedOperations: [],
  });
  equal(items.length, 0);
});

test("Overview: freshnessWatermark takes the newest per-resource timestamp", () => {
  const newest = freshnessWatermark({
    treasury: sampleTreasury(), // NOW_SECS - 60
    newestScanUnix: NOW_SECS - 600,
    selfCheck: null,
    audit: auditEvents(2), // NOW_SECS
    plans: [samplePlan("p", NOW_SECS - 900)],
    queueJobs: [],
  });
  equal(newest, NOW_SECS);
  equal(
    freshnessWatermark({
      treasury: null,
      newestScanUnix: null,
      selfCheck: null,
      audit: [],
      plans: [],
      queueJobs: [],
    }),
    null,
  );
});

// ── Controller rendering (fake DOM) ──────────────────────────────────

test("Overview: mount renders the calm workspace from store + api", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  const { runtime, store } = makeRuntime(calmApi());
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  // Skeletons show while the first refresh is in flight.
  const hostEarly = dom.el("statusCard");
  ok(byClass(hostEarly, "skeleton").length > 0, "skeletons while loading");
  await flush();

  const host = dom.el("statusCard");
  ok(byClass(host, "dest-overview").length === 1, "root rendered");
  // Calm empty state IS the design.
  const titles = byClass(host, "section-empty-title");
  ok(
    titles.some((node) => node.textContent === "Nothing needs your attention"),
    "calm empty state",
  );
  equal(byClass(host, "attention-item").length, 0);
  // Strip: lock pill, compartment, and counts from store + resources.
  const pills = byClass(host, "pill");
  ok(
    pills.some((node) => node.textContent === "Unlocked"),
    "lock pill",
  );
  ok(
    byClass(host, "dest-overview-comp").some(
      (node) => node.textContent === "Compartment Main",
    ),
    "compartment label",
  );
  const stats = statValues(host);
  equal(stats["Providers"], "2");
  equal(stats["Wallets"], "1");
  equal(stats["Addresses"], "3 (2 funded)");
  equal(stats["Connection keys"], "2");
  equal(stats["Secrets"], "3");
  // Watermark from the newest resource timestamp.
  const watermark = byClass(host, "dest-overview-watermark")[0];
  ok(
    watermark.children[1].textContent.startsWith("Data current as of"),
    watermark.children[1].textContent,
  );
  // Audit digest is empty → empty state with a next action.
  ok(
    byClass(host, "section-empty-title").some(
      (node) => node.textContent === "No activity yet",
    ),
    "audit empty state",
  );
  controller.unmount();
});

test("Overview: attention queue renders ranked rows with deep links", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  const { runtime, store } = makeRuntime(
    calmApi({
      listPlans: async () => ({ plans: [samplePlan("plan-1")] }),
      listQueueJobs: async () => ({
        jobs: [sampleQueueJob("j1")],
        pagination: { total: 2, limit: 20, offset: 0, has_more: false },
      }),
    }),
  );
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();

  const host = dom.el("statusCard");
  const rows = byClass(host, "attention-item");
  equal(rows.length, 2);
  equal(rows[0].dataset.tier, "danger");
  equal(
    rows[0].children[0].children[0].textContent,
    "2 queue jobs need operator action",
  );
  equal(rows[1].dataset.tier, "review");
  const action = rows[1].children[1];
  equal(action.getAttribute("href"), "#/move/plan/plan-1");
  equal(action.textContent, "Review plan");
  controller.unmount();
});

test("Overview: refresh failure shows a persistent banner and Retry recovers", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  let queueFails = true;
  const { runtime, store } = makeRuntime(
    calmApi({
      listQueueJobs: async () => {
        if (queueFails) throw { code: "internal", error: "boom" };
        return {
          jobs: [],
          pagination: { total: 0, limit: 20, offset: 0, has_more: false },
        };
      },
    }),
  );
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();

  const host = dom.el("statusCard");
  let banners = byClass(host, "dest-overview-banner");
  equal(banners.length, 1);
  equal(banners[0].getAttribute("role"), "alert");
  ok(
    banners[0].children[0].children[1].textContent.includes("the queue"),
    banners[0].children[0].children[1].textContent,
  );

  // Retry is a real button; fixing the daemon and clicking clears the banner.
  queueFails = false;
  const retry = byClass(banners[0], "btn-ghost")[0];
  retry.click();
  await flush();
  banners = byClass(host, "dest-overview-banner");
  equal(banners.length, 0);
  controller.unmount();
});

test("Overview: vault_locked swaps the view to unlock guidance", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  const lockedError = { code: "vault_locked", error: "vault is locked" };
  const { runtime, store } = makeRuntime(
    calmApi({
      listPlans: async () => {
        throw lockedError;
      },
      listQueueJobs: async () => {
        throw lockedError;
      },
      getTreasuryOverview: async () => {
        throw lockedError;
      },
      listInventoryWallets: async () => {
        throw lockedError;
      },
      listAudit: async () => {
        throw lockedError;
      },
      runSelfCheck: async () => {
        throw lockedError;
      },
    }),
  );
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();

  const host = dom.el("statusCard");
  ok(
    byClass(host, "section-empty-title").some(
      (node) => node.textContent === "The vault is locked",
    ),
    "locked guidance",
  );
  const unlockLinks = byClass(host, "btn-primary").filter(
    (node) => node.getAttribute("href") === "#/vault",
  );
  equal(unlockLinks.length, 1);
  ok(!unlockLinks[0].classList.contains("hidden"), "unlock action visible");
  // No stale banner in locked mode — the guidance is the message.
  equal(byClass(host, "dest-overview-banner").length, 0);
  controller.unmount();
});

test("Overview: audit digest paginates via Show more", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  const seenLimits: number[] = [];
  const { runtime, store } = makeRuntime(
    calmApi({
      listAudit: async (query) => {
        const limit = query?.limit ?? 20;
        seenLimits.push(limit);
        return { events: auditEvents(Math.min(limit, 15)) };
      },
    }),
  );
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();

  const host = dom.el("statusCard");
  equal(byClass(host, "dest-overview-audit-row").length, 10);
  equal(seenLimits[0], 10);
  const moreRow = byClass(host, "dest-overview-more-row")[0];
  ok(!moreRow.classList.contains("hidden"), "show more visible");
  byClass(moreRow, "btn-ghost")[0].click();
  await flush();

  equal(seenLimits[1], 20);
  equal(byClass(host, "dest-overview-audit-row").length, 15);
  ok(
    byClass(host, "dest-overview-more-row")[0].classList.contains("hidden"),
    "show more hidden when exhausted",
  );
  controller.unmount();
});

test("Overview: queueEvents slice triggers a live queue refetch", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  let jobs: QueueJob[] = [];
  const { runtime, store } = makeRuntime(
    calmApi({
      listQueueJobs: async () => ({
        jobs,
        pagination: { total: jobs.length, limit: 20, offset: 0, has_more: false },
      }),
    }),
  );
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();
  const host = dom.el("statusCard");
  equal(byClass(host, "attention-item").length, 0);

  jobs = [sampleQueueJob("j9")];
  store.set("queueEvents", [
    { v: 1, job_id: "j9", state: "operator_action_required" },
  ]);
  await flush();

  const rows = byClass(host, "attention-item");
  equal(rows.length, 1);
  equal(rows[0].dataset.tier, "danger");
  controller.unmount();
});

test("Overview: unmount restores the legacy hero markup", async () => {
  const dom = installDom(["statusCard"]);
  mockProviderAndChainFetch();
  const host = dom.el("statusCard");
  host.innerHTML = "<p>legacy hero</p>";
  const { runtime, store } = makeRuntime(calmApi());
  store.set("status", unlockedStatus());

  const controller = createOverviewDestination(runtime);
  controller.mount(BOOT_ROUTE);
  await flush();
  ok(byClass(host, "dest-overview").length === 1, "takeover rendered");

  controller.unmount();
  equal(host.innerHTML, "<p>legacy hero</p>");
  equal(host.childNodes.length, 0);
  // Post-unmount store traffic must not touch the restored markup.
  store.set("operations", []);
  await flush();
  equal(host.innerHTML, "<p>legacy hero</p>");
});
