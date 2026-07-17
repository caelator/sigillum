import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  createMoveDestination,
  blockerLabel,
  describeQueueJob,
  destinationTrust,
  formatWeiHexAsEth,
  futureTime,
  groupQueueJobs,
  humanizeQueueError,
  parseEthToWeiHex,
  parseGweiToWeiHex,
  planNativeTotalWeiHex,
  queueJobCanProcess,
  queueProcessSummary,
  relativeTime,
  shortAddress,
  simulationBadge,
  stepExecutionEligible,
  stepPlainLanguage,
  treasuryPolicySummary,
} from "../src/destinations/Move";
import { ApiError, type DaemonApi } from "../src/core/api";
import { createCoreStore } from "../src/core/state";
import { createRouter, type Route } from "../src/core/router";
import type { CoreRuntime } from "../src/core/live";
import type {
  ConsolidationPlan,
  ConsolidationPlanStep,
  QueueJob,
  TreasuryPolicy,
} from "../src/contracts";
import { installDom, FakeElement, type FakeNode } from "./dom-fixture";
import { MemoryHashSource, mockFetchJson, tick } from "./core-helpers";

// ── Fixtures ─────────────────────────────────────────────────────────────

const ADDRESS_FROM = "0x71C7656EC7ab88b098defB751B7401B5f6d8976F";
const ADDRESS_DEST = "0xAAAA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
const ADDRESS_FOREIGN = "0xBBBB39b223FE8D0A0e5C4F27eAD9083C756Cc3";
const WEI_0_42_ETH = "0x" + (42n * 10n ** 16n).toString(16); // 0.42 ETH

const nowSecs = (): number => Math.floor(Date.now() / 1000);

function makeStep(overrides: Partial<ConsolidationPlanStep> = {}): ConsolidationPlanStep {
  return {
    id: "step-1",
    sequence: 1,
    action: "sweep_native",
    status: "approved",
    wallet_family: "stealth",
    wallet_profile: "main",
    provider_profile: "local",
    chain_id: 1,
    address: ADDRESS_FROM,
    derivation_path: "m/44'/60'/0'/0/0",
    asset_kind: "native",
    amount_hex: WEI_0_42_ETH,
    destination_address: ADDRESS_DEST,
    signer_status: "available",
    simulation_status: "passed",
    simulation_evidence: [
      "simulated_at_unix=" + String(nowSecs()),
      "max_fee_per_gas_hex=0x4a817c800", // 20 gwei
      "transaction_gas_limit=21000",
    ],
    risk_level: "low",
    blockers: [],
    auto_eligible: true,
    approved: true,
    ...overrides,
  } as ConsolidationPlanStep;
}

function makePlan(
  steps: ConsolidationPlanStep[],
  overrides: Partial<ConsolidationPlan> = {},
): ConsolidationPlan {
  return {
    id: "plan-abc123",
    status: "approved",
    chain_id: 1,
    created_at_unix: nowSecs() - 3600,
    updated_at_unix: nowSecs() - 60,
    summary: {
      total_steps: steps.length,
      blocked_steps: steps.filter((step) => step.status === "blocked").length,
      review_required_steps: steps.filter((step) => step.status === "review_required")
        .length,
      approved_steps: steps.filter((step) => step.status === "approved").length,
      executable_steps: steps.length,
      value_items: steps.length,
    },
    steps,
    ...overrides,
  } as ConsolidationPlan;
}

function makePolicy(overrides: Partial<TreasuryPolicy> = {}): TreasuryPolicy {
  return {
    enabled: true,
    allowed_destinations: [
      { address: ADDRESS_DEST, label: "Treasury vault (cold)" },
    ],
    require_simulation: true,
    block_cross_party_linkage: true,
    allow_plan_execution: true,
    allow_sweep_execution: true,
    allow_revoke_execution: false,
    allow_exit_execution: false,
    allow_claim_execution: false,
    allow_gas_topups: false,
    simulation_freshness_secs: 900,
    created_at_unix: nowSecs() - 86_400,
    updated_at_unix: nowSecs() - 300,
    ...overrides,
  } as TreasuryPolicy;
}

function makeQueueJob(overrides: Partial<QueueJob> = {}): QueueJob {
  return {
    id: "job-1",
    kind: "plan_step_execution",
    state: "queued",
    attempts: 0,
    created_at_unix: nowSecs() - 600,
    updated_at_unix: nowSecs() - 30,
    ...overrides,
  } as QueueJob;
}

// ── Fake-DOM helpers ─────────────────────────────────────────────────────

function textOf(node: FakeNode | null | undefined): string {
  if (!node) return "";
  let text = node.textContent || "";
  const children = (node as FakeElement).childNodes;
  if (children) {
    for (const child of children) {
      const childText = textOf(child);
      if (childText) text += (text ? " " : "") + childText;
    }
  }
  return text.replace(/\s+/g, " ").trim();
}

function findAll(
  root: FakeElement,
  pred: (element: FakeElement) => boolean,
): FakeElement[] {
  const out: FakeElement[] = [];
  const walk = (node: FakeNode): void => {
    if (!(node instanceof FakeElement)) return;
    if (pred(node)) out.push(node);
    node.childNodes.forEach(walk);
  };
  walk(root);
  return out;
}

function findByRegion(root: FakeElement, region: string): FakeElement | null {
  return (
    findAll(
      root,
      (element) => element.attributes["data-move-region"] === region,
    )[0] ?? null
  );
}

function findButton(root: FakeElement, text: string): FakeElement | null {
  return (
    findAll(
      root,
      (element) => element.tagName === "BUTTON" && textOf(element) === text,
    )[0] ?? null
  );
}

function findLink(root: FakeElement, href: string): FakeElement | null {
  return (
    findAll(
      root,
      (element) => element.tagName === "A" && element.attributes.href === href,
    )[0] ?? null
  );
}

// ── Mock runtime ─────────────────────────────────────────────────────────

function fakeApi(overrides: Partial<DaemonApi> = {}): DaemonApi {
  const base = {
    getStatus: async () => ({ initialized: true, locked: false, unlocked_compartments: [] }),
    listOperations: async () => ({ operations: [] }),
    getOperation: async (id: string) => ({
      operation: {
        id,
        kind: "queue_process",
        state: "running",
        progress: { processed: 0 },
        created_at_unix: 1,
        updated_at_unix: 1,
      },
    }),
    cancelOperation: async (id: string) => ({
      status: "ok",
      operation: {
        id,
        kind: "queue_process",
        state: "cancel_requested",
        progress: { processed: 0 },
        created_at_unix: 1,
        updated_at_unix: 1,
      },
    }),
    listQueueJobs: async () => ({ jobs: [], pagination: null }),
    pauseQueue: async () => ({ status: "ok", execution_paused: true }),
    resumeQueue: async () => ({ status: "ok", execution_paused: false }),
    processQueue: async () => ({
      processed: 0,
      succeeded: 0,
      blocked: 0,
      retrying: 0,
      operator_action_required: 0,
      failed: 0,
      confirmed: 0,
      jobs: [],
    }),
    listPlans: async () => ({ plans: [], pagination: null }),
    getTreasuryOverview: async () => ({}),
    getTreasuryPolicy: async () => ({ policy: null }),
    updateTreasuryPolicy: async () => ({ status: "ok", policy: makePolicy() }),
    getReceivingOverview: async () => ({}),
    listDeposits: async () => ({ deposits: [], pagination: null }),
    listInventoryWallets: async () => ({ wallets: [], pagination: null }),
    listAudit: async () => ({ events: [] }),
    runSelfCheck: async () => ({}),
    getDiagnostics: async () => ({}),
  };
  return { ...base, ...overrides } as unknown as DaemonApi;
}

function makeRuntime(
  api: DaemonApi,
  hash = "#/move",
): { runtime: CoreRuntime; source: MemoryHashSource } {
  const source = new MemoryHashSource();
  source.hash = hash;
  const bootRoute: Route = {
    destination: "move",
    path: [],
    params: {},
    hash: "#/move",
  };
  const store = createCoreStore(bootRoute);
  const router = createRouter({
    source,
    onRoute: (route) => store.set("route", route),
  });
  router.start();
  const runtime = {
    store,
    api,
    router,
    adapter: {
      notifyLegacySection: () => {},
      handleRoute: () => {},
      start: () => {},
      controller: () => undefined,
    },
    events: { start: () => {}, stop: () => {}, transport: () => "off" },
    notifyLegacySection: () => {},
    stop: () => {},
  } as unknown as CoreRuntime;
  return { runtime, source };
}

const MOVE_CARDS = ["plansCard", "queueCard", "policyCard", "maintenanceCard"];

function installMoveDom(legacyHtml = true) {
  const dom = installDom(MOVE_CARDS);
  if (legacyHtml) {
    for (const id of MOVE_CARDS) {
      dom.el(id).innerHTML = "<p>legacy " + id + "</p>";
    }
  }
  mockFetchJson(() => ({})); // thin wrappers default to empty payloads
  return dom;
}

async function flush(): Promise<void> {
  await tick();
  await tick();
  await tick();
}

// ── Pure helpers ─────────────────────────────────────────────────────────

test("Move stepExecutionEligible mirrors the daemon gates", () => {
  const policy = makePolicy();
  const step = makeStep();
  ok(stepExecutionEligible(step, policy, nowSecs()));
  ok(!stepExecutionEligible(step, makePolicy({ enabled: false }), nowSecs()));
  ok(
    !stepExecutionEligible(
      step,
      makePolicy({ execution_paused: true }),
      nowSecs(),
    ),
  );
  ok(
    !stepExecutionEligible(
      step,
      makePolicy({ allow_sweep_execution: false }),
      nowSecs(),
    ),
  );
  ok(
    !stepExecutionEligible(
      step,
      makePolicy({ allow_plan_execution: false }),
      nowSecs(),
    ),
  );
  ok(
    !stepExecutionEligible(
      makeStep({ simulation_status: "not_run" }),
      policy,
      nowSecs(),
    ),
  );
  ok(
    !stepExecutionEligible(
      makeStep({ blockers: ["missing_destination"] }),
      policy,
      nowSecs(),
    ),
  );
  ok(
    !stepExecutionEligible(makeStep({ queued_job_id: "job-9" }), policy, nowSecs()),
  );
  // Stale simulation (older than the freshness window) is not eligible.
  const stale = makeStep({
    simulation_evidence: ["simulated_at_unix=" + String(nowSecs() - 2000)],
  });
  ok(!stepExecutionEligible(stale, policy, nowSecs()));
  // review_asset is never executable (no family gate).
  ok(
    !stepExecutionEligible(
      makeStep({ action: "review_asset" }),
      policy,
      nowSecs(),
    ),
  );
});

test("Move stepPlainLanguage reads like a hardware-wallet line", () => {
  const policy = makePolicy();
  const sentence = stepPlainLanguage(makeStep(), { symbol: "ETH", policy });
  equal(
    sentence,
    "Sweep 0.42 ETH from 0x71C7…976F → Treasury vault (cold)",
  );
  const foreign = stepPlainLanguage(
    makeStep({ destination_address: ADDRESS_FOREIGN }),
    { symbol: "ETH", policy },
  );
  ok(foreign.includes("(foreign)"), foreign);
  const topup = stepPlainLanguage(
    makeStep({ action: "fund_gas", destination_address: ADDRESS_FOREIGN }),
    { symbol: "ETH", policy },
  );
  ok(topup.startsWith("Top up 0xBBBB…6Cc3 with 0.42 ETH of gas from sponsor"), topup);
});

test("Move simulationBadge reports fresh, stale, missing, and failed", () => {
  const fresh = simulationBadge(makeStep(), 900, nowSecs());
  equal(fresh.kind, "fresh");
  const stale = simulationBadge(
    makeStep({
      simulation_evidence: ["simulated_at_unix=" + String(nowSecs() - 5000)],
    }),
    900,
    nowSecs(),
  );
  equal(stale.kind, "stale");
  equal(stale.tier, "review");
  const missing = simulationBadge(
    makeStep({ simulation_status: "not_run", simulation_evidence: [] }),
    900,
    nowSecs(),
  );
  equal(missing.kind, "missing");
  const failed = simulationBadge(
    makeStep({ simulation_status: "failed", simulation_evidence: [] }),
    900,
    nowSecs(),
  );
  equal(failed.kind, "failed");
  equal(failed.tier, "danger");
});

test("Move destinationTrust grades allowlisted, party, foreign, unset", () => {
  const policy = makePolicy();
  const parties = [
    {
      id: "p1",
      name: "Acme",
      sweep_destination_address: ADDRESS_FOREIGN,
      created_at_unix: 1,
    },
  ];
  equal(destinationTrust(ADDRESS_DEST, policy, []).kind, "allowlisted");
  equal(
    destinationTrust(ADDRESS_DEST, policy, []).label,
    "Treasury vault (cold)",
  );
  const party = destinationTrust(ADDRESS_FOREIGN, policy, parties);
  equal(party.kind, "party");
  const foreign = destinationTrust(ADDRESS_FOREIGN, policy, []);
  equal(foreign.kind, "foreign");
  equal(foreign.tier, "review");
  equal(destinationTrust(null, policy, []).kind, "unset");
  equal(destinationTrust(null, policy, []).tier, "danger");
});

test("Move treasuryPolicySummary ports the Phase 0 plain-English summary", () => {
  const disabled = treasuryPolicySummary(makePolicy({ enabled: false }));
  ok(disabled[0].includes("nothing may execute"), disabled[0]);
  const on = treasuryPolicySummary(makePolicy());
  ok(on.some((line) => line === "Plans may execute sweeps."), on.join(" "));
  ok(
    on.some((line) =>
      line.includes("sweep gate also covers stealth deposit sweeps"),
    ),
    on.join(" "),
  );
  ok(
    on.some((line) => line.includes("Cross-party linkage blocking is on")),
    on.join(" "),
  );
});

test("Move amount parsers round-trip ETH and gwei", () => {
  equal(parseEthToWeiHex("0.42"), WEI_0_42_ETH);
  equal(parseEthToWeiHex("1"), "0xde0b6b3a7640000");
  equal(parseEthToWeiHex("nope"), null);
  equal(parseEthToWeiHex("0.1234567890123456789"), null); // >18 decimals
  equal(formatWeiHexAsEth(WEI_0_42_ETH), "0.42");
  equal(parseGweiToWeiHex("20"), "0x4a817c800");
  equal(parseGweiToWeiHex("0.0000000001"), null); // >9 decimals
});

test("Move queue helpers group, label, and gate jobs", () => {
  const actionRequired = makeQueueJob({
    id: "j1",
    state: "operator_action_required",
  });
  const queued = makeQueueJob({ id: "j2", state: "queued" });
  const failed = makeQueueJob({ id: "j3", state: "failed_terminal" });
  const groups = groupQueueJobs([queued, failed, actionRequired]);
  equal(groups[0].state, "operator_action_required");
  equal(groups.length, 3);

  ok(!queueJobCanProcess(actionRequired));
  ok(!queueJobCanProcess(failed));
  ok(queueJobCanProcess(queued));
  // W7.4: `sent` plan-step jobs can still be receipt-polled via Process.
  ok(
    queueJobCanProcess(
      makeQueueJob({ state: "sent", kind: "plan_step_execution" }),
    ),
  );
  ok(!queueJobCanProcess(makeQueueJob({ state: "sent", kind: "eth_stealth_transfer" })));

  const description = describeQueueJob(
    makeQueueJob({
      action: "sweep_native",
      amount_hex: WEI_0_42_ETH,
      source_address: ADDRESS_FROM,
      destination_address: ADDRESS_DEST,
    }),
  );
  equal(
    description,
    "Sweep 0.42 ETH from 0x71C7…976F → 0xAAAA…6Cc2",
  );
});

test("Move misc formatters stay human and truthful", () => {
  equal(shortAddress(ADDRESS_FROM), "0x71C7…976F");
  equal(relativeTime(nowSecs() - 300, nowSecs()), "5m ago");
  equal(relativeTime(nowSecs() - 7200, nowSecs()), "2h ago");
  equal(futureTime(nowSecs() + 300, nowSecs()), "in 5m");
  equal(planNativeTotalWeiHex(makePlan([makeStep(), makeStep()])), "0x" + (84n * 10n ** 16n).toString(16));
  equal(blockerLabel("cross_party_linkage"), "Destination shared with another payer");
  ok(humanizeQueueError("insufficient_gas: balance 0").startsWith("Not enough gas"));
  const summary = queueProcessSummary({
    processed: 3,
    succeeded: 2,
    operator_action_required: 1,
    failures_by_cause: { insufficient_gas: 1 },
  });
  ok(summary.includes("Processed 3 job(s): 2 succeeded, 1 need your action."), summary);
  ok(summary.includes("1 gas shortfalls"), summary);
});

// ── Controller integration (fake DOM + mock runtime) ────────────────────

test("Move mount renders plans, queue groups, and the policy summary", async () => {
  const dom = installMoveDom();
  const plan = makePlan([makeStep()]);
  const api = fakeApi({
    listPlans: async () => ({
      plans: [plan],
      pagination: { total: 1, limit: 20, offset: 0, has_more: false },
    }),
    listQueueJobs: async () => ({
      jobs: [
        makeQueueJob({ id: "j-q", state: "queued" }),
        makeQueueJob({ id: "j-a", state: "operator_action_required" }),
      ],
      pagination: { total: 2, limit: 25, offset: 0, has_more: false },
    }),
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const plansCard = dom.el("plansCard");
  const plansText = textOf(plansCard);
  ok(plansText.includes("Plan plan-abc123"), plansText);
  ok(plansText.includes("1 steps"), plansText);
  ok(plansText.includes("1 ready to enqueue"), plansText);
  const review = findLink(plansCard, "#/move/plan/plan-abc123");
  ok(review, "Review deep link present");

  const queueText = textOf(dom.el("queueCard"));
  ok(queueText.includes("Needs your action (1)"), queueText);
  ok(
    queueText.indexOf("Needs your action (1)") < queueText.indexOf("Queued (1)"),
    "operator_action_required group floats to the top",
  );

  const policyText = textOf(dom.el("policyCard"));
  ok(policyText.includes("Current policy"), policyText);
  ok(policyText.includes("Plans may execute sweeps."), policyText);

  controller.unmount();
});

test("Move shows a persistent stale banner when a refresh fails", async () => {
  const dom = installMoveDom();
  const plan = makePlan([makeStep()]);
  let failPlans = false;
  const api = fakeApi({
    listPlans: async () => {
      if (failPlans) throw new ApiError({ code: "unavailable", error: "boom" });
      return { plans: [plan], pagination: null };
    },
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();
  ok(!textOf(dom.el("plansCard")).includes("out of date"));

  failPlans = true;
  runtime.store.set("resync", runtime.store.get("resync") + 1);
  await flush();

  const text = textOf(dom.el("plansCard"));
  ok(text.includes("may be out of date"), text);
  ok(text.includes("Plan plan-abc123"), "earlier data stays on screen");
  controller.unmount();
});

test("Move renders vault_locked guidance with a path to unlock", async () => {
  const dom = installMoveDom();
  const locked = new ApiError({ code: "vault_locked", error: "vault is locked" });
  const api = fakeApi({
    listPlans: async () => {
      throw locked;
    },
    listQueueJobs: async () => {
      throw locked;
    },
    getTreasuryPolicy: async () => {
      throw locked;
    },
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const text = textOf(dom.el("plansCard"));
  ok(text.includes("The vault is locked"), text);
  ok(findLink(dom.el("plansCard"), "#/vault"), "unlock deep link present");
  controller.unmount();
});

test("Move plan review renders step cards and conceals the aux cards", async () => {
  const dom = installMoveDom();
  const eligible = makeStep();
  const blocked = makeStep({
    id: "step-2",
    status: "blocked",
    approved: false,
    simulation_status: "not_run",
    simulation_evidence: [],
    blockers: ["missing_destination"],
    destination_address: null,
  });
  const plan = makePlan([eligible, blocked], {
    linkage_findings: ["payers A and B share a destination"],
  });
  const api = fakeApi({
    listPlans: async () => ({ plans: [plan], pagination: null }),
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
  });
  const { runtime } = makeRuntime(api, "#/move/plan/plan-abc123");
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const plansCard = dom.el("plansCard");
  const text = textOf(plansCard);
  ok(
    text.includes("Sweep 0.42 ETH from 0x71C7…976F → Treasury vault (cold)"),
    text,
  );
  ok(text.includes("2 steps · moving up to 0.84 ETH"), text);
  ok(text.includes("Blocked: No destination set"), text);
  ok(text.includes("Privacy: this plan would link payers"), text);
  ok(text.includes("fee ≤ 0.00042 ETH · 20 gwei"), text);
  ok(findLink(plansCard, "#/move"), "back-to-plans link present");
  ok(
    dom.el("queueCard").classList.contains("move-concealed"),
    "queue card concealed during review",
  );

  const enqueue = findByRegion(plansCard, "enqueue-plan") as FakeElement & {
    disabled: boolean;
  };
  ok(enqueue, "enqueue button rendered");
  equal(enqueue.disabled, false);
  const hint = textOf(
    enqueue.parentNode as FakeElement,
  );
  ok(hint.includes("1 of 2 steps eligible"), hint);
  controller.unmount();
});

test("Move enqueue keeps the probe → phrase → enqueue server contract", async () => {
  const dom = installMoveDom();
  const plan = makePlan([makeStep()]);
  const PHRASE = "EXECUTE 1 PLAN STEPS TOTAL 420000000000000000 WEI";
  const enqueueCalls: { plan_id?: string; confirmation?: string }[] = [];
  mockFetchJson((path: string, init: unknown) => {
    if (path === "/api/plans/enqueue-plan") {
      const body = JSON.parse(
        (init as { body?: string }).body || "{}",
      ) as { plan_id?: string; confirmation?: string };
      enqueueCalls.push(body);
      if (!body.confirmation) {
        return {
          code: "bad_request",
          error:
            'confirmation_mismatch: type the exact phrase "' +
            PHRASE +
            '" to enqueue 1 steps',
          action: PHRASE,
        };
      }
      return {
        status: "ok",
        enqueued: [{ plan_id: body.plan_id, step_id: "step-1", job_id: "job-1" }],
        skipped: [],
      };
    }
    return {};
  });
  const api = fakeApi({
    listPlans: async () => ({ plans: [plan], pagination: null }),
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
  });
  const { runtime } = makeRuntime(api, "#/move/plan/plan-abc123");
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const enqueue = findByRegion(dom.el("plansCard"), "enqueue-plan");
  ok(enqueue, "enqueue button present");
  enqueue.click();
  await flush();

  // Probe fired with an empty confirmation; the typed dialog carries the
  // server-computed phrase and stays disabled until it is typed.
  equal(enqueueCalls.length, 1);
  equal(enqueueCalls[0].confirmation, "");
  const overlay = findAll(dom.document.body, (element) =>
    Boolean(element.attributes["data-confirm-overlay"]),
  )[0];
  ok(overlay, "typed confirm dialog open");
  const phrase = findAll(overlay, (element) =>
    "data-confirm-phrase" in element.attributes,
  )[0];
  equal(textOf(phrase), PHRASE);
  const input = findAll(overlay, (element) =>
    "data-confirm-input" in element.attributes,
  )[0] as FakeElement & { value: string };
  const action = findAll(overlay, (element) =>
    "data-confirm-action" in element.attributes,
  )[0] as FakeElement & { disabled: boolean };
  equal(action.disabled, true);

  input.value = PHRASE;
  input.dispatchEvent({ type: "input", target: input });
  equal(action.disabled, false);
  action.click();
  await flush();

  equal(enqueueCalls.length, 2);
  equal(enqueueCalls[1].confirmation, PHRASE);
  equal(enqueueCalls[1].plan_id, "plan-abc123");
  ok(
    textOf(dom.el("plansCard")).includes("Enqueued 1 step(s)"),
    "success notice after enqueue",
  );
  controller.unmount();
});

test("Move generate form submits the legacy request DTO on Enter-submit", async () => {
  const dom = installMoveDom();
  const generateCalls: Record<string, unknown>[] = [];
  mockFetchJson((path: string, init: unknown) => {
    if (path === "/api/plans/consolidation/generate") {
      generateCalls.push(
        JSON.parse((init as { body?: string }).body || "{}") as Record<
          string,
          unknown
        >,
      );
      return { status: "ok", plan_id: "plan-new" };
    }
    return {};
  });
  const api = fakeApi({
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const plansCard = dom.el("plansCard");
  const destination = findByRegion(plansCard, "generate-destination") as FakeElement & {
    value: string;
  };
  ok(destination, "destination input present");
  destination.value = "0xDEST";
  const form = findAll(plansCard, (element) => element.tagName === "FORM")[0];
  ok(form, "generate form present");
  let prevented = false;
  form.dispatchEvent({
    type: "submit",
    target: form,
    preventDefault: () => {
      prevented = true;
    },
  });
  await flush();

  equal(generateCalls.length, 1);
  equal(generateCalls[0].destination_address, "0xDEST");
  equal(generateCalls[0].routing_strategy, "single");
  equal(generateCalls[0].include_watch_only, true);
  equal(generateCalls[0].auto_queue_low_risk, false);
  ok(prevented, "default submit navigation prevented");
  controller.unmount();
});

test("Move unmount restores the legacy card markup untouched", async () => {
  const dom = installMoveDom();
  const saved = MOVE_CARDS.map((id) => dom.el(id).innerHTML);
  const api = fakeApi();
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();
  // Mount replaced the legacy content with the rebuilt destination.
  ok(
    textOf(dom.el("plansCard")).includes("Review consolidation plans"),
    "mounted content present",
  );
  controller.unmount();
  MOVE_CARDS.forEach((id, index) => {
    equal(dom.el(id).innerHTML, saved[index], id + " markup restored");
    equal(textOf(dom.el(id)), "", id + " rebuilt children cleared");
    ok(!dom.el(id).classList.contains("move-concealed"), id + " revealed");
    equal(dom.el(id).getAttribute("tabindex"), null);
  });
});

test("Move policy save posts the Phase 0 DTO shape after a danger confirm", async () => {
  const dom = installMoveDom();
  const updates: Record<string, unknown>[] = [];
  const api = fakeApi({
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
    updateTreasuryPolicy: async (body) => {
      updates.push(body as unknown as Record<string, unknown>);
      return { status: "ok", policy: makePolicy() };
    },
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const policyCard = dom.el("policyCard");
  const maxStep = findByRegion(policyCard, "policy-max-step") as FakeElement & {
    value: string;
  };
  maxStep.value = "0.5";
  const form = findAll(policyCard, (element) => element.tagName === "FORM")[0];
  form.dispatchEvent({
    type: "submit",
    target: form,
    preventDefault: () => {},
  });
  await flush();

  // Danger-tier confirm guards the save.
  const overlay = findAll(dom.document.body, (element) =>
    Boolean(element.attributes["data-confirm-overlay"]),
  )[0];
  ok(overlay, "confirm dialog open");
  const action = findAll(overlay, (element) =>
    "data-confirm-action" in element.attributes,
  )[0];
  action.click();
  await flush();

  equal(updates.length, 1);
  const body = updates[0];
  equal(body.enabled, true);
  equal(body.max_step_native_wei_hex, "0x6f05b59d3b20000"); // 0.5 ETH
  equal(body.allow_plan_execution, true);
  equal(body.allow_sweep_execution, true);
  equal(body.block_cross_party_linkage, true);
  ok(
    textOf(policyCard).includes("Treasury policy saved."),
    "success notice after save",
  );
  controller.unmount();
});

test("Move policy editor flags client-side validation before any request", async () => {
  const dom = installMoveDom();
  let updates = 0;
  const api = fakeApi({
    getTreasuryPolicy: async () => ({ policy: makePolicy() }),
    updateTreasuryPolicy: async () => {
      updates += 1;
      return { status: "ok", policy: makePolicy() };
    },
  });
  const { runtime } = makeRuntime(api);
  const controller = createMoveDestination(runtime);
  controller.mount(runtime.store.get("route"));
  await flush();

  const policyCard = dom.el("policyCard");
  const maxStep = findByRegion(policyCard, "policy-max-step") as FakeElement & {
    value: string;
  };
  maxStep.value = "not-a-number";
  const form = findAll(policyCard, (element) => element.tagName === "FORM")[0];
  form.dispatchEvent({
    type: "submit",
    target: form,
    preventDefault: () => {},
  });
  await flush();

  equal(updates, 0);
  ok(maxStep.classList.contains("input-invalid"), "field marked invalid");
  ok(
    textOf(findByRegion(policyCard, "policy-field-errors")).includes(
      "decimal ETH amount",
    ),
    "inline message shown",
  );
  controller.unmount();
});
