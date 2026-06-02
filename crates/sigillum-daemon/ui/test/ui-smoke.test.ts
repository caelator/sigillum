import { deepEqual, equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  clearSessionToken,
  readSessionToken,
  requestWithSession,
  writeSessionToken,
} from "../src/api/session";
import { dispatchDataAction } from "../src/actions/dispatcher";
import { createInventoryActions } from "../src/views/inventory";
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
