import { deepEqual, equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  clearSessionToken,
  readSessionToken,
  requestWithSession,
  writeSessionToken,
} from "../src/api/session";
import { dispatchDataAction } from "../src/actions/dispatcher";
import {
  buildInventoryReport,
  createInventoryActions,
  parseWatchAddressProbes,
} from "../src/views/inventory";
import { createOperationsActions } from "../src/views/operations";
import { createSessionActions } from "../src/views/session";
import { createSetupWizard } from "../src/views/setup";
import { createShellRenderer } from "../src/views/shell";
import {
  createTreasuryActions,
  formatWeiHexAsEth,
  parseEthToWeiHex,
  parseTreasuryDestinationLines,
} from "../src/views/treasury";
import type {
  TreasuryOverviewResponse,
  TreasuryPolicy,
  TreasuryReceiveAllocation,
} from "../src/contracts";
import { installDom } from "./dom-fixture";

test("shell renderer applies setup, locked, and unlocked DOM state", () => {
  const dom = installDom([
    "compartmentBadge",
    "setupCard",
    "authCard",
    "lockForm",
    "authRecovery",
    "compSwitcher",
    "authTitle",
    "authLead",
    "unlockPassphrase",
    "unlockFido2",
    "unlockTabs",
    "compartmentCard",
    "pushCard",
    "guideCard",
    "profilesCard",
    "xpubCard",
    "treasuryCard",
    "inventoryCard",
    "depositsCard",
    "queueCard",
    "maintenanceCard",
    "backupCard",
    "auditCard",
    "diagCard",
    "opCard",
  ]);
  let mode = "";
  const calls: string[] = [];
  const renderer = createShellRenderer({
    operatorCardIds: ["opCard"],
    setUiMode: (next) => {
      mode = next;
    },
    setCardsHidden: (ids, hidden) =>
      ids.forEach((id) => dom.el(id).classList.toggle("hidden", hidden)),
    setStatusBadge: (_className, label) => calls.push("status:" + label),
    setSecretsAccess: (unlocked) => calls.push("secrets:" + String(unlocked)),
    resetVaultCounts: () => calls.push("counts"),
    setUnlockGuidance: (next) => calls.push("guidance:" + next),
    updateHeroState: (next) => calls.push("hero:" + next),
    updateWizardChrome: (id) => calls.push("wizard:" + id),
    resetSetupWizard: () => calls.push("wizard:reset"),
    renderCompartmentSwitcher: () => calls.push("switcher"),
    renderActiveCompartment: () => calls.push("active"),
    buildPushSelectors: () => calls.push("push-selectors"),
  });

  renderer.applySetupUi();
  equal(mode, "setup");
  equal(document.body.dataset.mode, "setup");
  equal(dom.el("setupCard").classList.contains("hidden"), false);
  equal(dom.el("authCard").classList.contains("hidden"), true);

  renderer.applyLockedUi();
  equal(mode, "locked");
  equal(document.body.dataset.mode, "locked");
  equal(dom.el("lockForm").classList.contains("hidden"), true);
  equal(dom.el("authRecovery").classList.contains("hidden"), false);

  renderer.applyUnlockedUi({ compartment_id: 1 }, [
    { id: 1, label: "daily" },
    { id: 2, label: "secure" },
  ]);
  equal(mode, "unlocked");
  equal(document.body.dataset.mode, "unlocked");
  equal(dom.el("pushCard").classList.contains("hidden"), false);
  equal(dom.el("treasuryCard").classList.contains("hidden"), false);
  ok(calls.includes("push-selectors"));
});

test("session requests persist fresh tokens and clear stale tokens on 401", async () => {
  installDom();
  clearSessionToken();
  writeSessionToken("old-token");
  let captured: any = null;
  (globalThis as any).fetch = async (_path: string, init: any) => {
    captured = init;
    return {
      status: 200,
      json: async () => ({ ok: true, session_token: "new-token" }),
    };
  };

  await requestWithSession("POST", "/api/example", { ok: true });
  equal(captured.headers.Authorization, "Bearer old-token");
  equal(captured.body, '{"ok":true}');
  equal(readSessionToken(), "new-token");

  (globalThis as any).fetch = async () => ({
    status: 401,
    json: async () => ({ error: "expired" }),
  });
  await requestWithSession("GET", "/api/expired");
  equal(readSessionToken(), null);
});

test("session actions drive unlock, lock, and browser logout workflow", async () => {
  const dom = installDom(["passphrase"]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  let refreshCount = 0;
  let confirmResult = false;
  const actions = createSessionActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      const requestBody = body as { passphrase?: string } | undefined;
      if (path === "/api/unlock" && requestBody?.passphrase === "already") {
        return { error: "Vault is already unlocked." };
      }
      if (path === "/api/unlock") {
        return {
          status: "unlocked",
          session_token: "fresh-session",
          unlocked_compartments: [{ id: 0, label: "browser-smoke" }],
        };
      }
      return { status: "ok" };
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => {
      refreshCount += 1;
    },
    confirm: () => confirmResult,
  });

  dom.el("passphrase").value = "browser-smoke-passphrase-123";
  await actions.unlock();
  deepEqual(calls.pop(), {
    method: "POST",
    path: "/api/unlock",
    body: { passphrase: "browser-smoke-passphrase-123" },
  });
  equal(dom.el("passphrase").value, "");
  deepEqual(toasts.pop(), { message: "Unlocked: browser-smoke", type: undefined });
  equal(refreshCount, 1);

  dom.el("passphrase").value = "already";
  await actions.unlock();
  equal(dom.el("passphrase").value, "already");
  deepEqual(toasts.pop(), {
    message: "Session already active. Refreshing workspace...",
    type: undefined,
  });
  equal(refreshCount, 2);

  writeSessionToken("still-active");
  await actions.lock();
  equal(readSessionToken(), "still-active");
  ok(!calls.some((call) => call.path === "/api/lock"));

  confirmResult = true;
  await actions.lock();
  deepEqual(calls.pop(), { method: "POST", path: "/api/lock", body: undefined });
  equal(readSessionToken(), null);
  deepEqual(toasts.pop(), { message: "All compartments locked", type: undefined });
  equal(refreshCount, 3);

  writeSessionToken("browser-only");
  await actions.logoutSession();
  deepEqual(calls.pop(), {
    method: "POST",
    path: "/api/session/revoke",
    body: undefined,
  });
  equal(readSessionToken(), null);
  deepEqual(toasts.pop(), { message: "Session logged out", type: undefined });
  equal(refreshCount, 4);
});

test("setup wizard passphrase path validates and initializes a local vault", async () => {
  const dom = installDom([
    "wizStep0",
    "wizStepPassphrase",
    "wizStepDone",
    "wizStagePill",
    "wizStageTitle",
    "wizStageSummary",
    "wizChecklist",
    "wizPLabel",
    "wizPassphrase",
    "wizPassphraseConfirm",
    "wizDoneMsg",
    "wizDoneDetail",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  let refreshed = false;
  const originalSetTimeout = globalThis.setTimeout;
  (globalThis as any).setTimeout = (handler: TimerHandler) => {
    if (typeof handler === "function") handler();
    return 0;
  };

  try {
    const wizard = createSetupWizard({
      api: async (method, path, body) => {
        calls.push({ method, path, body });
        return {
          status: "initialized",
          compartment_id: 0,
          session_token: "session-1",
        };
      },
      toast: (message, type) => toasts.push({ message, type }),
      refresh: () => {
        refreshed = true;
      },
      submitNewFido2Pin: async () => undefined,
      friendlyFidoError: (message) => String(message),
    });

    wizard.wizPreset("passphrase");
    equal(dom.el("wizStepPassphrase").classList.contains("active"), true);
    equal(dom.el("wizStageTitle").textContent, "Create your first local compartment");

    dom.el("wizPLabel").value = "browser-smoke";
    dom.el("wizPassphrase").value = "short";
    dom.el("wizPassphraseConfirm").value = "short";
    await wizard.wizInitPassphrase();
    equal(calls.length, 0);
    deepEqual(toasts.pop(), { message: "Min 8 characters", type: "error" });

    dom.el("wizPassphrase").value = "browser-smoke-passphrase-123";
    dom.el("wizPassphraseConfirm").value = "browser-smoke-passphrase-456";
    await wizard.wizInitPassphrase();
    equal(calls.length, 0);
    deepEqual(toasts.pop(), { message: "Passphrases do not match", type: "error" });

    dom.el("wizPassphraseConfirm").value = "browser-smoke-passphrase-123";
    await wizard.wizInitPassphrase();

    deepEqual(calls, [
      {
        method: "POST",
        path: "/api/compartment/init",
        body: {
          id: 0,
          label: "browser-smoke",
          threshold: 1,
          passphrase: "browser-smoke-passphrase-123",
        },
      },
    ]);
    equal(dom.el("wizDoneMsg").textContent, "Vault Created");
    equal(
      dom.el("wizDoneDetail").textContent,
      'Compartment "browser-smoke" initialized. You are unlocked.',
    );
    equal(dom.el("wizStepDone").classList.contains("active"), true);
    equal(refreshed, true);
  } finally {
    globalThis.setTimeout = originalSetTimeout;
  }
});

test("queue and inventory renderers produce reviewable DOM summaries", () => {
  const dom = installDom([
    "queueList",
    "inventoryJobList",
    "inventoryAddressList",
    "inventoryHoldingList",
    "consolidationPlanList",
  ]);
  const operations = createOperationsActions({
    api: async () => ({}),
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  operations.renderQueueJobs([
    {
      id: "job-1",
      state: "queued",
      kind: "eth_stealth_native_sweep",
      wallet_profile: "daily",
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  ok(dom.el("queueList").innerHTML.includes("job-1"));
  ok(dom.el("queueList").innerHTML.includes("native sweep"));

  const inventory = createInventoryActions({
    api: async () => ({}),
    toast: () => undefined,
    downloadJson: () => undefined,
  });
  inventory.renderInventoryState({
    jobs: [{ id: "scan-1", status: "queued", wallet_profiles: ["daily"] }],
    addresses: [
      {
        address: "0xabc",
        activity_state: "funded",
        wallet_family: "eth-seed",
        wallet_profile: "archive",
        chain_id: 1,
        derivation_path: "m/44'/60'/0'/0/0",
        classifications: ["signer_available", "dormant_candidate"],
      },
    ],
    holdings: [
      {
        asset_kind: "native",
        status: "active",
        address: "0xabc",
        amount_hex: "0x1",
        protocol_address: "0xpermit2",
        wallet_family: "eth-seed",
        wallet_profile: "archive",
        provider_profile: "mainnet",
      },
    ],
  });
  ok(dom.el("inventoryJobList").innerHTML.includes("scan-1"));
  ok(dom.el("inventoryAddressList").innerHTML.includes("0xabc"));
  ok(dom.el("inventoryAddressList").innerHTML.includes("dormant_candidate"));
  ok(dom.el("inventoryHoldingList").innerHTML.includes("native"));
  ok(dom.el("inventoryHoldingList").innerHTML.includes("0xpermit2"));

  inventory.renderConsolidationPlans([
    {
      id: "plan-1",
      status: "review_required",
      summary: {
        total_steps: 1,
        blocked_steps: 0,
        review_required_steps: 1,
        approved_steps: 0,
        executable_steps: 0,
        value_items: 1,
      },
      created_at_unix: 1,
      updated_at_unix: 2,
      steps: [
        {
          id: "step-1",
          action: "revoke_erc20_approval",
          status: "review_required",
          wallet_family: "eth-seed",
          wallet_profile: "archive",
          provider_profile: "mainnet",
          chain_id: 1,
          address: "0xabc",
          derivation_path: "m/44'/60'/0'/0/0",
          asset_kind: "approval",
          asset_address: "0xtoken",
          amount_hex: "0xffff",
          counterparty_address: "0xspender",
          protocol_address: null,
          signer_status: "available",
          simulation_status: "required",
          simulation_evidence: ["rpc_method=eth_call"],
          risk_level: "high",
          blockers: [],
          auto_eligible: false,
          approved: false,
        },
      ],
    },
  ]);
  ok(dom.el("consolidationPlanList").innerHTML.includes("revoke_erc20_approval"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("0xspender"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("simulation=required"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("rpc_method=eth_call"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("simulateConsolidationPlan"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("exportConsolidationPlan"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("Safe JSON"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("Call JSON"));
});

test("inventory actions export consolidation manifests as downloads", async () => {
  installDom();
  let requestBody: any = null;
  let download: { filename: string; payload: any } | null = null;
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      equal(method, "POST");
      equal(path, "/api/plans/consolidation/export");
      requestBody = body;
      return {
        status: "exported",
        plan_id: "plan-1",
        format: "call_manifest",
        exported_steps: 2,
        skipped_steps: [{ step_id: "step-3", action: "claim_reward", reason: "blocked", blockers: [] }],
        bundles: [],
      };
    },
    toast: (message) => toasts.push(message),
    downloadJson: (filename, payload) => {
      download = { filename, payload };
    },
  });

  await inventory.exportConsolidationPlan("plan-1", "call_manifest");

  deepEqual(requestBody, {
    plan_id: "plan-1",
    step_ids: [],
    format: "call_manifest",
    safe_address: null,
  });
  equal(download?.filename, "sigillum-plan-1-call_manifest.json");
  equal(download?.payload.exported_steps, 2);
  ok(toasts.some((message) => message.includes("Exported 2 step(s); skipped 1")));
});

test("inventory scan sends optional EVM watch-address probes", async () => {
  const dom = installDom([
    "inventoryWatchAddress",
    "inventoryWatchLabel",
    "inventoryWatchAddresses",
    "inventoryIncludeWatchBook",
    "inventoryTokenAddress",
    "inventoryAllowanceSpender",
    "inventoryPermit2Contract",
    "inventoryPermit2Spender",
    "inventoryNftOperator",
    "inventoryWalletFamily",
    "inventoryWalletProfile",
    "inventoryProviderProfile",
    "inventoryGapLimit",
    "inventoryMaxIndex",
    "inventoryDiscoverErc20Transfers",
    "inventoryTokenDiscoveryFromBlock",
    "inventoryTokenDiscoveryToBlock",
    "inventoryTokenDiscoveryLimit",
    "inventoryDiscoverErc20Allowances",
    "inventoryAllowanceLimit",
    "inventoryDiscoverPermit2Allowances",
    "inventoryPermit2AllowanceLimit",
    "inventoryDiscoverErc721Transfers",
    "inventoryDiscoverErc1155Transfers",
    "inventoryDiscoverNftOperatorApprovals",
    "inventoryNftOperatorApprovalLimit",
    "inventoryNftDiscoveryFromBlock",
    "inventoryNftDiscoveryToBlock",
    "inventoryNftDiscoveryLimit",
  ]);
  dom.el("inventoryWatchAddress").value = "0x7777777777777777777777777777777777777777";
  dom.el("inventoryWatchLabel").value = "old-ledger";
  dom.el("inventoryIncludeWatchBook").checked = true;
  dom.el("inventoryWatchAddresses").value = [
    "# old client batch",
    "0x8888888888888888888888888888888888888888,client-vault",
    "0x7777777777777777777777777777777777777777:duplicate",
  ].join("\n");
  dom.el("inventoryProviderProfile").value = "mainnet";
  let requestBody: any = null;
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      equal(method, "POST");
      equal(path, "/api/inventory/scan/evm");
      requestBody = body;
      return { status: "completed" };
    },
    toast: () => undefined,
    downloadJson: () => undefined,
  });

  await inventory.scanInventoryEvm();

  deepEqual(requestBody.watch_addresses, [
    {
      address: "0x7777777777777777777777777777777777777777",
      label: "old-ledger",
    },
    {
      address: "0x8888888888888888888888888888888888888888",
      label: "client-vault",
    },
  ]);
  equal(requestBody.wallet_family, "eth-watch");
  equal(requestBody.include_watch_book, true);
  equal(requestBody.provider_profile, "mainnet");
  equal(requestBody.block_tag, "latest");
});

test("watch-address parser accepts bulk line formats and dedupes", () => {
  deepEqual(
    parseWatchAddressProbes(
      [
        "# imported sheet",
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,old-ledger",
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:client",
        "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA,duplicate",
      ].join("\n"),
      "0xcccccccccccccccccccccccccccccccccccccccc",
      "single",
    ),
    [
      {
        address: "0xcccccccccccccccccccccccccccccccccccccccc",
        label: "single",
      },
      {
        address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        label: "old-ledger",
      },
      {
        address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        label: "client",
      },
    ],
  );
});

test("inventory report export summarizes watch addresses and risk state", async () => {
  let download: { filename: string; payload: any } | null = null;
  const inventory = createInventoryActions({
    api: async (_method, path) => {
      if (path === "/api/inventory/watch-addresses") {
        return {
          entries: [
            {
              address: "0x7777777777777777777777777777777777777777",
              label: "old-ledger",
              tags: ["archive"],
              enabled: true,
            },
          ],
        };
      }
      if (path === "/api/inventory/wallets") {
        return {
          jobs: [{ id: "job-1" }],
          addresses: [
            {
              wallet_family: "eth-watch",
              wallet_profile: "watch:old-ledger",
              activity_state: "funded",
            },
            {
              wallet_family: "eth-seed",
              wallet_profile: "seed-main",
              activity_state: "empty",
            },
          ],
          holdings: [{ asset_kind: "native" }],
        };
      }
      if (path === "/api/risk/findings") {
        return { findings: [{ id: "risk-1" }] };
      }
      if (path === "/api/plans/consolidation") {
        return {
          plans: [
            {
              id: "plan-1",
              steps: [{ blockers: ["watch_only"] }, { blockers: [] }],
            },
          ],
        };
      }
      return { error: "unexpected path" };
    },
    toast: () => undefined,
    downloadJson: (filename, payload) => {
      download = { filename, payload };
    },
  });

  await inventory.exportInventoryReport();

  ok(download?.filename.startsWith("sigillum-inventory-report-"));
  equal(download?.payload.summary.watch_address_count, 1);
  equal(download?.payload.summary.saved_watch_address_count, 1);
  equal(download?.payload.summary.blocked_plan_step_count, 1);
  equal(download?.payload.watch_address_book[0].label, "old-ledger");
  equal(download?.payload.risk_findings.length, 1);

  const report = buildInventoryReport({ jobs: [], addresses: [], holdings: [] }, [], [], [], 42);
  equal(report.generated_at_unix, 42);
  equal(report.summary.address_count, 0);
});

test("saved watch-address UI renders, saves, toggles, and deletes entries", async () => {
  const dom = installDom([
    "watchAddressBookList",
    "watchBookAddress",
    "watchBookLabel",
    "watchBookTags",
    "watchBookEnabled",
    "inventoryWatchAddress",
    "inventoryWatchLabel",
    "inventoryWatchAddresses",
  ]);
  (globalThis as any).confirm = () => true;
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return { status: "ok", entries: [] };
    },
    toast: (message) => toasts.push(message),
    downloadJson: () => undefined,
  });

  inventory.renderWatchAddressBook([
    {
      id: "watch-1",
      address: "0x7777777777777777777777777777777777777777",
      label: "old-ledger",
      tags: ["archive", "ledger"],
      source: "operator",
      enabled: true,
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  ok(dom.el("watchAddressBookList").innerHTML.includes("old-ledger"));
  ok(dom.el("watchAddressBookList").innerHTML.includes("Disable"));

  inventory.loadWatchAddressBookEntry(
    "0x7777777777777777777777777777777777777777",
    "old-ledger",
    "archive, ledger",
    "true",
  );
  equal(dom.el("watchBookAddress").value, "0x7777777777777777777777777777777777777777");
  equal(dom.el("watchBookTags").value, "archive, ledger");
  equal(dom.el("watchBookEnabled").checked, true);

  dom.el("watchBookAddress").value = "0x8888888888888888888888888888888888888888";
  dom.el("watchBookLabel").value = "client";
  dom.el("watchBookTags").value = "archive, archive, client";
  dom.el("watchBookEnabled").checked = false;
  await inventory.upsertWatchAddressBookEntry();
  const firstUpsert = calls.find(
    (call) => call.path === "/api/inventory/watch-addresses/upsert",
  );
  deepEqual(firstUpsert, {
    method: "POST",
    path: "/api/inventory/watch-addresses/upsert",
    body: {
      address: "0x8888888888888888888888888888888888888888",
      label: "client",
      tags: ["archive", "client"],
      enabled: false,
    },
  });

  calls.length = 0;
  dom.el("inventoryWatchAddress").value = "0x9999999999999999999999999999999999999999";
  dom.el("inventoryWatchLabel").value = "batch-one";
  dom.el("inventoryWatchAddresses").value =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,batch-two";
  await inventory.upsertBulkWatchAddressBookEntries();
  const bulkUpserts = calls.filter(
    (call) => call.path === "/api/inventory/watch-addresses/upsert",
  );
  equal(bulkUpserts.length, 2);
  equal(bulkUpserts[0].body.address, "0x9999999999999999999999999999999999999999");
  equal(bulkUpserts[1].body.address, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

  calls.length = 0;
  await inventory.toggleWatchAddressBookEntry(
    "0x7777777777777777777777777777777777777777",
    "old-ledger",
    "archive",
    "false",
  );
  const toggleUpsert = calls.find(
    (call) => call.path === "/api/inventory/watch-addresses/upsert",
  );
  equal(toggleUpsert?.body.enabled, false);

  calls.length = 0;
  await inventory.deleteWatchAddressBookEntry("0x7777777777777777777777777777777777777777");
  const deleteCall = calls.find(
    (call) => call.path === "/api/inventory/watch-addresses/delete",
  );
  equal(deleteCall?.body.address, "0x7777777777777777777777777777777777777777");
  ok(toasts.some((message) => message.includes("deleted")));
});

test("data-action dispatcher coerces args and restores button busy state", async () => {
  const dom = installDom();
  const button = dom.el("action", "BUTTON");
  button.dataset.action = "run";
  button.dataset.arg0 = "7";
  button.dataset.arg0Type = "number";
  let args: unknown[] = [];

  dispatchDataAction(button as any, {
    actions: {
      run: (...received) => {
        args = received;
      },
    },
    toast: () => undefined,
  });

  equal(button.disabled, true);
  await Promise.resolve();
  await Promise.resolve();
  deepEqual(args, [7]);
  equal(button.disabled, false);
  equal(button.classList.contains("is-busy"), false);
});

test("wei-hex formatter converts to ETH units and guards invalid input", () => {
  equal(formatWeiHexAsEth("0xde0b6b3a7640000"), "1");
  equal(formatWeiHexAsEth("0x1"), "0.000000000000000001");
  equal(formatWeiHexAsEth("0x0"), "0");
  equal(formatWeiHexAsEth("0x" + (1500000000000000000n).toString(16)), "1.5");
  equal(formatWeiHexAsEth("0x" + (2000000000000000000000n).toString(16)), "2000");
  equal(formatWeiHexAsEth("0x" + (1000000000123n).toString(16)), "0.000001000000000123");
  equal(formatWeiHexAsEth(""), "0");
  equal(formatWeiHexAsEth("0x"), "0");
  equal(formatWeiHexAsEth("nonsense"), "0");
  equal(formatWeiHexAsEth("0xzz"), "0");
  equal(formatWeiHexAsEth(undefined as unknown as string), "0");
});

test("treasury overview loader renders tiles, chains, groups, routing, and risk", async () => {
  const dom = installDom([
    "treasuryOverviewStats",
    "treasuryGeneratedAt",
    "treasuryChainList",
    "treasuryGroupList",
    "treasuryRoutingList",
    "treasuryRiskPlanList",
    "treasuryPolicyList",
    "treasuryReceiveList",
  ]);
  const overview: TreasuryOverviewResponse = {
    generated_at_unix: 1717900000,
    tracked_address_count: 6,
    funded_address_count: 3,
    watch_only_address_count: 2,
    signer_address_count: 4,
    chains: [
      {
        chain_id: 1,
        native_symbol: "ETH",
        address_count: 5,
        funded_address_count: 3,
        native_total_wei_hex: "0x" + (1500000000000000000n).toString(16),
      },
    ],
    groups: [
      {
        wallet_family: "eth-seed",
        wallet_profile: "archive",
        chain_id: 1,
        address_count: 4,
        funded_address_count: 2,
        native_total_wei_hex: "0xde0b6b3a7640000",
        signer_address_count: 3,
        watch_only_address_count: 1,
        erc20_holding_count: 2,
        nft_holding_count: 1,
        defi_holding_count: 0,
        claimable_holding_count: 1,
        approval_exposure_count: 2,
        dormant_candidate_count: 1,
      },
      {
        wallet_family: "eth-watch",
        wallet_profile: "watch:client",
        chain_id: 137,
        address_count: 2,
        funded_address_count: 0,
        native_total_wei_hex: "0x0",
        signer_address_count: 0,
        watch_only_address_count: 2,
        erc20_holding_count: 0,
        nft_holding_count: 0,
        defi_holding_count: 0,
        claimable_holding_count: 0,
        approval_exposure_count: 0,
        dormant_candidate_count: 0,
      },
    ],
    routing: [
      {
        wallet_profile: "daily",
        hot_address: "0x1111111111111111111111111111111111111111",
        treasury_address: "0x2222222222222222222222222222222222222222",
        default_destination_address: null,
        hot_native_balance_wei_hex: "0x" + (500000000000000000n).toString(16),
        treasury_native_balance_wei_hex: "0x" + (2000000000000000000n).toString(16),
        routing_ready: true,
      },
      {
        wallet_profile: "archive",
        routing_ready: false,
      },
    ],
    risk: {
      total_findings: 4,
      critical_findings: 1,
      high_findings: 1,
      medium_findings: 1,
      low_findings: 1,
    },
    plans: {
      total_plans: 2,
      latest_plan_id: "plan-9",
      latest_plan_status: "review_required",
      latest_review_required_steps: 1,
      latest_approved_steps: 1,
      latest_executable_steps: 2,
      latest_blocked_steps: 1,
      policy_violations: ["destination_not_allowed:0xdead"],
    },
    receive: {
      active_allocations: 3,
      retired_allocations: 1,
      purposes: 2,
    },
  };
  const calls: string[] = [];
  const treasury = createTreasuryActions({
    api: async (method, path) => {
      calls.push(method + " " + path);
      return overview;
    },
    toast: () => undefined,
  });

  await treasury.loadTreasuryOverview();

  deepEqual(calls, [
    "GET /api/treasury/overview",
    "GET /api/treasury/policy",
    "GET /api/treasury/receive-addresses",
  ]);
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Tracked Addresses"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes(">6<"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Watch-Only"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Receive Active"));
  ok(dom.el("treasuryGeneratedAt").textContent.startsWith("Updated "));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("No treasury policy configured yet."));
  ok(dom.el("treasuryReceiveList").innerHTML.includes("No receive allocations yet."));

  ok(dom.el("treasuryChainList").innerHTML.includes("chain 1 · ETH"));
  ok(dom.el("treasuryChainList").innerHTML.includes("addresses=3/5 funded"));
  ok(dom.el("treasuryChainList").innerHTML.includes("native=1.5 ETH"));

  ok(dom.el("treasuryGroupList").innerHTML.includes("eth-seed/archive"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("native=1 ETH"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("erc20=2"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("claimable=1"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("approvals=2"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("dormant=1"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("approval exposure"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("eth-watch/watch:client"));
  ok(dom.el("treasuryGroupList").innerHTML.includes("native=0 ·"));

  ok(
    dom
      .el("treasuryRoutingList")
      .innerHTML.includes("hot=0x1111111111111111111111111111111111111111 (0.5)"),
  );
  ok(
    dom
      .el("treasuryRoutingList")
      .innerHTML.includes("treasury=0x2222222222222222222222222222222222222222 (2)"),
  );
  ok(dom.el("treasuryRoutingList").innerHTML.includes("ready"));
  ok(dom.el("treasuryRoutingList").innerHTML.includes("unconfigured"));
  ok(dom.el("treasuryRoutingList").innerHTML.includes("hot=-"));

  ok(dom.el("treasuryRiskPlanList").innerHTML.includes("critical=1"));
  ok(dom.el("treasuryRiskPlanList").innerHTML.includes("total=4"));
  ok(dom.el("treasuryRiskPlanList").innerHTML.includes("plan-9"));
  ok(dom.el("treasuryRiskPlanList").innerHTML.includes("review required"));
  ok(dom.el("treasuryRiskPlanList").innerHTML.includes("executable=2"));
  ok(
    dom
      .el("treasuryRiskPlanList")
      .innerHTML.includes("policyViolations=destination_not_allowed:0xdead"),
  );

  const toasts: Array<{ message: string; type?: string }> = [];
  const failing = createTreasuryActions({
    api: async () => ({ error: "treasury overview unavailable" }),
    toast: (message, type) => toasts.push({ message, type }),
  });
  await failing.loadTreasuryOverview();
  equal(toasts.length, 0);
  await failing.refreshTreasuryOverview();
  deepEqual(toasts.pop(), {
    message: "treasury overview unavailable",
    type: "error",
  });
});

test("eth-to-wei-hex parser converts decimal ETH amounts and rejects garbage", () => {
  equal(parseEthToWeiHex("1"), "0xde0b6b3a7640000");
  equal(parseEthToWeiHex("0.5"), "0x" + (500000000000000000n).toString(16));
  equal(parseEthToWeiHex("1.5"), "0x" + (1500000000000000000n).toString(16));
  equal(parseEthToWeiHex("0.000000000000000001"), "0x1");
  equal(parseEthToWeiHex(" 2 "), "0x" + (2000000000000000000n).toString(16));
  equal(parseEthToWeiHex("0"), "0x0");
  equal(formatWeiHexAsEth(parseEthToWeiHex("1.5") as string), "1.5");
  equal(parseEthToWeiHex(""), null);
  equal(parseEthToWeiHex("   "), null);
  equal(parseEthToWeiHex("nonsense"), null);
  equal(parseEthToWeiHex("1.2.3"), null);
  equal(parseEthToWeiHex("-1"), null);
  equal(parseEthToWeiHex("1,5"), null);
  equal(parseEthToWeiHex("0x1"), null);
  equal(parseEthToWeiHex("0.0000000000000000001"), null);
  equal(parseEthToWeiHex(undefined as unknown as string), null);
});

test("treasury destination-line parser splits on first colon, trims, and skips empties", () => {
  deepEqual(
    parseTreasuryDestinationLines(
      [
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "  0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb : cold vault ",
        "",
        "   ",
        "0xcccccccccccccccccccccccccccccccccccccccc:multi:part:label",
        "0xdddddddddddddddddddddddddddddddddddddddd:",
      ].join("\n"),
    ),
    [
      { address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
      { address: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", label: "cold vault" },
      {
        address: "0xcccccccccccccccccccccccccccccccccccccccc",
        label: "multi:part:label",
      },
      { address: "0xdddddddddddddddddddddddddddddddddddddddd" },
    ],
  );
  deepEqual(parseTreasuryDestinationLines(null), []);
  deepEqual(parseTreasuryDestinationLines(undefined), []);
});

test("treasury policy renderer shows configured policy and empty state", () => {
  const dom = installDom(["treasuryPolicyList"]);
  const treasury = createTreasuryActions({
    api: async () => ({}),
    toast: () => undefined,
  });
  const policy: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [
      { address: "0x2222222222222222222222222222222222222222", label: "cold-vault" },
      { address: "0x3333333333333333333333333333333333333333" },
    ],
    max_step_native_wei_hex: "0x" + (1500000000000000000n).toString(16),
    max_plan_native_wei_hex: null,
    require_simulation: true,
    created_at_unix: 1717900000,
    updated_at_unix: 1717900500,
  };

  treasury.renderTreasuryPolicy(policy);
  const html = dom.el("treasuryPolicyList").innerHTML;
  ok(html.includes("Treasury policy"));
  ok(html.includes(">enabled<"));
  ok(html.includes("0x2222222222222222222222222222222222222222 (cold-vault)"));
  ok(html.includes("0x3333333333333333333333333333333333333333"));
  ok(html.includes("maxStep=1.5 ETH"));
  ok(html.includes("maxPlan=-"));
  ok(html.includes("requireSimulation=true"));

  treasury.renderTreasuryPolicy(null);
  ok(
    dom.el("treasuryPolicyList").innerHTML.includes("No treasury policy configured yet."),
  );
});

test("treasury receive list renders rotate buttons only for active allocations", () => {
  const dom = installDom(["treasuryReceiveList"]);
  const treasury = createTreasuryActions({
    api: async () => ({}),
    toast: () => undefined,
  });
  const allocations: TreasuryReceiveAllocation[] = [
    {
      id: "alloc-1",
      wallet_family: "eth-seed",
      wallet_profile: "archive",
      address: "0x4444444444444444444444444444444444444444",
      derivation_path: "m/44'/60'/0'/0/7",
      address_index: 7,
      purpose: "invoices",
      label: "client-a",
      status: "active",
      created_at_unix: 1,
    },
    {
      id: "alloc-0",
      wallet_family: "eth-seed",
      wallet_profile: "archive",
      address: "0x5555555555555555555555555555555555555555",
      derivation_path: "m/44'/60'/0'/0/6",
      address_index: 6,
      purpose: "invoices",
      status: "retired",
      created_at_unix: 1,
      retired_at_unix: 2,
    },
  ];

  treasury.renderTreasuryReceiveAllocations(allocations);
  const html = dom.el("treasuryReceiveList").innerHTML;
  ok(html.includes("0x4444444444444444444444444444444444444444"));
  ok(html.includes("0x5555555555555555555555555555555555555555"));
  ok(html.includes("eth-seed/archive"));
  ok(html.includes("purpose=invoices"));
  ok(html.includes("label=client-a"));
  ok(html.includes("path=m/44'/60'/0'/0/7"));
  ok(html.includes("index=7"));
  ok(html.includes(">retired<"));
  equal(html.split('data-action="rotateTreasuryReceiveAddress"').length - 1, 1);
  ok(html.includes('data-arg0="alloc-1"'));
  ok(!html.includes('data-arg0="alloc-0"'));

  treasury.renderTreasuryReceiveAllocations([]);
  ok(dom.el("treasuryReceiveList").innerHTML.includes("No receive allocations yet."));
});

test("treasury policy loader prefills the form without clobbering operator edits", async () => {
  const dom = installDom([
    "treasuryPolicyList",
    "treasuryPolicyEnabled",
    "treasuryPolicyDestinations",
    "treasuryPolicyMaxStepEth",
    "treasuryPolicyMaxPlanEth",
    "treasuryPolicyRequireSim",
  ]);
  const policy: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [
      { address: "0x2222222222222222222222222222222222222222", label: "cold" },
    ],
    max_step_native_wei_hex: "0x" + (2000000000000000000n).toString(16),
    max_plan_native_wei_hex: "0xde0b6b3a7640000",
    require_simulation: true,
    created_at_unix: 1,
    updated_at_unix: 2,
  };
  const treasury = createTreasuryActions({
    api: async (_method, path) => (path === "/api/treasury/policy" ? { policy } : {}),
    toast: () => undefined,
  });

  await treasury.loadTreasuryOverview();
  equal(dom.el("treasuryPolicyEnabled").checked, true);
  equal(dom.el("treasuryPolicyRequireSim").checked, true);
  equal(
    dom.el("treasuryPolicyDestinations").value,
    "0x2222222222222222222222222222222222222222:cold",
  );
  equal(dom.el("treasuryPolicyMaxStepEth").value, "2");
  equal(dom.el("treasuryPolicyMaxPlanEth").value, "1");
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxPlan=1 ETH"));

  dom.el("treasuryPolicyDestinations").value = "0xdraft";
  await treasury.loadTreasuryOverview();
  equal(dom.el("treasuryPolicyDestinations").value, "0xdraft");
});

test("treasury policy save validates caps and submits the parsed update request", async () => {
  const dom = installDom([
    "treasuryPolicyList",
    "treasuryPolicyEnabled",
    "treasuryPolicyDestinations",
    "treasuryPolicyMaxStepEth",
    "treasuryPolicyMaxPlanEth",
    "treasuryPolicyRequireSim",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const savedPolicy: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [
      { address: "0x2222222222222222222222222222222222222222", label: "cold" },
      { address: "0x3333333333333333333333333333333333333333" },
    ],
    max_step_native_wei_hex: "0x" + (1500000000000000000n).toString(16),
    max_plan_native_wei_hex: null,
    require_simulation: false,
    created_at_unix: 1,
    updated_at_unix: 2,
  };
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy/update") {
        return { status: "updated", policy: savedPolicy };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  dom.el("treasuryPolicyEnabled").checked = true;
  dom.el("treasuryPolicyRequireSim").checked = false;
  dom.el("treasuryPolicyDestinations").value =
    "0x2222222222222222222222222222222222222222:cold\n0x3333333333333333333333333333333333333333";
  dom.el("treasuryPolicyMaxStepEth").value = "1.5";
  dom.el("treasuryPolicyMaxPlanEth").value = "not-a-number";

  await treasury.updateTreasuryPolicy();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Max per-plan cap must be a decimal ETH amount with up to 18 decimals",
    type: "error",
  });

  dom.el("treasuryPolicyMaxPlanEth").value = "";
  await treasury.updateTreasuryPolicy();

  const update = calls.find((call) => call.path === "/api/treasury/policy/update");
  deepEqual(update, {
    method: "POST",
    path: "/api/treasury/policy/update",
    body: {
      enabled: true,
      allowed_destinations: [
        { address: "0x2222222222222222222222222222222222222222", label: "cold" },
        { address: "0x3333333333333333333333333333333333333333" },
      ],
      max_step_native_wei_hex: "0x" + (1500000000000000000n).toString(16),
      max_plan_native_wei_hex: null,
      require_simulation: false,
    },
  });
  deepEqual(toasts.pop(), { message: "Treasury policy saved", type: undefined });
  ok(dom.el("treasuryPolicyList").innerHTML.includes(">enabled<"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxStep=1.5 ETH"));
  equal(dom.el("treasuryPolicyMaxStepEth").value, "1.5");
  ok(calls.some((call) => call.path === "/api/treasury/overview"));
});

test("treasury receive allocate and rotate dispatch api calls with toasts", async () => {
  const dom = installDom([
    "treasuryReceiveList",
    "treasuryReceiveProfile",
    "treasuryReceivePurpose",
    "treasuryReceiveLabel",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const allocation: TreasuryReceiveAllocation = {
    id: "alloc-7",
    wallet_family: "eth-seed",
    wallet_profile: "archive",
    address: "0x6666666666666666666666666666666666666666",
    derivation_path: "m/44'/60'/0'/0/9",
    address_index: 9,
    purpose: "donations",
    label: "fundraiser",
    status: "active",
    created_at_unix: 5,
  };
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/receive-addresses/allocate") {
        return { status: "allocated", allocation };
      }
      if (path === "/api/treasury/receive-addresses/rotate") {
        return { status: "rotated", allocation: { ...allocation, status: "retired" } };
      }
      if (path === "/api/treasury/receive-addresses") {
        return { allocations: [allocation] };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  await treasury.allocateTreasuryReceiveAddress();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Wallet profile and purpose are required",
    type: "error",
  });

  dom.el("treasuryReceiveProfile").value = "archive";
  dom.el("treasuryReceivePurpose").value = "donations";
  dom.el("treasuryReceiveLabel").value = "fundraiser";

  let pending: Promise<unknown> = Promise.resolve();
  const allocateButton = dom.el("allocateBtn", "BUTTON");
  allocateButton.dataset.action = "allocateTreasuryReceiveAddress";
  dispatchDataAction(allocateButton as any, {
    actions: {
      allocateTreasuryReceiveAddress: () =>
        (pending = treasury.allocateTreasuryReceiveAddress()),
    },
    toast: () => undefined,
  });
  await pending;

  const allocateCall = calls.find(
    (call) => call.path === "/api/treasury/receive-addresses/allocate",
  );
  deepEqual(allocateCall, {
    method: "POST",
    path: "/api/treasury/receive-addresses/allocate",
    body: { wallet_profile: "archive", purpose: "donations", label: "fundraiser" },
  });
  equal(dom.el("treasuryReceiveProfile").value, "");
  equal(dom.el("treasuryReceivePurpose").value, "");
  equal(dom.el("treasuryReceiveLabel").value, "");
  deepEqual(toasts.pop(), {
    message: "Receive address allocated: 0x6666666666666666666666666666666666666666",
    type: undefined,
  });
  ok(calls.some((call) => call.path === "/api/treasury/receive-addresses"));
  ok(calls.some((call) => call.path === "/api/treasury/overview"));

  calls.length = 0;
  const rotateButton = dom.el("rotateBtn", "BUTTON");
  rotateButton.dataset.action = "rotateTreasuryReceiveAddress";
  rotateButton.dataset.arg0 = "alloc-7";
  dispatchDataAction(rotateButton as any, {
    actions: {
      rotateTreasuryReceiveAddress: (...args: unknown[]) =>
        (pending = treasury.rotateTreasuryReceiveAddress(args[0] as string)),
    },
    toast: () => undefined,
  });
  await pending;

  deepEqual(calls[0], {
    method: "POST",
    path: "/api/treasury/receive-addresses/rotate",
    body: { allocation_id: "alloc-7" },
  });
  deepEqual(toasts.pop(), { message: "Receive address rotated", type: undefined });
  ok(calls.some((call) => call.path === "/api/treasury/receive-addresses"));
});
