import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import { createDaemonApi } from "../src/core/api";
import type { CoreRuntime } from "../src/core/live";
import { parseHash, type Route } from "../src/core/router";
import { createCoreStore } from "../src/core/state";
import type { Operation } from "../src/contracts";
import {
  buildScanLaunchPlan,
  createPortfolioDestination,
  middleTruncate,
  relativeTime,
} from "../src/destinations/portfolio";
import { installDom, FakeElement } from "./dom-fixture";
import { mockFetchJson, sleep, tick } from "./core-helpers";

// ── Fake-DOM inspection helpers ─────────────────────────────────────

/** Aggregate descendant text (FakeElement.textContent is own-text only). */
function textOf(node: FakeElement): string {
  let text = node.textContent ?? "";
  for (const child of node.childNodes) {
    text += " " + textOf(child as FakeElement);
  }
  return text.replace(/\s+/g, " ").trim();
}

function findAll(
  root: FakeElement,
  predicate: (node: FakeElement) => boolean,
): FakeElement[] {
  const out: FakeElement[] = [];
  const walk = (node: FakeElement): void => {
    for (const child of node.children) {
      if (predicate(child)) out.push(child);
      walk(child);
    }
  };
  walk(root);
  return out;
}

function byHook(root: FakeElement, value: string): FakeElement[] {
  return findAll(
    root,
    (node) =>
      node.dataset?.portfolio === value ||
      node.attributes?.["data-portfolio"] === value,
  );
}

function oneByHook(root: FakeElement, value: string): FakeElement {
  const found = byHook(root, value);
  ok(found.length > 0, `expected element with data-portfolio="${value}"`);
  return found[0];
}

async function flush(times = 8): Promise<void> {
  for (let index = 0; index < times; index++) await tick();
}

// ── Runtime / route fakes ───────────────────────────────────────────

function makeRoute(path: string[] = []): Route {
  return {
    destination: "portfolio",
    path,
    params: {},
    hash: `#/portfolio${path.length ? "/" + path.join("/") : ""}`,
  };
}

function makeRuntime(boot: Route) {
  const store = createCoreStore(boot);
  const registered: string[] = [];
  const router = {
    route: () => store.get("route"),
    register(destination: string, pattern: string) {
      registered.push(`${destination}/${pattern}`);
    },
    navigate(hash: string) {
      const parsed = parseHash(hash);
      if (!parsed) return;
      store.set("route", {
        destination: parsed.destination,
        path: parsed.path,
        params: {},
        hash,
      });
    },
    start() {},
    stop() {},
  };
  const runtime = {
    store,
    api: createDaemonApi(),
    router,
    adapter: {},
    events: { start() {}, stop() {}, transport: () => "off" },
    notifyLegacySection() {},
    stop() {},
  } as unknown as CoreRuntime;
  return { runtime, store, registered };
}

function mountAt(path: string[] = []) {
  const dom = installDom(["inventoryCard"]);
  const route = makeRoute(path);
  const { runtime, store, registered } = makeRuntime(route);
  const controller = createPortfolioDestination(runtime);
  controller.mount(route);
  const host = dom.el("inventoryCard");
  return { dom, host, controller, store, registered };
}

// ── Sample data ─────────────────────────────────────────────────────

const NOW = Math.floor(Date.now() / 1000);
const SAMPLE_ADDRESS = "0x71C7d3e1234567890abcdef1234567890aA976F";

function sampleInventoryResponse() {
  return {
    jobs: [],
    addresses: [
      {
        id: "a1",
        wallet_family: "eth-seed",
        wallet_profile: "cold",
        provider_profile: "main",
        chain_id: 1,
        address: SAMPLE_ADDRESS,
        derivation_path: "m/44'/60'/0'/0/0",
        address_index: 0,
        activity_state: "funded",
        native_balance_wei_hex: "0x" + (42n * 10n ** 16n).toString(16),
        transaction_count: 3,
        classifications: ["signer_available"],
        source: "discovery",
        first_seen_at_unix: NOW - 100000,
        last_checked_at_unix: NOW - 3 * 3600,
      },
    ],
    holdings: [
      {
        id: "h1",
        wallet_family: "eth-seed",
        wallet_profile: "cold",
        provider_profile: "main",
        chain_id: 1,
        address: SAMPLE_ADDRESS,
        derivation_path: "m/44'/60'/0'/0/0",
        asset_kind: "erc20",
        asset_address: "0xtoken",
        amount_hex: "0x" + (5n * 10n ** 6n).toString(16),
        source: "discovery",
        status: "detected",
        first_seen_at_unix: NOW - 9000,
        last_checked_at_unix: NOW - 600,
      },
    ],
    nft_metadata_cache: [],
    pagination: { total: 1, limit: 25, offset: 0, has_more: false },
  };
}

function mockPortfolioFetch(inventory: unknown = sampleInventoryResponse()) {
  mockFetchJson((path: string) => {
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
            updated_at_unix: NOW,
          },
        ],
      };
    }
    if (path.startsWith("/api/inventory/token-registry")) {
      return {
        lists: [
          {
            id: "l1",
            name: "main",
            compartment_id: 0,
            source: "import",
            entries: [
              { chain_id: 1, address: "0xtoken", symbol: "USDC", decimals: 6 },
            ],
            created_at_unix: NOW,
            updated_at_unix: NOW,
          },
        ],
      };
    }
    if (path.startsWith("/api/inventory/wallets")) return inventory;
    return {};
  });
}

// ── Pure helpers ────────────────────────────────────────────────────

test("Portfolio relativeTime renders human freshness", () => {
  equal(relativeTime(undefined, NOW), "never");
  equal(relativeTime(NOW, NOW), "just now");
  equal(relativeTime(NOW - 30, NOW), "just now");
  equal(relativeTime(NOW - 60, NOW), "a minute ago");
  equal(relativeTime(NOW - 5 * 60, NOW), "5 min ago");
  equal(relativeTime(NOW - 3600, NOW), "an hour ago");
  equal(relativeTime(NOW - 3 * 3600, NOW), "3h ago");
  equal(relativeTime(NOW - 86400, NOW), "a day ago");
  equal(relativeTime(NOW - 3 * 86400, NOW), "3d ago");
});

test("Portfolio middleTruncate keeps head and tail", () => {
  equal(middleTruncate(SAMPLE_ADDRESS), "0x71C7d3…aA976F");
  equal(middleTruncate("0xshort"), "0xshort");
  equal(middleTruncate(""), "-");
});

// ── Launch plan (scan stepper scope mapping) ────────────────────────

test("Portfolio buildScanLaunchPlan: all selected stays ONE unfiltered scan", () => {
  const plan = buildScanLaunchPlan({
    wallets: [
      { family: "eth-seed", profile: "cold" },
      { family: "eth-xpub", profile: "watch" },
    ],
    selectedWallets: new Set(["eth-seed/cold", "eth-xpub/watch"]),
    providers: [
      { name: "a", chainId: 1 },
      { name: "b", chainId: 1 },
    ],
    selectedProviders: new Set(["a", "b"]),
    includeWatchBook: true,
  });
  ok(plan.ok);
  if (!plan.ok) return;
  equal(plan.scans.length, 1);
  equal(plan.allProviders, true);
  equal(plan.allWallets, true);
  deepLooseEqual(plan.scans[0], { include_watch_book: true });
});

test("Portfolio buildScanLaunchPlan: single wallet + watch book fans out honestly", () => {
  const plan = buildScanLaunchPlan({
    wallets: [
      { family: "eth-seed", profile: "cold" },
      { family: "eth-xpub", profile: "watch" },
    ],
    selectedWallets: new Set(["eth-seed/cold"]),
    providers: [
      { name: "a", chainId: 1 },
      { name: "b", chainId: 1 },
    ],
    selectedProviders: new Set(["a", "b"]),
    includeWatchBook: true,
  });
  ok(plan.ok);
  if (!plan.ok) return;
  equal(plan.scans.length, 2);
  deepLooseEqual(plan.scans[0], {
    wallet_family: "eth-seed",
    wallet_profile: "cold",
  });
  deepLooseEqual(plan.scans[1], {
    wallet_family: "eth-watch",
    include_watch_book: true,
  });
  equal(plan.allWallets, false);
});

test("Portfolio buildScanLaunchPlan: provider subset fans out per provider", () => {
  const plan = buildScanLaunchPlan({
    wallets: [],
    selectedWallets: new Set(),
    providers: [
      { name: "a", chainId: 1 },
      { name: "b", chainId: 1 },
      { name: "c", chainId: 8453 },
    ],
    selectedProviders: new Set(["a", "c"]),
    includeWatchBook: true,
  });
  ok(plan.ok);
  if (!plan.ok) return;
  equal(plan.scans.length, 2);
  equal(plan.scans[0].provider_profile, "a");
  equal(plan.scans[1].provider_profile, "c");
  equal(plan.scans[0].wallet_family, "eth-watch");
  equal(plan.allProviders, false);
});

test("Portfolio buildScanLaunchPlan: rejects empty and over-wide selections", () => {
  const noProviders = buildScanLaunchPlan({
    wallets: [],
    selectedWallets: new Set(),
    providers: [],
    selectedProviders: new Set(),
    includeWatchBook: true,
  });
  equal(noProviders.ok, false);

  const noneSelected = buildScanLaunchPlan({
    wallets: [],
    selectedWallets: new Set(),
    providers: [{ name: "a", chainId: 1 }],
    selectedProviders: new Set(),
    includeWatchBook: true,
  });
  equal(noneSelected.ok, false);

  const noWalletsNoWatch = buildScanLaunchPlan({
    wallets: [{ family: "eth-seed", profile: "cold" }],
    selectedWallets: new Set(),
    providers: [{ name: "a", chainId: 1 }],
    selectedProviders: new Set(["a"]),
    includeWatchBook: false,
  });
  equal(noWalletsNoWatch.ok, false);

  const wallets = Array.from({ length: 3 }, (_, index) => ({
    family: "eth-seed",
    profile: `w${index}`,
  }));
  const providers = Array.from({ length: 3 }, (_, index) => ({
    name: `p${index}`,
    chainId: 1,
  }));
  const tooWide = buildScanLaunchPlan({
    wallets,
    // Two of three wallets AND two of three providers: 2 x 2 = 4 scans.
    selectedWallets: new Set(
      wallets.slice(0, 2).map((w) => `${w.family}/${w.profile}`),
    ),
    providers,
    selectedProviders: new Set(providers.map((p) => p.name).slice(0, 2)),
    includeWatchBook: false,
    maxScans: 2,
  });
  equal(tooWide.ok, false);
});

function deepLooseEqual(actual: unknown, expected: Record<string, unknown>) {
  const record = actual as Record<string, unknown>;
  for (const [key, value] of Object.entries(expected)) {
    equal(record[key], value, `key ${key}`);
  }
  for (const key of Object.keys(record)) {
    ok(key in expected, `unexpected key ${key}`);
  }
}

// ── Holdings view ───────────────────────────────────────────────────

test("Portfolio holdings view: skeleton first, then humanized table rows", async () => {
  mockPortfolioFetch();
  const { host, controller, registered } = mountAt();

  // Skeleton is rendered synchronously before the first fetch resolves.
  ok(byHook(host, "skeleton").length > 0, "skeleton while first-loading");

  await flush();

  ok(registered.includes("portfolio/scan"));
  ok(registered.includes("portfolio/risk"));
  ok(registered.includes("portfolio/tokens"));

  const rows = byHook(host, "address-row");
  equal(rows.length, 1);
  const text = textOf(rows[0]);
  ok(text.includes("1 (ethereum)"), `chain name in row: ${text}`);
  ok(text.includes("0.42 ETH"), `human amount in row: ${text}`);
  ok(text.includes("Seed wallet · cold"), `wallet in row: ${text}`);
  ok(text.includes("Signer"), `signer pill in row: ${text}`);
  ok(text.includes("Funded"), `state pill in row: ${text}`);
  ok(text.includes("3h ago"), `freshness in row: ${text}`);
  ok(text.includes("0x71C7d3…aA976F"), `truncated address in row: ${text}`);

  const holdingRows = byHook(host, "holding-row");
  equal(holdingRows.length, 1);
  ok(textOf(holdingRows[0]).includes("5 USDC"), "registry humanized amount");

  const pageLabel = oneByHook(host, "addresses-page-label");
  equal(pageLabel.textContent, "1–1 of 1");

  controller.unmount();
});

test("Portfolio holdings view: empty state offers the next action", async () => {
  mockPortfolioFetch({
    jobs: [],
    addresses: [],
    holdings: [],
    nft_metadata_cache: [],
  });
  const { host, controller } = mountAt();
  await flush();

  const empty = oneByHook(host, "addresses-empty");
  const text = textOf(empty);
  ok(text.includes("No holdings discovered yet"), text);
  const actions = findAll(
    empty,
    (node) => node.tagName === "A" && node.attributes.href === "#/portfolio/scan",
  );
  ok(actions.length > 0, "empty state links to the scan stepper");

  controller.unmount();
});

test("Portfolio holdings view: balance filter drives the 1.5 query params", async () => {
  const paths: string[] = [];
  mockFetchJson((path: string) => {
    paths.push(path);
    if (path.startsWith("/api/inventory/wallets")) {
      return { jobs: [], addresses: [], holdings: [], pagination: { total: 0, limit: 25, offset: 0, has_more: false } };
    }
    if (path.startsWith("/api/chains")) return { profiles: [] };
    if (path.startsWith("/api/inventory/token-registry")) return { lists: [] };
    return {};
  });
  const { host, controller } = mountAt();
  await flush();

  const select = oneByHook(host, "filter-funded") as FakeElement & {
    value: string;
  };
  select.value = "funded";
  select.dispatchEvent({ type: "change", target: select });
  await flush();

  const walletCalls = paths.filter((path) =>
    path.startsWith("/api/inventory/wallets"),
  );
  ok(walletCalls.length >= 2, "refetch after filter change");
  ok(
    walletCalls[walletCalls.length - 1].includes("funded=true"),
    `funded param in ${walletCalls[walletCalls.length - 1]}`,
  );

  controller.unmount();
});

test("Portfolio holdings view: vault_locked flips to unlock guidance", async () => {
  mockFetchJson(() => ({ code: "vault_locked", error: "The vault is locked." }));
  const { host, controller } = mountAt();
  await flush();

  const text = textOf(host);
  ok(text.includes("The vault is locked"), text);
  const unlockLinks = findAll(
    host,
    (node) => node.tagName === "A" && node.attributes.href === "#/vault",
  );
  ok(unlockLinks.length > 0, "guidance links to #/vault");

  controller.unmount();
});

test("Portfolio holdings view: failed refresh shows a persistent banner until retry", async () => {
  let fail = false;
  mockFetchJson((path: string) => {
    if (fail) return { code: "unavailable", error: "daemon unreachable" };
    if (path.startsWith("/api/chains")) return { profiles: [] };
    if (path.startsWith("/api/inventory/token-registry")) return { lists: [] };
    if (path.startsWith("/api/inventory/wallets")) {
      return { jobs: [], addresses: [], holdings: [], pagination: { total: 0, limit: 25, offset: 0, has_more: false } };
    }
    return {};
  });
  const { host, controller, store } = mountAt();
  await flush();
  equal(byHook(host, "banner").length, 0);

  fail = true;
  store.set("resync", 1);
  await flush();
  await sleep(300); // debounce
  await flush();

  const banner = oneByHook(host, "banner");
  ok(textOf(banner).includes("may be stale"), textOf(banner));

  fail = false;
  const retry = findAll(
    banner,
    (node) => node.tagName === "BUTTON" && node.textContent === "Retry now",
  );
  equal(retry.length, 1);
  retry[0].click();
  await flush();
  equal(byHook(host, "banner").length, 0, "banner clears after a good refresh");

  controller.unmount();
});

// ── Scan stepper ────────────────────────────────────────────────────

test("Portfolio scan stepper: pick wallets → providers with partition → launch → live progress → results", async () => {
  const scanBodies: Record<string, unknown>[] = [];
  const cancelPaths: string[] = [];
  let jobCompleted = false;

  mockFetchJson((path: string, init: unknown) => {
    const method = (init as { method?: string })?.method ?? "GET";
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
            updated_at_unix: NOW,
          },
        ],
      };
    }
    if (path.startsWith("/api/profiles/evm")) {
      return {
        profiles: [
          { name: "alpha", chain_id: 1 },
          { name: "beta", chain_id: 1 },
          { name: "sidechain", chain_id: 8453 },
        ],
      };
    }
    if (path.startsWith("/api/profiles/eth-seed")) {
      return { profiles: [{ name: "cold" }] };
    }
    if (path.startsWith("/api/profiles/eth-xpub")) {
      return { profiles: [{ name: "watch" }] };
    }
    if (path.startsWith("/api/inventory/token-registry")) return { lists: [] };
    if (path.startsWith("/api/inventory/wallets")) {
      return {
        jobs: jobCompleted
          ? [
              {
                id: "job-1",
                status: "completed",
                source: "operator",
                wallet_profiles: [],
                provider_profiles: ["alpha", "beta", "sidechain"],
                chain_ids: [1, 8453],
                addresses_scanned: 10,
                active_addresses: 2,
                holdings_detected: 1,
                started_at_unix: NOW - 60,
                completed_at_unix: NOW,
              },
            ]
          : [],
        addresses: [],
        holdings: [],
        nft_metadata_cache: [],
      };
    }
    if (path === "/api/inventory/scan/evm" && method === "POST") {
      scanBodies.push(
        JSON.parse((init as { body: string }).body) as Record<string, unknown>,
      );
      return {
        job: { id: "job-1", status: "running", source: "operator", started_at_unix: NOW },
        operation: {
          id: "op-1",
          kind: "inventory_scan_evm",
          state: "running",
          progress: { processed: 0 },
          related_ids: ["job-1"],
          created_at_unix: NOW,
          updated_at_unix: NOW,
        },
        addresses: [],
        holdings: [],
      };
    }
    if (path.includes("/api/operations/") && path.endsWith("/cancel")) {
      cancelPaths.push(path);
      return { status: "cancel_requested" };
    }
    return {};
  });
  const { host, controller, store } = mountAt(["scan"]);
  await flush();

  // Step 1: wallets (both selected by default).
  const walletChecks = byHook(host, "wallet-check");
  equal(walletChecks.length, 2);
  ok((walletChecks[0] as FakeElement & { checked: boolean }).checked);
  oneByHook(host, "step-next").click();
  await flush(2);

  // Step 2: providers grouped per chain; partition option visible (2 on chain 1).
  equal(byHook(host, "provider-check").length, 3);
  const partition = oneByHook(host, "partition-check") as FakeElement & {
    checked: boolean;
  };
  partition.checked = true;
  partition.dispatchEvent({ type: "change", target: partition });
  await flush(2);
  ok(textOf(oneByHook(host, "partition-note")).includes("disjoint"));
  oneByHook(host, "step-next").click();
  await flush(2);

  // Step 3: review summary, then launch.
  ok(textOf(oneByHook(host, "launch-summary")).includes("One scan will start"));
  oneByHook(host, "launch").click();
  await flush();

  equal(scanBodies.length, 1);
  const body = scanBodies[0];
  equal(body.run_async, true, "background by default");
  equal(body.partition_providers, true, "partitioning carried");
  equal(body.include_watch_book, true);
  equal(body.wallet_family ?? undefined, undefined, "no wallet filter when all selected");
  equal(body.provider_profile ?? undefined, undefined, "no provider filter when all selected");
  equal(body.block_tag, "latest");
  equal(body.gap_limit, 20, "legacy default preserved");
  equal(body.max_index, 200, "legacy default preserved");
  equal(body.account_limit, 3, "legacy default preserved");
  equal(body.discover_erc20_transfers, false);

  // Step 4: operation not in the store yet → "starting" placeholder.
  ok(textOf(host).includes("starting"), "waiting placeholder before first event");

  const running: Operation = {
    id: "op-1",
    kind: "inventory_scan_evm",
    state: "running",
    progress: { processed: 5 },
    related_ids: ["job-1"],
    created_at_unix: NOW,
    updated_at_unix: NOW,
  };
  store.set("operations", [running]);
  await flush(2);
  ok(textOf(host).includes("5 checks so far"), "live progress from the store");

  // Cancel is wired to POST /api/operations/{id}/cancel.
  oneByHook(host, "op-cancel").click();
  await flush();
  ok(cancelPaths.includes("/api/operations/op-1/cancel"), "cancel endpoint hit");

  // Terminal transition → refetch → results summary from the job record.
  jobCompleted = true;
  store.set("operations", [
    { ...running, state: "completed", progress: { processed: 10 } },
  ]);
  await flush();
  await sleep(300);
  await flush();
  const summary = oneByHook(host, "results-summary");
  ok(textOf(summary).includes("10 addresses checked"), textOf(summary));

  controller.unmount();
});

// ── Risk view ───────────────────────────────────────────────────────

test("Portfolio risk view: findings render with tiers and plain language; catalog add + field validation", async () => {
  const posts: { path: string; body: Record<string, unknown> }[] = [];
  let catalogFails = false;
  mockFetchJson((path: string, init: unknown) => {
    const method = (init as { method?: string })?.method ?? "GET";
    if (path.startsWith("/api/risk/findings")) {
      return {
        findings: [
          {
            id: "common_gas_funder:1:0xaaa",
            category: "common_gas_funder",
            risk_level: "high",
            status: "open",
            wallet_family: "eth-stealth",
            wallet_profile: "w",
            provider_profile: "p",
            chain_id: 1,
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            subject_type: "gas_funder",
            subject: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            source: "local-risk-engine",
            recommendation:
              "Fund each party's gas from a distinct sponsor address.",
            evidence: ["Gas funder/sponsor: 0xaaa"],
            first_seen_at_unix: NOW - 3600,
            last_checked_at_unix: NOW - 60,
          },
        ],
        pagination: { total: 1, limit: 25, offset: 0, has_more: false },
      };
    }
    if (path.startsWith("/api/risk/catalog") && method === "GET") {
      return { entries: [] };
    }
    if (path === "/api/risk/catalog/upsert" && method === "POST") {
      const body = JSON.parse(
        (init as { body: string }).body,
      ) as Record<string, unknown>;
      posts.push({ path, body });
      if (catalogFails) {
        return {
          code: "validation_failed",
          error: "Validation failed",
          fields: [{ field: "address", message: "not an address" }],
        };
      }
      return { status: "ok" };
    }
    return {};
  });
  const { host, controller } = mountAt(["risk"]);
  await flush();

  const rows = byHook(host, "finding-row");
  equal(rows.length, 1);
  equal(rows[0].dataset.tier, "danger", "high severity grades the row");
  const text = textOf(rows[0]);
  ok(
    text.includes("One gas funder pays into several payer identities"),
    `common_gas_funder in plain language: ${text}`,
  );
  ok(text.includes("distinct sponsor address"), "recommendation shown");
  ok(text.includes("high"), "severity pill");

  // Catalog add: successful submit posts the DTO.
  const form = oneByHook(host, "catalog-form");
  const address = form.querySelector('[name="address"]') as FakeElement & {
    value: string;
  };
  address.value = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  form.dispatchEvent({ type: "submit", target: form });
  await flush();
  equal(posts.length, 1);
  equal(posts[0].body.address, "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
  equal(posts[0].body.risk_level, "trusted");

  // validation_failed surfaces field-level messages + aria-invalid.
  catalogFails = true;
  address.value = "not-an-address";
  form.dispatchEvent({ type: "submit", target: form });
  await flush();
  const errors = oneByHook(host, "catalog-errors");
  ok(textOf(errors).includes("address: not an address"), textOf(errors));
  equal(address.getAttribute("aria-invalid"), "true");

  controller.unmount();
});

// ── Tokens view ─────────────────────────────────────────────────────

test("Portfolio tokens view: registry import validates locally; opt-in toggle posts DTO", async () => {
  const posts: { path: string; body: Record<string, unknown> }[] = [];
  mockFetchJson((path: string, init: unknown) => {
    const method = (init as { method?: string })?.method ?? "GET";
    if (path.startsWith("/api/chains")) return { profiles: [] };
    if (path.startsWith("/api/inventory/token-registry") && method === "GET") {
      return { lists: [] };
    }
    if (path.startsWith("/api/inventory/nft-metadata/opt-ins") && method === "GET") {
      return {
        opt_ins: [
          {
            chain_id: 1,
            contract_address: "0xcccccccccccccccccccccccccccccccccccccccc",
            enabled: true,
            created_at_unix: NOW - 1000,
            updated_at_unix: NOW - 100,
          },
        ],
        ipfs_gateway_url: "",
      };
    }
    if (path.startsWith("/api/inventory/wallets")) {
      return { jobs: [], addresses: [], holdings: [], nft_metadata_cache: [] };
    }
    if (method === "POST") {
      posts.push({
        path,
        body: JSON.parse((init as { body: string }).body) as Record<
          string,
          unknown
        >,
      });
      return { status: "ok" };
    }
    return {};
  });
  const { host, controller } = mountAt(["tokens"]);
  await flush();

  // Empty registry name → local validation, no network call.
  const form = oneByHook(host, "registry-form");
  form.dispatchEvent({ type: "submit", target: form });
  await flush();
  ok(
    textOf(oneByHook(host, "registry-errors")).includes("list name is required"),
    "client-side validation renders",
  );
  equal(
    posts.filter((post) => post.path.includes("token-registry/import")).length,
    0,
    "no request on invalid form",
  );

  // Opt-in row renders and the toggle posts enabled:false.
  const optInRows = byHook(host, "optin-row");
  equal(optInRows.length, 1);
  oneByHook(host, "optin-toggle").click();
  await flush();
  const toggle = posts.find((post) => post.path.includes("opt-ins/upsert"));
  ok(toggle, "toggle posts upsert");
  equal(toggle?.body.enabled, false);

  controller.unmount();
});
