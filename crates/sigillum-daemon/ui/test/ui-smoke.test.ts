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
import { createShellRenderer } from "../src/views/shell";
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
