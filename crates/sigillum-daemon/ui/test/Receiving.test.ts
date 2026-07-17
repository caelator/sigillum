import { deepEqual, equal, ok } from "node:assert/strict";
import { test } from "node:test";

import type {
  Counterparty,
  EthStealthDeposit,
  ReceivingOverviewResponse,
  TreasuryReceiveAllocation,
} from "../src/contracts";
import { createDaemonApi } from "../src/core/api";
import type { CoreRuntime } from "../src/core/live";
import type { Route } from "../src/core/router";
import { createCoreStore, type CoreStore } from "../src/core/state";
import {
  buildAddressCards,
  createReceivingDestination,
  depositGasNotes,
  depositLifecycle,
  oneTimeBlockerText,
} from "../src/destinations/Receiving";
import { installDom, type FakeElement } from "./dom-fixture";
import { mockFetchJson, tick } from "./core-helpers";

// ── Fixture helpers ───────────────────────────────────────────────────

const ADDRESS = "0x71C7e6B0f5A0b3C4d5E6f7A8B9c0D1E2F3A4976F";
const TRUNCATED = "0x71C7…976F";
const SWEEP_DEST = "0x000000000000000000000000000000000000dEaD";
const DEPOSIT_ADDRESS = "0x000000000000000000000000000000000000bEef";

const RECEIVE_ROUTE: Route = {
  destination: "receive",
  path: [],
  params: {},
  hash: "#/receive",
};

async function settle(rounds = 8): Promise<void> {
  for (let i = 0; i < rounds; i++) await tick();
}

function walkAll(node: FakeElement, visit: (el: FakeElement) => boolean): FakeElement[] {
  const found: FakeElement[] = [];
  for (const child of node.children) {
    if (visit(child)) found.push(child);
    found.push(...walkAll(child, visit));
  }
  return found;
}

function findFirst(
  node: FakeElement,
  visit: (el: FakeElement) => boolean,
): FakeElement | null {
  return walkAll(node, visit)[0] ?? null;
}

function hasClass(el: FakeElement, cls: string): boolean {
  return el.className.split(" ").includes(cls);
}

function byClass(root: FakeElement, cls: string): FakeElement | null {
  return findFirst(root, (el) => hasClass(el, cls));
}

function byText(root: FakeElement, text: string): FakeElement | null {
  return findFirst(root, (el) => el.textContent === text);
}

function buttonByText(root: FakeElement, text: string): FakeElement | null {
  return findFirst(root, (el) => el.tagName === "BUTTON" && el.textContent === text);
}

interface RecordedCall {
  method: string;
  path: string;
  body?: Record<string, unknown>;
}

type RouteValue = unknown | ((call: RecordedCall) => unknown);

/** Mock fetch with `METHOD path-prefix` routes; records every call. */
function mockRoutes(routes: Record<string, RouteValue>): RecordedCall[] {
  const calls: RecordedCall[] = [];
  mockFetchJson((path: string, init: unknown) => {
    const request = init as { method?: string; body?: string } | undefined;
    const method = request?.method ?? "GET";
    const call: RecordedCall = {
      method,
      path,
      body: request?.body ? (JSON.parse(request.body) as Record<string, unknown>) : undefined,
    };
    calls.push(call);
    for (const key of Object.keys(routes)) {
      const [routeMethod, prefix] = key.split(" ", 2);
      if (routeMethod === method && path.startsWith(prefix)) {
        const value = routes[key];
        return typeof value === "function" ? value(call) : value;
      }
    }
    return {};
  });
  return calls;
}

interface RuntimeHarness {
  runtime: CoreRuntime;
  store: CoreStore;
  navigated: string[];
  registered: string[];
}

function makeRuntime(): RuntimeHarness {
  const store = createCoreStore(RECEIVE_ROUTE);
  const navigated: string[] = [];
  const registered: string[] = [];
  const runtime = {
    store,
    api: createDaemonApi(),
    router: {
      route: () => RECEIVE_ROUTE,
      register: (destination: string, pattern: string) => {
        registered.push(destination + "/" + pattern);
      },
      navigate: (hash: string) => {
        navigated.push(hash);
      },
      start: () => {},
      stop: () => {},
    },
    adapter: {},
    events: {},
    notifyLegacySection: () => {},
    stop: () => {},
  } as unknown as CoreRuntime;
  return { runtime, store, navigated, registered };
}

// ── Sample data ───────────────────────────────────────────────────────

function sampleOverview(): ReceivingOverviewResponse {
  return {
    generated_at_unix: 1700000100,
    include_retired: false,
    groups: [
      {
        counterparty: null,
        item_count: 1,
        native_total_wei_hex: "0x6f05b59d3b20000", // 0.5 ETH
        items: [
          {
            source_type: "hd",
            address: ADDRESS,
            chain_id: 1,
            purpose: "invoices",
            label: null,
            counterparty_id: "p1",
            linkage_warning: null,
            balance_native_wei_hex: "0x6f05b59d3b20000",
            balance_known: true,
            status: "active",
            created_at_unix: 1700000000,
          },
        ],
      },
    ],
    totals: {
      item_count: 1,
      hd_count: 1,
      stealth_count: 0,
      native_total_wei_hex: "0x6f05b59d3b20000",
    },
    coverage: { addresses_total: 1, addresses_with_known_balance: 1, note: "covered" },
  };
}

function sampleAllocation(): TreasuryReceiveAllocation {
  return {
    id: "a1",
    wallet_family: "eth",
    wallet_profile: "ops",
    chain_id: 1,
    address: ADDRESS,
    derivation_path: "m/44'/60'/0'/0/0",
    address_index: 0,
    purpose: "invoices",
    label: null,
    status: "active",
    created_at_unix: 1700000000,
    counterparty_id: "p1",
    one_time: true,
    sweep_destination_address: SWEEP_DEST,
    min_sweep_amount_hex: "0x16345785d8a0000", // 0.025 ETH
    purge_after_sweep: true,
    lifecycle_state: "watching",
    sweep_blocker: "below_threshold",
  };
}

function sampleParty(): Counterparty {
  return {
    id: "p1",
    name: "Acme",
    note: null,
    sweep_destination_address: null,
    created_at_unix: 1700000000,
  };
}

function sampleDeposit(overrides: Partial<EthStealthDeposit> = {}): EthStealthDeposit {
  return {
    id: "d1",
    status: "funded_needs_gas",
    asset_kind: "erc20",
    wallet_profile: "ops",
    chain_id: 1,
    wallet: "0xwallet",
    short_name: "ops",
    stealth_meta_address: "st:eth:0xmeta",
    stealth_address: DEPOSIT_ADDRESS,
    ephemeral_public_key_hex: "0x02aa",
    view_tag_hex: "0xab",
    expected_amount_hex: "0x10",
    observed_amount_hex: "0x10",
    observed_native_balance_wei_hex: "0x0",
    auto_queue_sweep: true,
    requested_gas_wei_hex: "0x2386f26fc10000", // 0.01 ETH
    created_at_unix: 1700000000,
    updated_at_unix: 1700000100,
    ...overrides,
  };
}

function baseRoutes(overrides: Record<string, RouteValue> = {}): Record<string, RouteValue> {
  return {
    "GET /api/receiving/overview": sampleOverview(),
    "GET /api/treasury/receive-addresses": { allocations: [sampleAllocation()] },
    "GET /api/treasury/parties": { parties: [sampleParty()] },
    "GET /api/deposits/eth-stealth": {
      deposits: [],
      pagination: { total: 0, limit: 10, offset: 0, has_more: false },
    },
    "GET /api/profiles/eth-stealth": { profiles: [{ name: "ops", wallet: "0xwallet" }] },
    ...overrides,
  };
}

function install(): { host: FakeElement; siblings: FakeElement[] } {
  const dom = installDom(["receivingCard", "receiveBookCard", "depositsCard"]);
  return {
    host: dom.el("receivingCard"),
    siblings: [dom.el("receiveBookCard"), dom.el("depositsCard")],
  };
}

// ── Tests ─────────────────────────────────────────────────────────────

test("Receiving renders address cards with humanized balance and one-time lifecycle", async () => {
  const { host, siblings } = install();
  mockRoutes(baseRoutes());
  const { runtime, registered } = makeRuntime();
  const controller = createReceivingDestination(runtime);

  controller.mount(RECEIVE_ROUTE);

  // Takes over the legacy card; sibling legacy cards are hidden; skeletons show.
  equal(host.children.length, 1);
  ok(hasClass(host.children[0], "dest-recv"));
  ok(siblings.every((sibling) => sibling.classList.contains("hidden")));
  const addressEmptyPre = findFirst(host, (el) => hasClass(el, "skeleton"));
  ok(addressEmptyPre, "skeletons visible while first-loading");
  deepEqual(registered.sort(), ["receive/deposits", "receive/pay"]);

  await settle();

  const grid = byClass(host, "recv-grid");
  ok(grid, "address grid rendered");
  equal(grid.children.length, 1, "one merged card for the allocation+overview address");
  ok(byText(host, "0.5 ETH"), "balance humanized to ETH");
  ok(byText(host, TRUNCATED), "address middle-truncated");
  ok(byText(host, "One-time"), "one-time badge");
  ok(byText(host, "watching"), "lifecycle pill");
  ok(byText(host, "Not swept yet: below the sweep threshold."), "blocker humanized");
  ok(byText(host, "invoices · for Acme"), "purpose + counterparty line");
  ok(byText(host, "Address " + ADDRESS), "full address only behind the details disclosure");
  ok(
    byText(
      host,
      "1 of 1 addresses have a known balance — balances last checked " +
        new Date(1700000100 * 1000).toLocaleString() +
        ".",
    ),
    "coverage + freshness line",
  );

  controller.unmount();
});

test("Receiving renders stealth deposits as a guided lifecycle with the gas explainer", async () => {
  const { host } = install();
  mockRoutes(
    baseRoutes({
      "GET /api/deposits/eth-stealth": {
        deposits: [sampleDeposit()],
        pagination: { total: 1, limit: 10, offset: 0, has_more: false },
      },
    }),
  );
  const { runtime } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  const card = byClass(host, "recv-deposit-card");
  ok(card, "deposit card rendered");
  equal(card.dataset.tier, "review", "needs-gas deposit graded review tier");

  const stepper = byClass(host, "recv-lifecycle");
  ok(stepper, "lifecycle stepper rendered");
  deepEqual(
    stepper.children.map((step) => step.dataset.state),
    ["done", "done", "attention", "todo"],
    "announced + funded done, gas-ready needs attention, swept pending",
  );

  ok(
    byText(
      host,
      "Payment received, but the address holds no native gas for the sweep — ask the payer to attach gas, or fund the address manually.",
    ),
    "funded_needs_gas explainer",
  );
  ok(
    byText(host, "The payer was asked to attach 0.01 ETH for the sweep."),
    "requested payer gas humanized",
  );
  ok(byText(host, "Raw token amounts (base units)"), "ERC-20 amounts stay raw behind details");
  ok(byText(host, "Paid by"), "counterparty tagging control");
  const tagSelect = findFirst(
    host,
    (el) =>
      el.tagName === "SELECT" &&
      (el.attributes["aria-label"] ?? "").startsWith("Counterparty for deposit"),
  );
  ok(tagSelect, "tag select present");
  deepEqual(
    tagSelect.children.map((option) => option.textContent),
    ["No counterparty", "Acme"],
  );

  controller.unmount();
});

test("Receiving gates the sweep behind the confirm dialog and posts to the endpoint", async () => {
  const { host } = install();
  const calls = mockRoutes(
    baseRoutes({
      "GET /api/deposits/eth-stealth": {
        deposits: [
          sampleDeposit({
            id: "d9",
            status: "funded",
            asset_kind: "native",
            requested_gas_wei_hex: null,
            expected_amount_hex: null,
            observed_amount_hex: "0x6f05b59d3b20000",
          }),
        ],
        pagination: { total: 1, limit: 10, offset: 0, has_more: false },
      },
      "POST /api/deposits/eth-stealth/enqueue-sweep": { job: { id: "j7" } },
    }),
  );
  const { runtime } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  const sweep = buttonByText(host, "Queue sweep");
  ok(sweep, "sweep action present on a funded deposit");
  sweep.click();
  await settle(2);

  const overlay = byClass(document.body as unknown as FakeElement, "confirm-overlay");
  ok(overlay, "shared confirm dialog opens");
  const action = findFirst(overlay, (el) => "data-confirm-action" in el.attributes);
  ok(action, "dialog danger action present");
  action.click();
  await settle();

  const enqueue = calls.find((call) => call.path === "/api/deposits/eth-stealth/enqueue-sweep");
  ok(enqueue, "enqueue-sweep endpoint called");
  deepEqual(enqueue.body, { id: "d9" });
  ok(byText(host, "Sweep queued — track it in Move."), "queued feedback shown");

  controller.unmount();
});

test("Receiving shows a persistent stale banner when a refresh fails and recovers on retry", async () => {
  const { host } = install();
  const routes = baseRoutes({
    "GET /api/receiving/overview": { code: "internal", error: "database busy" },
  });
  mockRoutes(routes);
  const { runtime } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  const banners = walkAll(host, (el) => hasClass(el, "attention-item"));
  equal(banners.length, 2, "stale + lock banners exist");
  ok(!banners[0].classList.contains("hidden"), "stale banner visible");
  ok(banners[1].classList.contains("hidden"), "lock banner stays hidden");
  ok(
    findFirst(host, (el) => (el.textContent ?? "").includes("database busy")),
    "failure reason rendered in the persistent banner",
  );
  ok(
    byText(host, "Balance unknown — run Refresh balances."),
    "partial data still renders (allocation card without a balance)",
  );
  ok(byText(host, "No tracked stealth deposits yet"), "empty sections settle into empty states");

  // Recovery: the daemon heals, the operator hits Retry.
  routes["GET /api/receiving/overview"] = sampleOverview();
  const retry = buttonByText(host, "Retry");
  ok(retry, "retry affordance on the banner");
  retry.click();
  await settle();
  ok(banners[0].classList.contains("hidden"), "banner dismissed after a clean refresh");
  ok(byText(host, "0.5 ETH"), "data rendered after retry");

  controller.unmount();
});

test("Receiving guides to unlock when the vault is locked", async () => {
  const { host } = install();
  mockRoutes({
    "GET /api/receiving/overview": { code: "vault_locked", error: "vault is locked" },
    "GET /api/treasury/receive-addresses": { code: "vault_locked", error: "vault is locked" },
    "GET /api/treasury/parties": { code: "vault_locked", error: "vault is locked" },
    "GET /api/deposits/eth-stealth": { code: "vault_locked", error: "vault is locked" },
    "GET /api/profiles/eth-stealth": { code: "vault_locked", error: "vault is locked" },
  });
  const { runtime, navigated } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  ok(byText(host, "Vault locked"), "lock banner shown");
  const unlock = buttonByText(host, "Go to Vault");
  ok(unlock, "unlock guidance action present");
  unlock.click();
  deepEqual(navigated, ["#/vault"]);

  controller.unmount();
});

test("Receiving allocate flow posts the one-time DTO and highlights validation fields", async () => {
  const { host } = install();
  let allocateCalls = 0;
  const calls = mockRoutes(
    baseRoutes({
      "POST /api/treasury/receive-addresses/allocate": () => {
        allocateCalls += 1;
        if (allocateCalls === 1) {
          return {
            code: "validation_failed",
            error: "Check the form",
            fields: [{ field: "sweep_destination_address", message: "invalid address" }],
          };
        }
        return { allocation: { address: "0xfeed" } };
      },
    }),
  );
  const { runtime } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  const allocateForm = findFirst(
    host,
    (el) => el.tagName === "FORM" && buttonByText(el, "Allocate address") != null,
  );
  ok(allocateForm, "allocate form found");
  const inputByLabel = (label: string) =>
    findFirst(
      allocateForm,
      (el) => el.tagName === "INPUT" && el.attributes["aria-label"] === label,
    );
  const wallet = inputByLabel("Wallet profile");
  const purpose = inputByLabel("Purpose");
  const destination = inputByLabel("Sweep destination");
  const threshold = inputByLabel("Sweep threshold in ETH");
  ok(wallet && purpose && destination && threshold, "labeled fields present");
  const checkboxes = walkAll(
    allocateForm,
    (el) => el.tagName === "INPUT" && (el as unknown as { type: string }).type === "checkbox",
  );
  equal(checkboxes.length, 2, "one-time + purge checkboxes");

  wallet.value = "ops";
  purpose.value = "invoices";
  checkboxes[0].checked = true; // one-time
  destination.value = SWEEP_DEST;
  threshold.value = "0.05";
  checkboxes[1].checked = true; // purge after sweep

  allocateForm.dispatchEvent({ type: "submit", preventDefault: () => {} });
  await settle();
  ok(
    destination.classList.contains("input-invalid"),
    "validation_failed highlights the named field",
  );
  equal(destination.attributes["aria-invalid"], "true");
  ok(byText(host, "Check the form"), "server validation message shown");

  allocateForm.dispatchEvent({ type: "submit", preventDefault: () => {} });
  await settle();
  const allocate = calls.find((call) => call.path === "/api/treasury/receive-addresses/allocate");
  ok(allocate, "allocate endpoint called");
  const second = calls.filter(
    (call) => call.path === "/api/treasury/receive-addresses/allocate",
  )[1];
  deepEqual(second.body, {
    wallet_profile: "ops",
    purpose: "invoices",
    one_time: true,
    sweep_destination_address: SWEEP_DEST,
    min_sweep_amount_hex: "0x" + (5n * 10n ** 16n).toString(16),
    purge_after_sweep: true,
  });
  ok(!destination.classList.contains("input-invalid"), "field error cleared on resubmit");
  ok(
    findFirst(host, (el) => (el.textContent ?? "").startsWith("Address allocated:")),
    "success feedback",
  );

  controller.unmount();
});

test("Receiving refetches deposits when queue events arrive via the store", async () => {
  install();
  const calls = mockRoutes(baseRoutes());
  const { runtime, store } = makeRuntime();
  const controller = createReceivingDestination(runtime);
  controller.mount(RECEIVE_ROUTE);
  await settle();

  const before = calls.filter((call) => call.path.startsWith("/api/deposits/eth-stealth")).length;
  store.set("queueEvents", [{ job_id: "j1", state: "confirmed" } as never]);
  await settle();
  const after = calls.filter((call) => call.path.startsWith("/api/deposits/eth-stealth")).length;
  ok(after > before, "queueEvents slice triggers a deposit refetch");

  controller.unmount();
});

test("Receiving unmount restores the legacy card and remounts cleanly", async () => {
  const { host, siblings } = install();
  const stub = document.createElement("div") as unknown as FakeElement;
  stub.className = "legacy-stub";
  stub.textContent = "legacy content";
  host.appendChild(stub);

  mockRoutes(baseRoutes());
  const { runtime } = makeRuntime();
  const controller = createReceivingDestination(runtime);

  controller.mount(RECEIVE_ROUTE);
  await settle();
  equal(host.children.length, 1, "legacy content stashed while mounted");
  ok(hasClass(host.children[0], "dest-recv"));

  controller.unmount();
  equal(host.children.length, 1, "legacy content restored on unmount");
  equal(host.children[0].className, "legacy-stub");
  ok(
    siblings.every((sibling) => !sibling.classList.contains("hidden")),
    "sibling legacy cards restored",
  );

  controller.mount(RECEIVE_ROUTE);
  await settle();
  ok(byText(host, "0.5 ETH"), "remount renders fresh data");
  controller.unmount();
});

test("Receiving lifecycle helpers map statuses, gas notes, blockers, and card merging", () => {
  deepEqual(depositLifecycle(sampleDeposit({ status: "pending" })), {
    completed: 1,
    attention: false,
  });
  deepEqual(depositLifecycle(sampleDeposit({ status: "underfunded" })), {
    completed: 1,
    attention: true,
  });
  deepEqual(depositLifecycle(sampleDeposit({ status: "funded_needs_gas" })), {
    completed: 2,
    attention: true,
  });
  deepEqual(depositLifecycle(sampleDeposit({ status: "funded" })), {
    completed: 3,
    attention: false,
  });
  deepEqual(depositLifecycle(sampleDeposit({ status: "sweep_failed" })), {
    completed: 3,
    attention: true,
  });
  deepEqual(depositLifecycle(sampleDeposit({ status: "sweep_confirmed" })), {
    completed: 4,
    attention: false,
  });

  deepEqual(
    depositGasNotes(
      sampleDeposit({ gas_topup_job_id: "g1", gas_topup_job_state: "sent" }),
    ),
    [
      "The payer was asked to attach 0.01 ETH for the sweep.",
      "A sponsor gas top-up is sent.",
    ],
  );

  equal(oneTimeBlockerText("below_threshold"), "below the sweep threshold");
  equal(oneTimeBlockerText("cross_party_linkage"), "blocked: the shared destination would link payers");
  equal(oneTimeBlockerText("some_new_blocker"), "some new blocker");

  const cards = buildAddressCards(sampleOverview(), [sampleAllocation()], [sampleParty()]);
  equal(cards.length, 1, "allocation and overview item merge by address");
  equal(cards[0].balanceEth, "0.5");
  equal(cards[0].oneTime, true);
  equal(cards[0].counterpartyName, "Acme");
  equal(cards[0].sourceLabel, "HD address");
});
