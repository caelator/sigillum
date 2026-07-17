import { deepEqual, equal, ok } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  clearSessionToken,
  readSessionToken,
  requestWithSession,
  writeSessionToken,
} from "../src/api/session";
import { dispatchDataAction } from "../src/actions/dispatcher";
import {
  confirmDangerDialog,
  confirmTypedDialog,
  informDialog,
} from "../src/render/confirm";
import {
  amountWithRawHtml,
  chainLabel,
  formatEthAmount,
  formatHexQuantity,
  formatTimestamp,
  formatTokenAmount,
  quantityWithRawHtml,
} from "../src/render/format";
import { renderEntityList, showResultBox } from "../src/render/forms";
import {
  buildInventoryReport,
  createInventoryActions,
  parseWatchAddressProbes,
} from "../src/views/inventory";
import { computeJourneySteps, createJourneyActions } from "../src/views/journey";
import { createOperationsActions } from "../src/views/operations";
import { pillClass } from "../src/render/html";
import { createReceivingActions } from "../src/views/receiving";
import { createSelfCheckActions, formatClockTime } from "../src/views/selfcheck";
import { createSessionActions } from "../src/views/session";
import { createSetupWizard } from "../src/views/setup";
import {
  createShellRenderer,
  renderActiveCompartment,
  renderCompartmentSwitcher,
} from "../src/views/shell";
import { createWalletActions } from "../src/views/wallets";
import {
  createTreasuryActions,
  formatWeiHexAsEth,
  parseEthToWeiHex,
  parseTreasuryDestinationLines,
  treasuryPolicySummary,
} from "../src/views/treasury";
import {
  createWalletManagerActions,
  walletNativeBalanceFromGroups,
  walletRowMeta,
  xpubDisplay,
} from "../src/views/walletManager";
import type {
  EthSeedWalletProfile,
  EthXpubWalletProfile,
  ReceivingOverviewResponse,
  ReceivingRefreshResponse,
  SelfCheckRunResponse,
  TreasuryGroupSummary,
  TreasuryOverviewResponse,
  TreasuryPolicy,
  TreasuryReceiveAllocation,
} from "../src/contracts";
import { installDom } from "./dom-fixture";

// ── Shared confirm-dialog drivers ───────────────────────────────────────────
// Dangerous actions are gated by the modal in src/render/confirm.ts. The
// dialog renders into document.body via createElement/appendChild, so the
// fake DOM keeps real element references these helpers can drive.

async function tick(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function confirmOverlay(): any {
  return (
    ((document.body.children as unknown as any[]) || []).find(
      (child) => child.isConnected && child.getAttribute?.("data-confirm-overlay") != null,
    ) || null
  );
}

function confirmPart(selector: string): any {
  const overlay = confirmOverlay();
  return overlay ? overlay.querySelector(selector) : null;
}

function typeConfirmPhrase(phrase: string): void {
  const input = confirmPart("[data-confirm-input]");
  ok(input, "expected a typed-phrase input in the confirmation dialog");
  input.value = phrase;
  input.dispatchEvent({ type: "input", target: input });
}

async function answerConfirm(action: "action" | "cancel", phrase?: string): Promise<void> {
  await tick();
  const overlay = confirmOverlay();
  ok(overlay, "expected a confirmation dialog to be open");
  if (phrase !== undefined) typeConfirmPhrase(phrase);
  const button = overlay.querySelector(
    action === "action" ? "[data-confirm-action]" : "[data-confirm-cancel]",
  );
  ok(button, "expected the dialog " + action + " button");
  button.click();
  await tick();
}

test("confirmation dialog tiers resolve decisions and gate typed phrases", async () => {
  installDom();

  // inform: one acknowledgement button, no cancel, resolves true.
  let pending: Promise<boolean> = informDialog({
    title: "Clipboard unavailable",
    body: "Copy the value manually.",
    valueDisplay: "0xvalue",
  });
  let overlay = confirmOverlay();
  ok(overlay, "inform dialog opens");
  equal(overlay.getAttribute("data-confirm-overlay"), "inform");
  const dialog = overlay.children[0];
  equal(dialog.getAttribute("role"), "dialog");
  equal(dialog.getAttribute("aria-modal"), "true");
  ok(dialog.getAttribute("aria-labelledby"), "dialog is labelled by its title");
  equal(confirmPart("[data-confirm-cancel]"), null);
  equal(confirmPart("[data-confirm-value]").value, "0xvalue");
  confirmPart("[data-confirm-action]").click();
  equal(await pending, true);
  ok(!overlay.isConnected, "dialog closes after the decision");

  // confirm: Cancel and danger action, initial focus on the safe button.
  pending = confirmDangerDialog({ title: "Delete thing", body: "It is gone for good." });
  overlay = confirmOverlay();
  equal(overlay.getAttribute("data-confirm-overlay"), "confirm");
  const cancelButton = confirmPart("[data-confirm-cancel]");
  const actionButton = confirmPart("[data-confirm-action]");
  equal(cancelButton.textContent, "Cancel");
  equal(actionButton.textContent, "Confirm");
  equal(actionButton.className, "btn-danger");
  equal(document.activeElement, cancelButton, "focus starts on the safe button");
  actionButton.click();
  equal(await pending, true);

  // Cancel resolves false.
  pending = confirmDangerDialog({ title: "Delete thing", body: "Gone." });
  confirmPart("[data-confirm-cancel]").click();
  equal(await pending, false);

  // Escape resolves false.
  pending = confirmDangerDialog({ title: "Delete thing", body: "Gone." });
  (document as any).dispatchEvent({ type: "keydown", key: "Escape" });
  equal(await pending, false);

  // Backdrop click resolves false.
  pending = confirmDangerDialog({ title: "Delete thing", body: "Gone." });
  confirmOverlay().click();
  equal(await pending, false);

  // typed: the action stays disabled until the exact phrase is entered.
  const phrase = "EXECUTE 2 PLAN STEPS TOTAL 7 WEI";
  pending = confirmTypedDialog({ title: "Bulk enqueue", body: "Everything goes.", phrase });
  overlay = confirmOverlay();
  equal(overlay.getAttribute("data-confirm-overlay"), "typed");
  equal(confirmPart("[data-confirm-phrase]").textContent, phrase);
  const typedAction = confirmPart("[data-confirm-action]");
  equal(typedAction.disabled, true, "action disabled before the phrase matches");
  const phraseInput = confirmPart("[data-confirm-input]");
  equal(document.activeElement, phraseInput, "focus starts in the phrase input");

  // Wrong phrase: still disabled, clicking does nothing, Enter does nothing.
  typeConfirmPhrase("EXECUTE 9 PLAN STEPS TOTAL 1 WEI");
  equal(typedAction.disabled, true);
  typedAction.click();
  (document as any).dispatchEvent({
    type: "keydown",
    key: "Enter",
    target: phraseInput,
  });
  await tick();
  ok(confirmOverlay(), "dialog stays open while the phrase mismatches");

  // Exact phrase enables the action; Enter in the input submits.
  typeConfirmPhrase(phrase);
  equal(typedAction.disabled, false);
  (document as any).dispatchEvent({
    type: "keydown",
    key: "Enter",
    target: phraseInput,
  });
  equal(await pending, true);

  // Typed tier also cancels cleanly via Escape.
  pending = confirmTypedDialog({ title: "Bulk enqueue", body: "Everything goes.", phrase });
  (document as any).dispatchEvent({ type: "keydown", key: "Escape" });
  equal(await pending, false);
});

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
    "walletManagerCard",
    "profilesCard",
    "xpubCard",
    "receivingCard",
    "receiveBookCard",
    "treasuryCard",
    "inventoryCard",
    "depositsCard",
    "plansCard",
    "policyCard",
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

  // The periodic refresh re-applies setup mode; the wizard must only be
  // reset when ENTERING setup, or in-progress choices get wiped mid-flow.
  equal(calls.filter((entry) => entry === "wizard:reset").length, 1);
  renderer.applySetupUi();
  renderer.applySetupUi();
  equal(calls.filter((entry) => entry === "wizard:reset").length, 1);

  renderer.applyLockedUi();
  equal(mode, "locked");
  equal(document.body.dataset.mode, "locked");
  equal(dom.el("lockForm").classList.contains("hidden"), true);
  equal(dom.el("authRecovery").classList.contains("hidden"), false);

  renderer.applyUnlockedUi(
    { compartment_id: 1, compartment_label: "daily", api_key_count: 0 },
    [
      { id: 1, label: "daily", threshold: 1 },
      { id: 2, label: "secure", threshold: 2 },
    ],
  );
  equal(mode, "unlocked");
  equal(document.body.dataset.mode, "unlocked");
  equal(dom.el("pushCard").classList.contains("hidden"), false);
  equal(dom.el("receivingCard").classList.contains("hidden"), false);
  equal(dom.el("receiveBookCard").classList.contains("hidden"), false);
  equal(dom.el("treasuryCard").classList.contains("hidden"), false);
  equal(dom.el("plansCard").classList.contains("hidden"), false);
  equal(dom.el("policyCard").classList.contains("hidden"), false);
  equal(dom.el("walletManagerCard").classList.contains("hidden"), false);
  ok(calls.includes("push-selectors"));
});

test("shell renders the active compartment from the canonical status shape", () => {
  const dom = installDom([
    "compSwitcher",
    "compartmentBadge",
    "apiKeyCount",
    "secretCount",
    "compartmentCount",
  ]);
  // Canonical daemon wire shape (crates/sigillum-api/src/response.rs):
  // active_compartment carries compartment_id/compartment_label, while
  // unlocked_compartments entries carry id/label.
  const status = {
    initialized: true,
    locked: false,
    active_compartment: {
      compartment_id: 2,
      compartment_label: "vault2",
      api_key_count: 3,
      secret_count: 7,
    },
    unlocked_compartments: [
      { id: 1, label: "vault1", threshold: 1 },
      { id: 2, label: "vault2", threshold: 2 },
    ],
  };
  const renderer = createShellRenderer({
    operatorCardIds: [],
    setUiMode: () => undefined,
    setCardsHidden: () => undefined,
    setStatusBadge: () => undefined,
    setSecretsAccess: () => undefined,
    resetVaultCounts: () => undefined,
    setUnlockGuidance: () => undefined,
    updateHeroState: () => undefined,
    updateWizardChrome: () => undefined,
    resetSetupWizard: () => undefined,
    renderCompartmentSwitcher,
    renderActiveCompartment,
    buildPushSelectors: () => undefined,
  });

  renderer.applyUnlockedUi(status.active_compartment, status.unlocked_compartments);

  equal(dom.el("compartmentBadge").textContent, "vault2");
  equal(dom.el("apiKeyCount").textContent, "3");
  equal(dom.el("secretCount").textContent, "7");
  equal(dom.el("compartmentCount").textContent, "2");
  const switcherHtml = dom.el("compSwitcher").innerHTML;
  ok(switcherHtml.includes(">vault1</button>"));
  ok(switcherHtml.includes(">vault2</button>"));
  // Exactly one switcher button is active: the entry whose id matches
  // active_compartment.compartment_id.
  equal(switcherHtml.split('class="active"').length - 1, 1);
  ok(
    switcherHtml.includes(
      'class="active" data-action="switchCompartment" data-arg0="2" data-arg0-type="number"',
    ),
  );

  // A missing label falls back to the compartment id.
  renderActiveCompartment(
    { compartment_id: 3, compartment_label: "", api_key_count: 0, secret_count: null },
    status.unlocked_compartments,
  );
  equal(dom.el("compartmentBadge").textContent, "Compartment 3");
  equal(dom.el("secretCount").textContent, "(locked)");
});

test("discovery job cancel and resume controls drive the real endpoints", async () => {
  const dom = installDom([
    "inventoryJobList",
    "inventoryAddressList",
    "inventoryHoldingList",
    "nftMetadataList",
    "nftSuspiciousList",
  ]);
  const calls: Array<{ method: string; path: string; body: unknown }> = [];
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method: string, path: string, body?: unknown) => {
      calls.push({ method, path, body });
      if (path === "/api/discovery/jobs/cancel") {
        return {
          status: "cancel_requested",
          job: { id: "scan-1", status: "running" },
          operation: { id: "op-1", state: "cancel_requested" },
        };
      }
      if (path === "/api/discovery/jobs/resume") {
        return {
          status: "running",
          job: { id: "scan-2", status: "running" },
          operation: { id: "op-2", state: "running" },
        };
      }
      return {};
    },
    toast: (message: string) => {
      toasts.push(message);
    },
    downloadJson: () => undefined,
  });
  inventory.renderInventoryState({
    jobs: [{ id: "scan-1", status: "running", wallet_profiles: ["daily"] }],
    addresses: [],
    holdings: [],
  });
  const html = dom.el("inventoryJobList").innerHTML;
  // The job list renders with both verbs enabled — the daemon honors them
  // for real since plan task 1.2 landed.
  ok(html.includes("scan-1"));
  ok(html.includes(">running</span>"));
  equal(html.split('data-action="cancelDiscoveryJob"').length - 1, 1);
  equal(html.split('data-action="resumeDiscoveryJob"').length - 1, 1);
  ok(!html.includes("Cancel/resume arrives in a future update"));
  ok(!html.includes("disabled title="));

  // Cancel gates on the shared confirm dialog before posting.
  const posts = () => calls.filter((call) => call.method === "POST");
  let pending = inventory.cancelDiscoveryJob("scan-1");
  equal(posts().length, 0, "no request before the dialog is answered");
  await answerConfirm("action");
  await pending;
  deepEqual(posts()[0], {
    method: "POST",
    path: "/api/discovery/jobs/cancel",
    body: { id: "scan-1" },
  });
  ok(toasts.some((message) => message.includes("Cancel requested")));

  // Dismissing the dialog skips the request entirely.
  pending = inventory.cancelDiscoveryJob("scan-1");
  await answerConfirm("cancel");
  await pending;
  equal(posts().length, 1, "dismissed confirm must not post");

  // Resume posts to the real verb and reports the background restart.
  await inventory.resumeDiscoveryJob("scan-1");
  deepEqual(posts()[posts().length - 1], {
    method: "POST",
    path: "/api/discovery/jobs/resume",
    body: { id: "scan-1" },
  });
  ok(toasts.some((message) => message.includes("resumed in the background")));
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

test("session requests carry structured error code and fields through", async () => {
  installDom();
  clearSessionToken();
  const envelope = {
    code: "validation_failed",
    error: "name exceeds maximum length of 256 bytes (got 300 bytes)",
    fields: [
      {
        field: "name",
        message: "name exceeds maximum length of 256 bytes (got 300 bytes)",
      },
    ],
  };
  (globalThis as any).fetch = async () => ({
    status: 400,
    json: async () => envelope,
  });

  const payload = await requestWithSession("POST", "/api/profiles/evm/upsert", {
    name: "x",
  });
  equal(payload.error, envelope.error);
  equal(payload.code, "validation_failed");
  deepEqual(payload.fields, envelope.fields);
});

test("session actions drive unlock, lock, and browser logout workflow", async () => {
  const dom = installDom(["passphrase", "unlockButton", "unlockError"]);
  dom.el("unlockError").classList.add("hidden");
  dom.el("unlockButton").textContent = "Unlock vault";
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  let refreshCount = 0;
  const actions = createSessionActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      const requestBody = body as { passphrase?: string } | undefined;
      if (path === "/api/unlock" && requestBody?.passphrase === "already") {
        return { error: "Vault is already unlocked." };
      }
      if (path === "/api/unlock" && requestBody?.passphrase === "wrong") {
        return { error: "Unlock failed: bad passphrase." };
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
  });

  // Empty passphrase: inline error, no network call, nothing silent.
  dom.el("passphrase").value = "";
  await actions.unlock();
  equal(calls.length, 0);
  equal(dom.el("unlockError").textContent, "Enter your vault passphrase first.");
  ok(!dom.el("unlockError").classList.contains("hidden"));

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
  // Busy state is restored after the attempt completes.
  equal(dom.el("unlockButton").disabled, false);
  equal(dom.el("unlockButton").textContent, "Unlock vault");
  ok(dom.el("unlockError").classList.contains("hidden"));

  dom.el("passphrase").value = "already";
  await actions.unlock();
  equal(dom.el("passphrase").value, "already");
  deepEqual(toasts.pop(), {
    message: "Session already active. Refreshing workspace...",
    type: undefined,
  });
  equal(refreshCount, 2);
  equal(dom.el("unlockButton").disabled, false);

  // Wrong passphrase: the failure renders inline under the field too.
  dom.el("passphrase").value = "wrong";
  await actions.unlock();
  equal(dom.el("unlockError").textContent, "Unlock failed: bad passphrase.");
  ok(!dom.el("unlockError").classList.contains("hidden"));
  deepEqual(toasts.pop(), { message: "Unlock failed: bad passphrase.", type: "error" });

  writeSessionToken("still-active");
  let lockPending = actions.lock();
  await answerConfirm("cancel");
  await lockPending;
  equal(readSessionToken(), "still-active");
  ok(!calls.some((call) => call.path === "/api/lock"));

  lockPending = actions.lock();
  await answerConfirm("action");
  await lockPending;
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
    "wizLinkageChoiceStatus",
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

test("setup wizard enables payer-linkage protection from done step", async () => {
  const dom = installDom(["wizLinkageChoiceStatus"]);
  dom.el("wizLinkageChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy") return { policy: null };
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  await wizard.wizEnableLinkageProtection();

  deepEqual(calls, [
    {
      method: "GET",
      path: "/api/treasury/policy",
      body: undefined,
    },
    {
      method: "POST",
      path: "/api/treasury/policy/update",
      body: { enabled: false, block_cross_party_linkage: true },
    },
  ]);
  equal(dom.el("wizLinkageChoiceStatus").classList.contains("hidden"), false);
  ok((dom.el("wizLinkageChoiceStatus").textContent || "").length > 0);
  deepEqual(toasts.pop(), {
    message: "Payer-linkage protection enabled",
    type: undefined,
  });
});

test("setup wizard preserves merkle claim opt-in when enabling payer-linkage protection", async () => {
  const dom = installDom(["wizLinkageChoiceStatus"]);
  dom.el("wizLinkageChoiceStatus").classList.add("hidden");
  const existingPolicy: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [
      { address: "0x2222222222222222222222222222222222222222", label: "cold" },
    ],
    max_step_native_wei_hex: "0x1",
    max_plan_native_wei_hex: null,
    hot_floor_wei_hex: "0x2",
    hot_target_wei_hex: "0x3",
    hot_overflow_wei_hex: "0x5",
    require_simulation: true,
    block_cross_party_linkage: false,
    allow_claim_execution: true,
    allow_gas_topups: true,
    allow_treasury_automation: true,
    max_gas_topup_wei_hex: "0x4",
    simulation_freshness_secs: 120,
    created_at_unix: 1,
    updated_at_unix: 2,
  };
  const calls: Array<{ method: string; path: string; body?: any }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy") return { policy: existingPolicy };
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  await wizard.wizEnableLinkageProtection();

  const update = calls.find((call) => call.path === "/api/treasury/policy/update");
  deepEqual(update, {
    method: "POST",
    path: "/api/treasury/policy/update",
    body: {
      enabled: true,
      allowed_destinations: [
        { address: "0x2222222222222222222222222222222222222222", label: "cold" },
      ],
      max_step_native_wei_hex: "0x1",
      max_plan_native_wei_hex: null,
      require_simulation: true,
      block_cross_party_linkage: true,
      allow_claim_execution: true,
      allow_gas_topups: true,
      max_gas_topup_wei_hex: "0x4",
      simulation_freshness_secs: 120,
      hot_floor_wei_hex: "0x2",
      hot_target_wei_hex: "0x3",
      hot_overflow_wei_hex: "0x5",
      allow_treasury_automation: true,
    },
  });
});

test("setup wizard can defer payer-linkage protection without policy update", () => {
  const dom = installDom(["wizLinkageChoiceStatus"]);
  dom.el("wizLinkageChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  wizard.wizDeclineLinkageProtection();

  equal(
    calls.some((call) => call.path === "/api/treasury/policy/update"),
    false,
  );
  equal(dom.el("wizLinkageChoiceStatus").classList.contains("hidden"), false);
  ok((dom.el("wizLinkageChoiceStatus").textContent || "").length > 0);
  deepEqual(toasts.pop(), {
    message: "You can enable payer-linkage protection later in Treasury policy.",
    type: undefined,
  });
});

test("setup wizard enables merkle claim execution opt-in from done step", async () => {
  const dom = installDom(["wizClaimExecutionChoiceStatus"]);
  dom.el("wizClaimExecutionChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy") return { policy: null };
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  await wizard.wizEnableClaimExecution();

  deepEqual(calls, [
    {
      method: "GET",
      path: "/api/treasury/policy",
      body: undefined,
    },
    {
      method: "POST",
      path: "/api/treasury/policy/update",
      body: { enabled: false, allow_claim_execution: true },
    },
  ]);
  equal(dom.el("wizClaimExecutionChoiceStatus").classList.contains("hidden"), false);
  equal(
    dom.el("wizClaimExecutionChoiceStatus").textContent,
    "Claim execution opt-in recorded. Claims still cannot run until the Treasury policy is enabled and each claim passes simulation, has a trusted or reviewed claim contract in the risk catalog, and is explicitly approved.",
  );
  deepEqual(toasts.pop(), {
    message: "Merkle claim execution opt-in recorded",
    type: undefined,
  });
});

test("setup wizard can defer merkle claim execution without policy update", () => {
  const dom = installDom(["wizClaimExecutionChoiceStatus"]);
  dom.el("wizClaimExecutionChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  wizard.wizDeclineClaimExecution();

  equal(
    calls.some((call) => call.path === "/api/treasury/policy/update"),
    false,
  );
  equal(dom.el("wizClaimExecutionChoiceStatus").classList.contains("hidden"), false);
  equal(
    dom.el("wizClaimExecutionChoiceStatus").textContent,
    "You can enable Merkle claim execution later in Treasury policy.",
  );
  deepEqual(toasts.pop(), {
    message: "You can enable Merkle claim execution later in Treasury policy.",
    type: undefined,
  });
});

test("setup wizard enables sponsor gas top-ups from done step", async () => {
  const dom = installDom(["wizGasTopupsChoiceStatus"]);
  dom.el("wizGasTopupsChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy") return { policy: null };
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  await wizard.wizEnableGasTopups();

  deepEqual(calls, [
    {
      method: "GET",
      path: "/api/treasury/policy",
      body: undefined,
    },
    {
      method: "POST",
      path: "/api/treasury/policy/update",
      body: { enabled: false, allow_gas_topups: true },
    },
  ]);
  equal(dom.el("wizGasTopupsChoiceStatus").classList.contains("hidden"), false);
  equal(
    dom.el("wizGasTopupsChoiceStatus").textContent,
    "Sponsor gas top-up opt-in recorded. Top-ups only appear inside reviewed consolidation plans, are capped by the Treasury policy, and cross-party sponsor funding is still linkage-checked.",
  );
  deepEqual(toasts.pop(), {
    message: "Sponsor gas top-up opt-in recorded",
    type: undefined,
  });
});

test("setup wizard can defer sponsor gas top-ups without policy update", () => {
  const dom = installDom(["wizGasTopupsChoiceStatus"]);
  dom.el("wizGasTopupsChoiceStatus").classList.add("hidden");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];

  const wizard = createSetupWizard({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    submitNewFido2Pin: async () => undefined,
    friendlyFidoError: (message) => String(message),
  });

  wizard.wizDeclineGasTopups();

  equal(
    calls.some((call) => call.path === "/api/treasury/policy/update"),
    false,
  );
  equal(dom.el("wizGasTopupsChoiceStatus").classList.contains("hidden"), false);
  equal(
    dom.el("wizGasTopupsChoiceStatus").textContent,
    "You can enable sponsor gas top-ups later in Treasury policy.",
  );
  deepEqual(toasts.pop(), {
    message: "You can enable sponsor gas top-ups later in Treasury policy.",
    type: undefined,
  });
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
        last_activity_block: 123456,
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
  ok(dom.el("inventoryAddressList").innerHTML.includes("lastActivityBlock=123456"));
  ok(dom.el("inventoryAddressList").innerHTML.includes("dormant_candidate"));
  ok(dom.el("inventoryHoldingList").innerHTML.includes("native"));
  ok(dom.el("inventoryHoldingList").innerHTML.includes("0xpermit2"));

  inventory.renderConsolidationPlans([
    {
      id: "plan-1",
      status: "review_required",
      chain_id: 1,
      summary: {
        total_steps: 2,
        blocked_steps: 0,
        review_required_steps: 1,
        approved_steps: 0,
        executable_steps: 0,
        value_items: 1,
      },
      created_at_unix: 1,
      updated_at_unix: 2,
      linkage_findings: ["Destination 0xdeadbe... links 2 payers: Acme, Bob"],
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
          exit_token0_address: "0xtoken0",
          exit_token1_address: "0xtoken1",
          exit_amount0_min_hex: "0x10",
          exit_amount1_min_hex: "0x20",
          exit_deadline_unix: 123456,
          signer_status: "available",
          simulation_status: "required",
          simulation_evidence: ["rpc_method=eth_call"],
          risk_level: "high",
          blockers: [],
          linkage_warnings: ["shared destination links this payer with: Bob"],
          auto_eligible: false,
          approved: false,
        },
        {
          id: "step-gas",
          sequence: 0,
          depends_on: ["step-1"],
          action: "fund_gas",
          status: "review_required",
          wallet_family: "eth-seed",
          wallet_profile: "archive",
          provider_profile: "mainnet",
          chain_id: 1,
          address: "0xsponsor",
          derivation_path: "m/44'/60'/0'/0/99",
          asset_kind: "native",
          asset_address: null,
          amount_hex: "0x123",
          destination_address: "0xfunded",
          signer_status: "available",
          simulation_status: "passed",
          simulation_evidence: ["fee_basis=dependent_step"],
          risk_level: "medium",
          blockers: [],
          linkage_warnings: [],
          auto_eligible: false,
          approved: false,
        },
      ],
    },
  ]);
  ok(dom.el("consolidationPlanList").innerHTML.includes("revoke_erc20_approval"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("0xspender"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("token0=0xtoken0"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("amount0Min=0x10"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("deadline=123456"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("simulation=required"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("rpc_method=eth_call"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("fund_gas"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("sponsor=0xsponsor"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("funds=0xfunded"));
  // Native top-up humanizes to ETH; the raw hex stays behind the "raw" details.
  ok(
    dom
      .el("consolidationPlanList")
      .innerHTML.includes("topup=0.000000000000000291 ETH"),
  );
  ok(dom.el("consolidationPlanList").innerHTML.includes(">0x123</code>"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("simulateConsolidationPlan"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("exportConsolidationPlan"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("Safe JSON"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("Call JSON"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("would link payers"));
  ok(dom.el("consolidationPlanList").innerHTML.includes("Scope: flags payers"));
  ok(
    dom
      .el("consolidationPlanList")
      .innerHTML.includes("shared destination links this payer with: Bob"),
  );
  // No treasury policy loaded: no Execute affordance may render.
  ok(!dom.el("consolidationPlanList").innerHTML.includes("enqueuePlanStep"));
  ok(!dom.el("consolidationPlanList").innerHTML.includes("Execute All Eligible"));
});

test("format helpers humanize amounts, quantities, timestamps, and chains", () => {
  installDom();

  // BigInt-safe token amounts at various decimals.
  equal(formatTokenAmount("0xde0b6b3a7640000", 18), "1");
  equal(formatTokenAmount("0x" + (1500000000000000000n).toString(16), 18), "1.5");
  equal(formatTokenAmount("0x123", 18), "0.000000000000000291");
  equal(formatTokenAmount("0xf4240", 6), "1");
  equal(formatTokenAmount("0x" + (25000000n).toString(16), 6), "25");
  equal(formatTokenAmount("0x0", 18), "0");
  equal(formatTokenAmount("nonsense"), null);
  equal(formatTokenAmount(""), null);
  equal(formatTokenAmount("0x"), null);
  equal(formatTokenAmount(null), null);
  equal(formatTokenAmount(undefined), null);

  equal(formatEthAmount("0xde0b6b3a7640000"), "1");
  equal(formatEthAmount("0xde0b6b3a7640000", "ETH"), "1 ETH");
  equal(formatEthAmount("not-hex", "ETH"), null);

  equal(formatHexQuantity("0x5208"), "21000");
  equal(formatHexQuantity("0x0"), "0");
  equal(formatHexQuantity("nope"), null);
  equal(formatHexQuantity(null), null);

  // Timestamps share the single locale formatter; falsy stays a placeholder.
  equal(formatTimestamp(0), "-");
  equal(formatTimestamp(null), "-");
  equal(formatTimestamp(undefined), "-");
  equal(formatTimestamp(1717900000), new Date(1717900000 * 1000).toLocaleString());

  // Chain ids resolve via the registry, else fall back to "Chain N".
  const chains = [
    { name: "ethereum", chain_id: 1, enabled: true },
    { name: "retired", chain_id: 2, enabled: false },
  ] as any;
  equal(chainLabel(1, chains), "1 (ethereum)");
  equal(chainLabel("1", chains), "1 (ethereum)");
  equal(chainLabel(2, chains), "Chain 2", "disabled profiles do not resolve");
  equal(chainLabel(56, chains), "Chain 56");
  equal(chainLabel(1, []), "Chain 1");
  equal(chainLabel(null, chains), "-");
  equal(chainLabel(undefined, chains), "-");

  // Raw values stay one click away behind the "raw" details affordance.
  const amountHtml = amountWithRawHtml("0xde0b6b3a7640000", { symbol: "ETH" });
  ok(amountHtml.includes("1 ETH"));
  ok(amountHtml.includes('<details class="raw-details">'));
  ok(amountHtml.includes(">0xde0b6b3a7640000</code>"));
  equal(amountWithRawHtml(undefined, { symbol: "ETH" }), "-");
  const quantityHtml = quantityWithRawHtml("0x5208");
  ok(quantityHtml.includes("21000"));
  ok(quantityHtml.includes(">0x5208</code>"));
  equal(quantityWithRawHtml(undefined), "-");
});

test("inventory views humanize balances, amounts, and timestamps", async () => {
  const dom = installDom([
    "chainProfileList",
    "inventoryJobList",
    "inventoryAddressList",
    "inventoryHoldingList",
    "watchAddressBookList",
    "tokenRegistryList",
    "riskCatalogList",
    "riskFindingList",
    "consolidationPlanList",
    "nftMetaOptInList",
    "nftMetadataList",
    "nftSuspiciousList",
  ]);
  const onePointFiveEthHex = "0x" + (1500000000000000000n).toString(16);
  const quarterEthHex = "0x" + (250000000000000000n).toString(16);
  const twentyFiveUsdcHex = "0x" + (25000000n).toString(16);
  const inventory = createInventoryActions({
    api: async (_method, path) => {
      if (path === "/api/chains") {
        return {
          profiles: [
            {
              name: "ethereum",
              chain_family: "evm",
              chain_id: 1,
              provider_profile: null,
              native_symbol: "ETH",
              native_decimals: 18,
              finality_blocks: 0,
              permit2_address: null,
              uniswap_v2_router_address: null,
              capabilities: [],
              enabled: true,
              source: "builtin",
              builtin: true,
            },
          ],
        };
      }
      if (path === "/api/inventory/wallets") {
        return {
          jobs: [],
          addresses: [
            {
              address: "0xabc",
              activity_state: "funded",
              wallet_family: "eth-seed",
              wallet_profile: "archive",
              chain_id: 1,
              derivation_path: "m/44'/60'/0'/0/0",
              native_balance_wei_hex: onePointFiveEthHex,
              transaction_count: 3,
            },
          ],
          holdings: [
            {
              asset_kind: "native",
              status: "active",
              address: "0xabc",
              amount_hex: quarterEthHex,
              wallet_family: "eth-seed",
              wallet_profile: "archive",
              provider_profile: "mainnet",
              chain_id: 1,
            },
            {
              asset_kind: "erc20",
              status: "active",
              address: "0xabc",
              asset_address: "0xToken",
              amount_hex: twentyFiveUsdcHex,
              wallet_family: "eth-seed",
              wallet_profile: "archive",
              provider_profile: "mainnet",
              chain_id: 1,
            },
            {
              asset_kind: "erc20",
              status: "active",
              address: "0xabc",
              asset_address: "0xUnknownToken",
              amount_hex: "0xff",
              wallet_family: "eth-seed",
              wallet_profile: "archive",
              provider_profile: "mainnet",
              chain_id: 1,
            },
          ],
        };
      }
      if (path === "/api/inventory/token-registry") {
        return {
          lists: [
            {
              id: "list-1",
              name: "default",
              compartment_id: 1,
              source: "operator",
              entries: [{ chain_id: 1, address: "0xtoken", symbol: "USDC", decimals: 6 }],
              created_at_unix: 1,
              updated_at_unix: 1717900000,
            },
          ],
        };
      }
      if (path === "/api/inventory/watch-addresses") {
        return {
          entries: [
            {
              id: "watch-1",
              address: "0xwatch",
              label: "vault",
              tags: [],
              source: "operator",
              enabled: true,
              created_at_unix: 1,
              updated_at_unix: 1717900000,
            },
          ],
        };
      }
      if (path === "/api/inventory/nft-metadata/opt-ins") {
        return {
          opt_ins: [
            { chain_id: 1, contract_address: "0xnft", enabled: true, updated_at_unix: 1717900000 },
          ],
        };
      }
      return { entries: [], findings: [], plans: [] };
    },
    toast: () => undefined,
    downloadJson: () => undefined,
  });

  await inventory.loadInventoryOperations();

  // Native balances render as ETH with the raw wei behind "raw" details.
  const addressHtml = dom.el("inventoryAddressList").innerHTML;
  ok(addressHtml.includes("native=1.5 ETH"));
  ok(addressHtml.includes(">" + onePointFiveEthHex + "</code>"));
  ok(!addressHtml.includes("native=" + onePointFiveEthHex + " ·"));

  // Holdings: native → ETH, registry-known token → token units, unknown
  // token keeps the raw hex (decimals are never guessed).
  const holdingHtml = dom.el("inventoryHoldingList").innerHTML;
  ok(holdingHtml.includes("amount=0.25 ETH"));
  ok(holdingHtml.includes(">" + quarterEthHex + "</code>"));
  ok(holdingHtml.includes("amount=25 USDC"));
  ok(holdingHtml.includes(">" + twentyFiveUsdcHex + "</code>"));
  ok(holdingHtml.includes("amount=0xff"));

  // Raw unix seconds become locale timestamps everywhere they were shown.
  const humanTs = formatTimestamp(1717900000);
  ok(dom.el("tokenRegistryList").innerHTML.includes("updated=" + humanTs));
  ok(!dom.el("tokenRegistryList").innerHTML.includes("updated=1717900000"));
  ok(dom.el("watchAddressBookList").innerHTML.includes("updated=" + humanTs));
  ok(!dom.el("watchAddressBookList").innerHTML.includes("updated=1717900000"));
  const optInHtml = dom.el("nftMetaOptInList").innerHTML;
  ok(optInHtml.includes("updated=" + humanTs));
  ok(!optInHtml.includes("updated=1717900000"));
  ok(optInHtml.includes("chain=1 (ethereum)"));
});

test("operations views humanize deposit amounts and queue gas used", () => {
  const dom = installDom(["depositList", "queueList"]);
  const operations = createOperationsActions({
    api: async () => ({}),
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  const oneEthHex = "0x" + (1000000000000000000n).toString(16);
  operations.renderDeposits([
    {
      id: "dep-native",
      status: "observed",
      asset_kind: "native",
      wallet_profile: "daily",
      short_name: "dep-1",
      stealth_address: "0xstealth",
      ephemeral_public_key_hex: "0xephem",
      view_tag_hex: "0x01",
      expected_amount_hex: oneEthHex,
      observed_amount_hex: "0x" + (500000000000000000n).toString(16),
      observed_native_balance_wei_hex: oneEthHex,
      auto_queue_sweep: true,
      created_at_unix: 1,
      updated_at_unix: 2,
    },
    {
      id: "dep-token",
      status: "observed",
      asset_kind: "erc20",
      wallet_profile: "daily",
      short_name: "dep-2",
      stealth_address: "0xstealth2",
      ephemeral_public_key_hex: "0xephem2",
      view_tag_hex: "0x02",
      token_address: "0xtoken",
      expected_amount_hex: "0xff",
      auto_queue_sweep: false,
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  const depositsHtml = dom.el("depositList").innerHTML;
  ok(depositsHtml.includes("expected=1 ETH"));
  ok(depositsHtml.includes("observed=0.5 ETH"));
  ok(depositsHtml.includes("native=1 ETH"));
  ok(depositsHtml.includes(">" + oneEthHex + "</code>"));
  // ERC-20 amounts stay raw: this view has no registry decimals loaded.
  ok(depositsHtml.includes("expected=0xff"));

  operations.renderQueueJobs([
    {
      id: "job-1",
      state: "confirmed",
      kind: "eth_stealth_native_sweep",
      wallet_profile: "daily",
      transaction_hash_hex: "0xtx",
      receipt_status: "success",
      receipt_gas_used_hex: "0x5208",
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  const queueHtml = dom.el("queueList").innerHTML;
  ok(queueHtml.includes("gasUsed=21000"));
  ok(queueHtml.includes(">0x5208</code>"));
});

test("plan execute affordances render only when gates pass and drive enqueue routes", async () => {
  const dom = installDom([
    "chainProfileList",
    "inventoryJobList",
    "inventoryAddressList",
    "inventoryHoldingList",
    "watchAddressBookList",
    "riskCatalogList",
    "riskFindingList",
    "consolidationPlanList",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: string[] = [];
  const nowSecs = Math.floor(Date.now() / 1000);
  const expectedPhrase = "EXECUTE 1 PLAN STEPS TOTAL 1000000000000000000 WEI";
  const eligibleStep = {
    id: "step-exec",
    sequence: 0,
    action: "sweep_native",
    status: "approved",
    wallet_family: "eth-seed",
    wallet_profile: "archive",
    provider_profile: "mainnet",
    chain_id: 1,
    address: "0xabc",
    derivation_path: "m/44'/60'/0'/0/0",
    asset_kind: "native",
    amount_hex: "0xde0b6b3a7640000",
    destination_address: "0xdest",
    signer_status: "available",
    simulation_status: "passed",
    simulation_evidence: [
      "fee_basis=static_profile",
      "simulated_at_unix=" + String(nowSecs),
    ],
    risk_level: "low",
    blockers: [],
    linkage_warnings: [],
    auto_eligible: true,
    approved: true,
  };
  const staleStep = {
    ...eligibleStep,
    id: "step-stale",
    simulation_evidence: ["simulated_at_unix=1"],
  };
  const queuedStep = { ...eligibleStep, id: "step-queued", queued_job_id: "job-1" };
  const plans = {
    plans: [
      {
        id: "plan-exec",
        status: "approved",
        chain_id: 1,
        created_at_unix: 1,
        updated_at_unix: 2,
        summary: {
          total_steps: 3,
          blocked_steps: 0,
          review_required_steps: 0,
          approved_steps: 3,
          executable_steps: 3,
          value_items: 3,
        },
        steps: [eligibleStep, staleStep, queuedStep],
      },
    ],
  };
  const policy = {
    policy: {
      enabled: true,
      execution_paused: false,
      allow_plan_execution: true,
      allow_sweep_execution: true,
      simulation_freshness_secs: 900,
    },
  };
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/plans/consolidation") return plans;
      if (path === "/api/treasury/policy") return policy;
      if (path === "/api/plans/enqueue-step") {
        return { status: "queued", job: { id: "job-new" } };
      }
      if (path === "/api/plans/enqueue-plan") {
        if ((body as any)?.confirmation !== expectedPhrase) {
          return {
            error:
              'confirmation_mismatch: type the exact phrase "' + expectedPhrase + '"',
            action: expectedPhrase,
          };
        }
        return {
          status: "queued",
          enqueued: [{ step_id: "step-exec", job_id: "job-new" }],
          skipped: [],
        };
      }
      if (path === "/api/inventory/wallets") {
        return { jobs: [], addresses: [], holdings: [] };
      }
      if (path === "/api/chains") return { profiles: [] };
      return { entries: [], findings: [], plans: [], lists: [] };
    },
    toast: (message) => toasts.push(message),
    downloadJson: () => undefined,
  });

  await inventory.loadInventoryOperations();
  const html = dom.el("consolidationPlanList").innerHTML;
  // Exactly one step passes every gate: fresh passed simulation, approved,
  // unblocked, policy gates on, not yet enqueued.
  equal(html.split('data-action="enqueuePlanStep"').length - 1, 1);
  ok(html.includes('data-arg1="step-exec"'));
  ok(html.includes("Execute All Eligible"));
  ok(html.includes("queuedJob=job-1"));

  // Single-step enqueue asks once, then posts the explicit confirm flag.
  const stepPending = inventory.enqueuePlanStep("plan-exec", "step-exec");
  await answerConfirm("action");
  await stepPending;
  deepEqual(
    calls.find((call) => call.path === "/api/plans/enqueue-step"),
    {
      method: "POST",
      path: "/api/plans/enqueue-step",
      body: { plan_id: "plan-exec", step_id: "step-exec", confirm: true },
    },
  );

  // Bulk enqueue probes for the exact daemon-computed phrase, renders it in
  // the typed-confirmation dialog, and submits only after the operator types
  // it. A mistyped phrase keeps the danger button disabled and never reaches
  // the daemon.
  const bulkPending = inventory.enqueuePlanBulk("plan-exec");
  await tick();
  equal(confirmPart("[data-confirm-phrase]")?.textContent, expectedPhrase);
  typeConfirmPhrase("EXECUTE 9 PLAN STEPS TOTAL 1 WEI");
  equal(confirmPart("[data-confirm-action]").disabled, true);
  typeConfirmPhrase(expectedPhrase);
  equal(confirmPart("[data-confirm-action]").disabled, false);
  confirmPart("[data-confirm-action]").click();
  await bulkPending;
  const bulkCalls = calls.filter((call) => call.path === "/api/plans/enqueue-plan");
  equal(bulkCalls.length, 2);
  equal((bulkCalls[0].body as any).confirmation, "");
  equal((bulkCalls[1].body as any).confirmation, expectedPhrase);
  ok(toasts.some((message) => message.includes("Enqueued 1 step(s)")));

  // Cancelling the typed dialog stops after the probe: nothing is enqueued.
  const cancelledBulk = inventory.enqueuePlanBulk("plan-exec");
  await answerConfirm("cancel");
  await cancelledBulk;
  equal(calls.filter((call) => call.path === "/api/plans/enqueue-plan").length, 3);
});

test("chain profile UI renders registry fields and uses chain routes", async () => {
  const dom = installDom([
    "chainProfileList",
    "inventoryJobList",
    "inventoryAddressList",
    "inventoryHoldingList",
    "watchAddressBookList",
    "riskCatalogList",
    "riskFindingList",
    "consolidationPlanList",
    "chainProfileName",
    "chainProfileFamily",
    "chainProfileId",
    "chainProfileProvider",
    "chainProfileNativeSymbol",
    "chainProfileNativeDecimals",
    "chainProfileFinalityBlocks",
    "chainProfilePermit2Address",
    "chainProfileUniswapV2Router",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: string[] = [];
  const chains = {
    profiles: [
      {
        name: "ethereum",
        chain_family: "evm",
        chain_id: 1,
        provider_profile: null,
        native_symbol: "ETH",
        native_decimals: 18,
        finality_blocks: 0,
        permit2_address: null,
        uniswap_v2_router_address: null,
        capabilities: [],
        enabled: true,
        source: "builtin",
        builtin: true,
      },
      {
        name: "test-rollup",
        chain_family: "evm",
        chain_id: 999999,
        provider_profile: "rollup-rpc",
        native_symbol: "TST",
        native_decimals: 18,
        finality_blocks: 64,
        permit2_address: "0x5555555555555555555555555555555555555555",
        uniswap_v2_router_address: "0x6666666666666666666666666666666666666666",
        capabilities: ["erc20"],
        enabled: true,
        source: "operator",
        builtin: false,
      },
    ],
  };
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/chains") return chains;
      if (path === "/api/inventory/wallets") {
        return {
          jobs: [],
          addresses: [
            {
              address: "0xabc",
              activity_state: "funded",
              wallet_family: "eth-seed",
              wallet_profile: "archive",
              provider_profile: "mainnet",
              chain_id: 1,
              derivation_path: "m/44'/60'/0'/0/0",
              native_balance_wei_hex: "0x1",
              transaction_count: 1,
              classifications: [],
            },
          ],
          holdings: [],
        };
      }
      if (path === "/api/chains/upsert" || path === "/api/chains/delete") {
        return { status: "ok" };
      }
      return { entries: [], findings: [], plans: [] };
    },
    toast: (message) => toasts.push(message),
    downloadJson: () => undefined,
  });

  inventory.renderChainProfiles(chains.profiles);
  const chainHtml = dom.el("chainProfileList").innerHTML;
  ok(chainHtml.includes("builtin"));
  ok(chainHtml.includes("finality=64"));
  ok(chainHtml.includes("0x5555555555555555555555555555555555555555"));
  ok(chainHtml.includes("univ2Router=0x6666666666666666666666666666666666666666"));
  equal(chainHtml.split('data-action="deleteChainProfile"').length - 1, 1);

  await inventory.loadInventoryOperations();
  ok(calls.some((call) => call.method === "GET" && call.path === "/api/chains"));
  ok(dom.el("inventoryAddressList").innerHTML.includes("chain=1 (ethereum)"));

  dom.el("chainProfileName").value = "custom-rollup";
  dom.el("chainProfileFamily").value = "evm";
  dom.el("chainProfileId").value = "777";
  dom.el("chainProfileProvider").value = "custom-rpc";
  dom.el("chainProfileNativeSymbol").value = "CST";
  dom.el("chainProfileNativeDecimals").value = "18";
  dom.el("chainProfileFinalityBlocks").value = "32";
  dom.el("chainProfilePermit2Address").value = "0x5555555555555555555555555555555555555555";
  dom.el("chainProfileUniswapV2Router").value = "0x6666666666666666666666666666666666666666";
  await inventory.upsertChainProfile();
  deepEqual(
    calls.find((call) => call.path === "/api/chains/upsert"),
    {
      method: "POST",
      path: "/api/chains/upsert",
      body: {
        name: "custom-rollup",
        chain_family: "evm",
        chain_id: 777,
        provider_profile: "custom-rpc",
        native_symbol: "CST",
        native_decimals: 18,
        finality_blocks: 32,
        permit2_address: "0x5555555555555555555555555555555555555555",
        uniswap_v2_router_address: "0x6666666666666666666666666666666666666666",
        capabilities: [],
        enabled: true,
      },
    },
  );

  const deleteChainPending = inventory.deleteChainProfile("test-rollup");
  await answerConfirm("action");
  await deleteChainPending;
  deepEqual(
    calls.find((call) => call.path === "/api/chains/delete"),
    {
      method: "POST",
      path: "/api/chains/delete",
      body: { name: "test-rollup" },
    },
  );
  ok(toasts.includes("Chain profile saved"));
  ok(toasts.includes("Chain profile deleted"));
});

test("operation results include failure cause breakdowns", async () => {
  installDom([
    "queueProcessLimit",
    "maintenanceDepositLimit",
    "maintenanceQueueLimit",
    "maintenanceAutoEnqueue",
    "depositList",
    "queueList",
  ]);
  const resultBoxes: Record<string, string> = {};
  const operations = createOperationsActions({
    api: async (_method, path) => {
      if (path === "/api/queue/process") {
        return {
          processed: 4,
          succeeded: 0,
          blocked: 1,
          retrying: 1,
          operator_action_required: 0,
          failed: 2,
          failures_by_cause: {
            provider_error: 1,
            policy_block: 1,
            insufficient_gas: 1,
            validation: 1,
            unknown: 0,
          },
          jobs: [],
        };
      }
      if (path === "/api/maintenance/run") {
        return {
          refreshed: 2,
          detected: 1,
          queued: 1,
          processed: 4,
          succeeded: 0,
          blocked: 1,
          retrying: 1,
          operator_action_required: 0,
          failed: 2,
          failures_by_cause: {
            provider_error: 1,
            policy_block: 1,
            insufficient_gas: 1,
            validation: 1,
            unknown: 0,
          },
          treasury_automation: {
            generated_steps: 2,
            enqueued_steps: 1,
            skipped_steps: 1,
            skipped_reasons: ["simulation_not_passed"],
          },
          deposits: [],
          jobs: [],
        };
      }
      if (path === "/api/deposits/eth-stealth") {
        return { deposits: [] };
      }
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: (id, html) => {
      resultBoxes[id] = html;
    },
    updateNextStepCard: () => undefined,
  });

  const batchPending = operations.processQueueBatch();
  await answerConfirm("action");
  await batchPending;
  ok(resultBoxes.queueProcessResult.includes("failures_by_cause"));
  ok(resultBoxes.queueProcessResult.includes("provider_error=1"));
  ok(resultBoxes.queueProcessResult.includes("policy_block=1"));
  ok(resultBoxes.queueProcessResult.includes("insufficient_gas=1"));
  ok(resultBoxes.queueProcessResult.includes("validation=1"));

  await operations.runMaintenanceCycle();
  ok(resultBoxes.maintenanceResult.includes("failures_by_cause"));
  ok(resultBoxes.maintenanceResult.includes("provider_error=1"));
  ok(resultBoxes.maintenanceResult.includes("policy_block=1"));
  ok(resultBoxes.maintenanceResult.includes("insufficient_gas=1"));
  ok(resultBoxes.maintenanceResult.includes("validation=1"));
  ok(resultBoxes.maintenanceResult.includes("automationGenerated=2"));
  ok(resultBoxes.maintenanceResult.includes("automationEnqueued=1"));
  ok(resultBoxes.maintenanceResult.includes("automationSkipped=1"));
});

test("processQueueBatch surfaces a mid-drain pause reason in the result line", async () => {
  installDom(["queueProcessLimit", "queueList"]);
  const resultBoxes: Record<string, string> = {};
  const operations = createOperationsActions({
    api: async (_method, path) => {
      if (path === "/api/queue/process") {
        return {
          processed: 2,
          succeeded: 1,
          blocked: 0,
          retrying: 0,
          operator_action_required: 0,
          failed: 0,
          failures_by_cause: {
            provider_error: 0,
            policy_block: 0,
            insufficient_gas: 0,
            validation: 0,
            unknown: 0,
          },
          paused_reason:
            "execution_paused: queue execution is paused by the operator kill switch",
          jobs: [],
        };
      }
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: (id, html) => {
      resultBoxes[id] = html;
    },
    updateNextStepCard: () => undefined,
  });

  const pauseBatchPending = operations.processQueueBatch();
  await answerConfirm("action");
  await pauseBatchPending;
  ok(
    resultBoxes.queueProcessResult.includes(
      "paused: execution_paused: queue execution is paused by the operator kill switch",
    ),
  );
});

test("processQueueBatch can run in background and surfaces the operation id", async () => {
  const dom = installDom(["queueProcessLimit", "queueProcessRunAsync", "queueList"]);
  dom.el("queueProcessRunAsync").checked = true;
  const resultBoxes: Record<string, string> = {};
  const toasts: string[] = [];
  let requestBody: any = null;
  const operations = createOperationsActions({
    api: async (_method, path, body) => {
      if (path === "/api/queue/process") {
        requestBody = body;
        return {
          processed: 0,
          succeeded: 0,
          jobs: [],
          operation: { id: "op-q1", kind: "queue_process", state: "running" },
        };
      }
      if (path === "/api/queue/jobs") return { jobs: [] };
      if (path === "/api/treasury/policy") return {};
      return {};
    },
    toast: (message: string) => {
      toasts.push(message);
    },
    refresh: () => undefined,
    showResultBox: (id, html) => {
      resultBoxes[id] = html;
    },
    updateNextStepCard: () => undefined,
  });

  // The Phase 0 confirm dialog still gates the background submission.
  const pending = operations.processQueueBatch();
  await answerConfirm("action");
  await pending;

  equal(requestBody.run_async, true);
  equal(requestBody.id, null);
  ok(
    toasts.some((message) => message.includes("operation op-q1")),
    "toast surfaces the operation id: " + toasts.join(" | "),
  );
  equal(
    resultBoxes.queueProcessResult,
    undefined,
    "background mode does not render a synchronous tally box",
  );
});

test("runMaintenanceCycle sends run_async only in background mode and surfaces the operation id", async () => {
  const dom = installDom([
    "maintenanceDepositLimit",
    "maintenanceQueueLimit",
    "maintenanceAutoEnqueue",
    "maintenanceRunAsync",
    "depositList",
    "queueList",
  ]);
  const resultBoxes: Record<string, string> = {};
  const toasts: string[] = [];
  const requestBodies: any[] = [];
  const operations = createOperationsActions({
    api: async (_method, path, body) => {
      if (path === "/api/maintenance/run") {
        requestBodies.push(body);
        if ((body as any)?.run_async === true) {
          return {
            status: "accepted",
            operation: { id: "op-m1", kind: "maintenance_run", state: "running" },
          };
        }
        return {
          status: "ok",
          refreshed: 0,
          detected: 0,
          queued: 0,
          processed: 0,
          succeeded: 0,
          failed: 0,
          failures_by_cause: {},
          deposits: [],
          jobs: [],
        };
      }
      if (path === "/api/queue/jobs") return { jobs: [] };
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      if (path === "/api/treasury/policy") return {};
      return {};
    },
    toast: (message: string) => {
      toasts.push(message);
    },
    refresh: () => undefined,
    showResultBox: (id, html) => {
      resultBoxes[id] = html;
    },
    updateNextStepCard: () => undefined,
  });

  // Default: synchronous run, run_async stays absent from the request.
  await operations.runMaintenanceCycle();
  equal(
    requestBodies[0]?.run_async,
    undefined,
    "run_async stays absent unless the background checkbox is checked",
  );
  ok(resultBoxes.maintenanceResult !== undefined, "sync run renders the tally box");

  dom.el("maintenanceRunAsync").checked = true;
  await operations.runMaintenanceCycle();
  equal(requestBodies[1]?.run_async, true);
  ok(
    toasts.some((message) => message.includes("operation op-m1")),
    "toast surfaces the operation id: " + toasts.join(" | "),
  );
});

test("queue process batch and single job require confirmation before broadcast", async () => {
  const dom = installDom(["queueProcessLimit", "queueList", "depositList"]);
  dom.el("queueProcessLimit").value = "20";
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const operations = createOperationsActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/queue/process") {
        return { processed: 1, succeeded: 1, jobs: [] };
      }
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });

  // Cancelling the batch dialog never touches the process route.
  let pending: Promise<void> = operations.processQueueBatch();
  await tick();
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes(
      "Process up to 20 queued jobs now?",
    ),
    "batch dialog states the job count and consequence",
  );
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes("signed and broadcast"),
    "batch dialog states the broadcast consequence",
  );
  await answerConfirm("cancel");
  await pending;
  equal(calls.filter((call) => call.path === "/api/queue/process").length, 0);

  // Confirming posts the batch drain with the operator's limit.
  pending = operations.processQueueBatch();
  await answerConfirm("action");
  await pending;
  deepEqual(calls.find((call) => call.path === "/api/queue/process"), {
    method: "POST",
    path: "/api/queue/process",
    body: { id: null, limit: 20 },
  });

  // Single-job processing is guarded the same way.
  pending = operations.processQueueJob("job-7");
  await answerConfirm("cancel");
  await pending;
  equal(calls.filter((call) => call.path === "/api/queue/process").length, 1);

  pending = operations.processQueueJob("job-7");
  await tick();
  ok(confirmPart("[data-confirm-body]")?.textContent.includes('"job-7"'));
  await answerConfirm("action");
  await pending;
  deepEqual(calls.filter((call) => call.path === "/api/queue/process")[1], {
    method: "POST",
    path: "/api/queue/process",
    body: { id: "job-7", limit: 1 },
  });
});

test("deposit sweep enqueue and deposit delete require confirmation", async () => {
  installDom(["depositList", "queueList", "depositRefreshResult"]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const operations = createOperationsActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/deposits/eth-stealth/enqueue-sweep") {
        return { status: "queued", job: { id: "job-sweep-1" } };
      }
      return { status: "ok", deposits: [], jobs: [] };
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });

  let pending: Promise<void> = operations.enqueueDepositSweep("dep-1");
  await answerConfirm("cancel");
  await pending;
  equal(
    calls.filter((call) => call.path === "/api/deposits/eth-stealth/enqueue-sweep").length,
    0,
  );

  pending = operations.enqueueDepositSweep("dep-1");
  await tick();
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes("signed and broadcast"),
    "sweep dialog states the on-chain consequence",
  );
  await answerConfirm("action");
  await pending;
  deepEqual(
    calls.find((call) => call.path === "/api/deposits/eth-stealth/enqueue-sweep"),
    {
      method: "POST",
      path: "/api/deposits/eth-stealth/enqueue-sweep",
      body: { id: "dep-1" },
    },
  );

  pending = operations.deleteDeposit("dep-1");
  await answerConfirm("cancel");
  await pending;
  equal(
    calls.filter((call) => call.path === "/api/deposits/eth-stealth/delete").length,
    0,
  );

  pending = operations.deleteDeposit("dep-1");
  await answerConfirm("action");
  await pending;
  deepEqual(
    calls.find((call) => call.path === "/api/deposits/eth-stealth/delete"),
    {
      method: "POST",
      path: "/api/deposits/eth-stealth/delete",
      body: { id: "dep-1" },
    },
  );
});

test("deposit create surfaces stealth generation warnings as toasts and a pinned box", async () => {
  const dom = installDom([
    "depositNativeWalletProfile",
    "depositNativeExpected",
    "depositNativeMinSweep",
    "depositNativeDestination",
    "depositNativeNote",
    "depositNativeAutoQueue",
    "depositCreateWarnings",
  ]);
  dom.el("depositNativeWalletProfile").value = "stealth-main";
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const operations = createOperationsActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/deposits/eth-stealth/create-native") {
        return {
          status: "created",
          deposit: { id: "dep-1", stealth_address: "0xstealth1" },
          warnings: [
            "This meta-address does not match any of this vault's known stealth wallets.",
            "This ephemeral key was already used for an existing deposit.",
          ],
        };
      }
      return {};
    },
    toast: (message, type) => {
      toasts.push({ message, type });
    },
    refresh: () => undefined,
    showResultBox,
    updateNextStepCard: () => undefined,
  });

  await operations.createNativeDeposit();

  deepEqual(
    calls.map((call) => call.path),
    ["/api/deposits/eth-stealth/create-native"],
  );
  deepEqual(
    toasts.filter((toast) => toast.type === "warning").map((toast) => toast.message),
    [
      "This meta-address does not match any of this vault's known stealth wallets.",
      "This ephemeral key was already used for an existing deposit.",
    ],
  );
  ok(toasts.some((toast) => toast.message === "Native deposit created"));
  const box = dom.el("depositCreateWarnings");
  equal(box.classList.contains("hidden"), false);
  ok(box.innerHTML.includes("0xstealth1"));
  ok(box.innerHTML.includes("does not match any of this vault"));
  ok(box.innerHTML.includes("ephemeral key was already used"));
});

test("deposit create without stealth generation warnings shows no warning UI", async () => {
  const dom = installDom([
    "depositErc20WalletProfile",
    "depositErc20TokenAddress",
    "depositErc20Expected",
    "depositErc20MinSweep",
    "depositErc20Destination",
    "depositErc20Note",
    "depositErc20AutoQueue",
    "depositNativeWalletProfile",
    "depositNativeExpected",
    "depositNativeMinSweep",
    "depositNativeDestination",
    "depositNativeNote",
    "depositNativeAutoQueue",
    "depositCreateWarnings",
  ]);
  dom.el("depositErc20WalletProfile").value = "stealth-main";
  dom.el("depositErc20TokenAddress").value = "0xtoken";
  dom.el("depositNativeWalletProfile").value = "stealth-main";
  const toasts: Array<{ message: string; type?: string }> = [];
  const operations = createOperationsActions({
    api: async (_method, path) => {
      if (path === "/api/deposits/eth-stealth/create-erc20") {
        return { status: "created", deposit: { id: "dep-2" }, warnings: [] };
      }
      if (path === "/api/deposits/eth-stealth/create-native") {
        return { status: "created", deposit: { id: "dep-3" } };
      }
      return {};
    },
    toast: (message, type) => {
      toasts.push({ message, type });
    },
    refresh: () => undefined,
    showResultBox,
    updateNextStepCard: () => undefined,
  });

  await operations.createErc20Deposit();
  await operations.createNativeDeposit();

  equal(toasts.filter((toast) => toast.type === "warning").length, 0);
  const box = dom.el("depositCreateWarnings");
  equal(box.classList.contains("hidden"), true);
  equal(box.innerHTML, "");
});

test("deposit create sends request-gas fields when the payer-gas option is checked", async () => {
  const dom = installDom([
    "depositNativeWalletProfile",
    "depositNativeExpected",
    "depositNativeMinSweep",
    "depositNativeDestination",
    "depositNativeNote",
    "depositNativeAutoQueue",
    "depositNativeRequestGas",
    "depositNativeGasAmount",
    "depositErc20WalletProfile",
    "depositErc20TokenAddress",
    "depositErc20Expected",
    "depositErc20MinSweep",
    "depositErc20Destination",
    "depositErc20Note",
    "depositErc20AutoQueue",
    "depositErc20RequestGas",
    "depositErc20GasAmount",
    "depositCreateWarnings",
  ]);
  dom.el("depositNativeWalletProfile").value = "stealth-main";
  dom.el("depositNativeRequestGas").checked = true;
  dom.el("depositNativeGasAmount").value = "0x5208";
  dom.el("depositErc20WalletProfile").value = "stealth-main";
  dom.el("depositErc20TokenAddress").value = "0xtoken";
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const operations = createOperationsActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return { status: "created", deposit: { id: "dep-gas" }, warnings: [] };
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox,
    updateNextStepCard: () => undefined,
  });

  await operations.createNativeDeposit();
  await operations.createErc20Deposit();

  const nativeBody = calls.find(
    (call) => call.path === "/api/deposits/eth-stealth/create-native",
  )?.body;
  equal(nativeBody.request_gas, true);
  equal(nativeBody.gas_amount_wei_hex, "0x5208");
  const erc20Body = calls.find(
    (call) => call.path === "/api/deposits/eth-stealth/create-erc20",
  )?.body;
  // Unchecked: the request goes out with request_gas off and no gas amount.
  equal(erc20Body.request_gas, false);
  equal(erc20Body.gas_amount_wei_hex, null);
  // The gas amount field clears after a successful create.
  equal(dom.el("depositNativeGasAmount").value, "");
});

test("deposit rows surface requested gas, sponsor top-up state, and the needs-gas explainer", () => {
  const dom = installDom(["depositList", "queueList"]);
  const operations = createOperationsActions({
    api: async () => ({}),
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  operations.renderDeposits([
    {
      id: "dep-needs-gas-manual",
      status: "funded_needs_gas",
      asset_kind: "erc20",
      wallet_profile: "daily",
      short_name: "dep-1",
      stealth_address: "0xstealth1",
      ephemeral_public_key_hex: "0xephem1",
      view_tag_hex: "0x01",
      token_address: "0xtoken",
      requested_gas_wei_hex: "0x" + (42000000000000n).toString(16),
      observed_native_balance_wei_hex: "0x0",
      auto_queue_sweep: true,
      created_at_unix: 1,
      updated_at_unix: 2,
    },
    {
      id: "dep-needs-gas-sponsored",
      status: "funded_needs_gas",
      asset_kind: "erc20",
      wallet_profile: "daily",
      short_name: "dep-2",
      stealth_address: "0xstealth2",
      ephemeral_public_key_hex: "0xephem2",
      view_tag_hex: "0x02",
      token_address: "0xtoken",
      gas_topup_job_id: "job-topup-1",
      gas_topup_job_state: "sent",
      auto_queue_sweep: true,
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  const html = dom.el("depositList").innerHTML;
  // Requested payer gas humanizes to ETH.
  ok(html.includes("requested payer gas=0.000042 ETH"), html);
  // A gas-starved deposit without a sponsor explains the manual path.
  ok(html.includes("no native gas for the sweep"), html);
  ok(html.includes("fund the address manually"), html);
  // A sponsored deposit shows the top-up job state and what it waits for.
  ok(html.includes("sponsor top-up state=sent"), html);
  ok(html.includes("waiting for the sponsor gas top-up to confirm"), html);
  // A needs-gas status warns rather than reading as a healthy "funded" green.
  ok(html.includes('pill pill-warn">funded needs gas<'), html);
});

test("treasury party delete requires confirmation", async () => {
  installDom(["treasuryPartyList", "treasuryReceiveParty"]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/parties" && method === "GET") {
        return {
          parties: [{ id: "party-1", name: "Client One", created_at_unix: 1717900000 }],
        };
      }
      return { status: "ok", allocations: [] };
    },
    toast: () => undefined,
  });

  await treasury.loadTreasuryParties();
  calls.length = 0;

  let pending: Promise<void> = treasury.deleteTreasuryParty("party-1");
  await tick();
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes('"Client One"'),
    "party dialog names the counterparty being deleted",
  );
  await answerConfirm("cancel");
  await pending;
  equal(
    calls.filter((call) => call.path === "/api/treasury/parties/delete").length,
    0,
  );

  pending = treasury.deleteTreasuryParty("party-1");
  await answerConfirm("action");
  await pending;
  deepEqual(
    calls.find((call) => call.path === "/api/treasury/parties/delete"),
    {
      method: "POST",
      path: "/api/treasury/parties/delete",
      body: { id: "party-1" },
    },
  );
});

test("queue kill switch toggles the paused banner and buttons and re-loads the queue", async () => {
  const dom = installDom([
    "queueList",
    "queuePausedBanner",
    "queuePauseBtn",
    "queueResumeBtn",
  ]);
  const calls: Array<{ method: string; path: string }> = [];
  let executionPaused = false;
  const operations = createOperationsActions({
    api: async (method, path) => {
      calls.push({ method, path });
      if (path === "/api/queue/jobs") return { jobs: [] };
      if (path === "/api/treasury/policy") {
        return { policy: { execution_paused: executionPaused } };
      }
      if (path === "/api/queue/pause") {
        executionPaused = true;
        return { status: "paused", execution_paused: true };
      }
      if (path === "/api/queue/resume") {
        executionPaused = false;
        return { status: "resumed", execution_paused: false };
      }
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });

  await operations.loadQueueJobs();
  equal(dom.el("queuePausedBanner").classList.contains("hidden"), true);
  equal(dom.el("queuePauseBtn").classList.contains("hidden"), false);
  equal(dom.el("queueResumeBtn").classList.contains("hidden"), true);

  const toasts: Array<{ message: string; type?: string }> = [];
  const pausable = createOperationsActions({
    api: async (method, path) => {
      calls.push({ method, path });
      if (path === "/api/queue/jobs") return { jobs: [] };
      if (path === "/api/treasury/policy") {
        return { policy: { execution_paused: executionPaused } };
      }
      if (path === "/api/queue/pause") {
        executionPaused = true;
        return { status: "paused", execution_paused: true };
      }
      if (path === "/api/queue/resume") {
        executionPaused = false;
        return { status: "resumed", execution_paused: false };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });

  // pauseQueueExecution/resumeQueueExecution re-trigger loadQueueJobs with `void`
  // (fire-and-forget, matching this module's existing idiom), so flush the
  // microtask queue before asserting on its effects.
  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  const callsBeforePause = calls.length;
  await pausable.pauseQueueExecution();
  await flush();
  ok(calls.some((call) => call.path === "/api/queue/pause"));
  ok(calls.length > callsBeforePause + 1); // pause call plus the re-triggered loadQueueJobs fetches
  ok(toasts.some((t) => t.message === "Queue execution paused"));
  equal(dom.el("queuePausedBanner").classList.contains("hidden"), false);
  equal(dom.el("queuePauseBtn").classList.contains("hidden"), true);
  equal(dom.el("queueResumeBtn").classList.contains("hidden"), false);

  const callsBeforeResume = calls.length;
  await pausable.resumeQueueExecution();
  await flush();
  ok(calls.some((call) => call.path === "/api/queue/resume"));
  ok(calls.length > callsBeforeResume + 1);
  ok(toasts.some((t) => t.message === "Queue execution resumed"));
  equal(dom.el("queuePausedBanner").classList.contains("hidden"), true);
  equal(dom.el("queuePauseBtn").classList.contains("hidden"), false);
  equal(dom.el("queueResumeBtn").classList.contains("hidden"), true);
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

test("inventory consolidation generate submits per-party destinations", async () => {
  const dom = installDom([
    "planDestinationAddress",
    "planRoutingStrategy",
    "planPartyDestinations",
    "planPerPartyHint",
    "planPartyDest_party_acme",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (method === "GET" && path === "/api/treasury/parties") {
        return {
          parties: [{ id: "party_acme", name: "Acme", created_at_unix: 1 }],
        };
      }
      if (method === "POST" && path === "/api/plans/consolidation/generate") {
        return { status: "generated" };
      }
      return {};
    },
    toast: (message) => toasts.push(message),
    downloadJson: () => undefined,
  });

  dom.el("planRoutingStrategy").value = "per_party";
  await inventory.renderPlanPartyDestinations();
  ok(dom.el("planPartyDestinations").innerHTML.includes("Destination for Acme"));

  dom.el("planDestinationAddress").value =
    "0x9999999999999999999999999999999999999999";
  dom.el("planPartyDest_party_acme").value =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  await inventory.generateConsolidationPlan();

  const generate = calls.find(
    (call) => call.path === "/api/plans/consolidation/generate",
  );
  deepEqual(generate, {
    method: "POST",
    path: "/api/plans/consolidation/generate",
    body: {
      destination_address: "0x9999999999999999999999999999999999999999",
      include_watch_only: true,
      auto_queue_low_risk: false,
      routing_strategy: "per_party",
      party_destinations: [
        {
          counterparty_id: "party_acme",
          destination_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        },
      ],
    },
  });
  ok(toasts.includes("Dry-run consolidation plan generated"));
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
    "inventoryProbeTokenRegistry",
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
  equal(
    requestBody.run_async,
    undefined,
    "run_async stays absent unless the background checkbox is checked",
  );
});

test("inventory scan can run in background and surfaces the operation id", async () => {
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
    "inventoryProbeTokenRegistry",
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
    "inventoryRunAsync",
  ]);
  dom.el("inventoryRunAsync").checked = true;
  dom.el("inventoryProviderProfile").value = "mainnet";
  let requestBody: any = null;
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method: string, path: string, body?: unknown) => {
      if (path === "/api/inventory/scan/evm") {
        requestBody = body;
        return {
          job: { id: "job-1", status: "running" },
          addresses: [],
          holdings: [],
          operation: { id: "op-1", kind: "inventory_scan_evm", state: "running" },
        };
      }
      return {};
    },
    toast: (message: string) => {
      toasts.push(message);
    },
    downloadJson: () => undefined,
  });

  await inventory.scanInventoryEvm();

  equal(requestBody.run_async, true);
  ok(
    toasts.some((message) => message.includes("operation op-1")),
    "toast surfaces the operation id: " + toasts.join(" | "),
  );
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
  const deleteWatchPending = inventory.deleteWatchAddressBookEntry(
    "0x7777777777777777777777777777777777777777",
  );
  await answerConfirm("action");
  await deleteWatchPending;
  const deleteCall = calls.find(
    (call) => call.path === "/api/inventory/watch-addresses/delete",
  );
  equal(deleteCall?.body.address, "0x7777777777777777777777777777777777777777");
  ok(toasts.some((message) => message.includes("deleted")));
});

test("token registry UI renders imported lists and posts local import body", async () => {
  const dom = installDom([
    "tokenRegistryList",
    "tokenRegistryName",
    "tokenRegistryEntriesJson",
    "tokenRegistryFilePath",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: string[] = [];
  const inventory = createInventoryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/inventory/token-registry/import") {
        return { status: "ok", list: { name: "core-list" } };
      }
      if (path === "/api/inventory/token-registry") return { lists: [] };
      return {
        profiles: [],
        entries: [],
        findings: [],
        plans: [],
        jobs: [],
        addresses: [],
        holdings: [],
      };
    },
    toast: (message) => toasts.push(message),
    downloadJson: () => undefined,
  });

  inventory.renderTokenRegistry([
    {
      id: "tr1",
      name: "core-list",
      compartment_id: 0,
      source: "pasted-json",
      entries: [
        {
          chain_id: 1,
          address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          symbol: "AAA",
          decimals: 18,
        },
      ],
      created_at_unix: 1,
      updated_at_unix: 2,
    },
  ]);
  ok(dom.el("tokenRegistryList").innerHTML.includes("core-list"));
  ok(dom.el("tokenRegistryList").innerHTML.includes('data-action="deleteTokenRegistryList"'));

  const entriesJson =
    '[{"chain_id":1,"address":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","symbol":"AAA","decimals":18}]';
  dom.el("tokenRegistryName").value = "core-list";
  dom.el("tokenRegistryEntriesJson").value = entriesJson;
  await inventory.importTokenRegistry();

  deepEqual(
    calls.find((call) => call.path === "/api/inventory/token-registry/import"),
    {
      method: "POST",
      path: "/api/inventory/token-registry/import",
      body: {
        name: "core-list",
        entries_json: entriesJson,
        file_path: undefined,
      },
    },
  );
  ok(toasts.includes("Token registry list imported"));
  equal(dom.el("tokenRegistryName").value, "");
  equal(dom.el("tokenRegistryEntriesJson").value, "");
  equal(dom.el("tokenRegistryFilePath").value, "");
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
    "treasuryPartyList",
    "treasuryReceiveParty",
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
    "GET /api/treasury/parties",
    "GET /api/treasury/receive-addresses",
  ]);
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Tracked Addresses"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes(">6<"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Watch-Only"));
  ok(dom.el("treasuryOverviewStats").innerHTML.includes("Receive Active"));
  ok(dom.el("treasuryGeneratedAt").textContent.startsWith("Updated "));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("No treasury policy configured yet."));
  ok(dom.el("treasuryReceiveList").innerHTML.includes("No receive allocations yet."));
  ok(dom.el("treasuryPartyList").innerHTML.includes("No counterparties yet."));
  ok(dom.el("treasuryReceiveParty").innerHTML.includes("No party (optional)"));

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

test("receiving overview renders party groups, hd and stealth items, and balance states", async () => {
  const dom = installDom([
    "receivingOverviewStats",
    "receivingCoverage",
    "receivingQuickActions",
    "receivingGroupList",
  ]);
  const overview: ReceivingOverviewResponse = {
    generated_at_unix: 1717900000,
    include_retired: false,
    groups: [
      {
        counterparty: {
          id: "party-acme",
          name: "Acme",
          note: "Vendor",
          created_at_unix: 1717800000,
        },
        item_count: 1,
        native_total_wei_hex: "0xde0b6b3a7640000",
        items: [
          {
            source_type: "hd",
            address: "0x1111111111111111111111111111111111111111",
            chain_id: 1,
            derivation_path: "m/44'/60'/0'/0/5",
            purpose: "invoices",
            label: "June",
            counterparty_id: "party-acme",
            balance_native_wei_hex: "0xde0b6b3a7640000",
            balance_known: true,
            status: "active",
            created_at_unix: 1717900000,
          },
        ],
      },
      {
        counterparty: null,
        item_count: 2,
        native_total_wei_hex: "0xa688906bd8b0000",
        items: [
          {
            source_type: "hd",
            address: "0x2222222222222222222222222222222222222222",
            chain_id: 1,
            derivation_path: "m/44'/60'/0'/0/6",
            purpose: "intake",
            label: null,
            counterparty_id: null,
            balance_native_wei_hex: null,
            balance_known: false,
            status: "active",
            created_at_unix: 1717900100,
          },
          {
            source_type: "stealth",
            address: "0x3333333333333333333333333333333333333333",
            chain_id: 1,
            purpose: "private deposit",
            label: "scan result",
            counterparty_id: null,
            balance_native_wei_hex: "0xa688906bd8b0000",
            balance_known: true,
            status: "observed",
            created_at_unix: 1717900200,
          },
        ],
      },
    ],
    totals: {
      item_count: 3,
      hd_count: 2,
      stealth_count: 1,
      native_total_wei_hex: "0x18493fba64ef0000",
    },
    coverage: {
      addresses_total: 3,
      addresses_with_known_balance: 2,
      note: "persisted balances only",
    },
  };
  const calls: string[] = [];
  const receiving = createReceivingActions({
    api: async (method, path) => {
      calls.push(method + " " + path);
      if (path === "/api/treasury/parties") return { parties: [] };
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      return overview;
    },
    toast: () => undefined,
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });

  await receiving.loadReceivingOverview();

  deepEqual(calls, [
    "GET /api/receiving/overview",
    "GET /api/treasury/parties",
    "GET /api/deposits/eth-stealth",
  ]);
  ok(dom.el("receivingOverviewStats").innerHTML.includes("HD Addresses"));
  ok(dom.el("receivingOverviewStats").innerHTML.includes("Stealth Deposits"));
  ok(dom.el("receivingGroupList").innerHTML.includes("Acme"));
  ok(dom.el("receivingGroupList").innerHTML.includes("Unassigned"));
  ok(
    dom
      .el("receivingGroupList")
      .innerHTML.includes("balance unknown — run Refresh balances"),
  );
  ok(dom.el("receivingGroupList").innerHTML.includes("copyText"));
});

test("receiving stealth tag posts deposit id then reloads overview", async () => {
  const dom = installDom([
    "receivingOverviewStats",
    "receivingCoverage",
    "receivingQuickActions",
    "receivingGroupList",
  ]);
  const stealthAddress = "0x3333333333333333333333333333333333333333";
  const overview: ReceivingOverviewResponse = {
    generated_at_unix: 1717900000,
    include_retired: false,
    groups: [
      {
        counterparty: null,
        item_count: 1,
        native_total_wei_hex: "0x5",
        items: [
          {
            source_type: "stealth",
            address: stealthAddress,
            chain_id: 1,
            purpose: null,
            label: "scan result",
            counterparty_id: null,
            balance_native_wei_hex: "0x5",
            balance_known: true,
            status: "observed",
            created_at_unix: 1717900200,
          },
        ],
      },
    ],
    totals: {
      item_count: 1,
      hd_count: 0,
      stealth_count: 1,
      native_total_wei_hex: "0x5",
    },
    coverage: {
      addresses_total: 1,
      addresses_with_known_balance: 1,
      note: "persisted balances only",
    },
  };
  const calls: Array<{ method: string; path: string; body?: unknown }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const receiving = createReceivingActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (method === "GET" && path === "/api/receiving/overview") return overview;
      if (method === "GET" && path === "/api/treasury/parties") {
        return {
          parties: [{ id: "party-acme", name: "Acme", created_at_unix: 1 }],
        };
      }
      if (method === "GET" && path === "/api/deposits/eth-stealth") {
        return { deposits: [{ id: "dep-1", stealth_address: stealthAddress }] };
      }
      if (method === "POST" && path === "/api/receiving/deposits/tag") {
        return { status: "tagged" };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });

  await receiving.loadReceivingOverview();
  const selectEl = dom.document.createElement("select");
  selectEl.value = "party-acme";
  await receiving.tagStealthDeposit(stealthAddress, selectEl);

  const postIndex = calls.findIndex(
    (call) => call.method === "POST" && call.path === "/api/receiving/deposits/tag",
  );
  ok(postIndex > 0);
  deepEqual(calls[postIndex], {
    method: "POST",
    path: "/api/receiving/deposits/tag",
    body: { deposit_id: "dep-1", counterparty_id: "party-acme" },
  });
  deepEqual(calls[postIndex + 1], {
    method: "GET",
    path: "/api/receiving/overview",
    body: undefined,
  });
  deepEqual(toasts.pop(), { message: "Counterparty updated", type: undefined });
});

test("receiving balance refresh posts, reloads overview, and handles no-provider", async () => {
  installDom([
    "receivingOverviewStats",
    "receivingCoverage",
    "receivingQuickActions",
    "receivingGroupList",
  ]);
  const overview: ReceivingOverviewResponse = {
    generated_at_unix: 1717900000,
    include_retired: false,
    groups: [],
    totals: {
      item_count: 0,
      hd_count: 0,
      stealth_count: 0,
      native_total_wei_hex: "0x0",
    },
    coverage: {
      addresses_total: 0,
      addresses_with_known_balance: 0,
      note: "persisted balances only",
    },
  };
  const okRefresh: ReceivingRefreshResponse = {
    generated_at_unix: 1717900100,
    addresses_requested: 2,
    addresses_refreshed: 2,
    addresses_skipped: 0,
    stealth_refreshed: true,
    provider_status: "ok",
    errors: [],
  };
  const calls: string[] = [];
  const receiving = createReceivingActions({
    api: async (method, path) => {
      calls.push(method + " " + path);
      if (path === "/api/receiving/refresh-balances") return okRefresh;
      if (path === "/api/treasury/parties") return { parties: [] };
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      return overview;
    },
    toast: () => undefined,
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });

  await receiving.refreshReceivingBalances();

  deepEqual(calls, [
    "POST /api/receiving/refresh-balances",
    "GET /api/receiving/overview",
    "GET /api/treasury/parties",
    "GET /api/deposits/eth-stealth",
  ]);

  const noProviderCalls: string[] = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const noProviderRefresh: ReceivingRefreshResponse = {
    ...okRefresh,
    addresses_refreshed: 0,
    stealth_refreshed: false,
    provider_status: "no_provider",
  };
  const noProvider = createReceivingActions({
    api: async (method, path) => {
      noProviderCalls.push(method + " " + path);
      if (path === "/api/receiving/refresh-balances") return noProviderRefresh;
      if (path === "/api/treasury/parties") return { parties: [] };
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      return overview;
    },
    toast: (message, type) => toasts.push({ message, type }),
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });

  await noProvider.refreshReceivingBalances();

  deepEqual(noProviderCalls, [
    "POST /api/receiving/refresh-balances",
    "GET /api/receiving/overview",
    "GET /api/treasury/parties",
    "GET /api/deposits/eth-stealth",
  ]);
  deepEqual(toasts[0], {
    message: "Configure an RPC provider before refreshing receiving balances.",
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
    allow_claim_execution: true,
    allow_gas_topups: true,
    max_gas_topup_wei_hex: "0x" + (250000000000000000n).toString(16),
    allow_plan_execution: true,
    allow_sweep_execution: true,
    allow_revoke_execution: false,
    allow_exit_execution: false,
    max_fee_per_gas_cap_hex: "0x" + (50000000000n).toString(16),
    execution_paused: true,
    created_at_unix: 1717900000,
    updated_at_unix: 1717900500,
  };

  treasury.renderTreasuryPolicy(policy);
  const html = dom.el("treasuryPolicyList").innerHTML;
  ok(html.includes("Treasury policy"));
  ok(html.includes(">enabled<"));
  ok(html.includes("0x2222222222222222222222222222222222222222 (cold-vault)"));
  ok(html.includes("0x3333333333333333333333333333333333333333"));
  // The camelCase state line is now a plain-English summary…
  ok(html.includes('class="policy-summary"'));
  ok(html.includes("Plans may execute sweeps and claims."));
  ok(html.includes("DeFi exits and Revokes are blocked."));
  ok(html.includes("Cross-party linkage blocking is off"));
  ok(html.includes("Sponsor gas top-ups are allowed"));
  ok(html.includes("queue execution is currently paused"));
  // …and the raw state stays one click away behind "Technical state".
  ok(html.includes("<summary>Technical state</summary>"));
  ok(html.includes("maxStep=1.5 ETH"));
  ok(html.includes("maxPlan=-"));
  ok(html.includes("requireSimulation=true"));
  ok(html.includes("allowClaimExecution=true"));
  ok(html.includes("allowGasTopups=true"));
  ok(html.includes("maxGasTopup=0.25 ETH"));
  ok(html.includes("allowPlanExecution=true"));
  ok(html.includes("allowSweepExecution=true"));
  ok(html.includes("allowRevokeExecution=false"));
  ok(html.includes("allowExitExecution=false"));
  ok(html.includes("maxFeePerGasCap=50 Gwei"));
  ok(html.includes("paused=true"));

  treasury.renderTreasuryPolicy(null);
  ok(
    dom.el("treasuryPolicyList").innerHTML.includes("No treasury policy configured yet."),
  );
});

test("treasury policy summary describes the gates in plain English", () => {
  const base: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [],
    max_step_native_wei_hex: null,
    max_plan_native_wei_hex: null,
    require_simulation: true,
    allow_claim_execution: false,
    allow_gas_topups: false,
    max_gas_topup_wei_hex: null,
    allow_plan_execution: true,
    allow_sweep_execution: true,
    allow_revoke_execution: true,
    allow_exit_execution: false,
    max_fee_per_gas_cap_hex: null,
    execution_paused: false,
    created_at_unix: 1,
    updated_at_unix: 2,
  };

  deepEqual(treasuryPolicySummary({ ...base, enabled: false }), [
    "The policy is disabled, so nothing may execute from a plan.",
    "Cross-party linkage blocking is off — plans may route different payers to a shared destination.",
    "Sponsor gas top-ups are off.",
  ]);

  deepEqual(treasuryPolicySummary({ ...base, allow_plan_execution: false }), [
    "Plan execution is switched off, so no step may execute yet.",
    "Cross-party linkage blocking is off — plans may route different payers to a shared destination.",
    "Sponsor gas top-ups are off.",
  ]);

  deepEqual(
    treasuryPolicySummary({
      ...base,
      allow_exit_execution: true,
      allow_claim_execution: true,
      allow_gas_topups: true,
      block_cross_party_linkage: true,
    }),
    [
      "Plans may execute sweeps, revokes, DeFi exits, and claims.",
      "Cross-party linkage blocking is on; destinations are limited to the allow-list below.",
      "Sponsor gas top-ups are allowed.",
    ],
  );

  deepEqual(treasuryPolicySummary(base), [
    "Plans may execute sweeps and revokes.",
    "Claims and DeFi exits are blocked.",
    "Cross-party linkage blocking is off — plans may route different payers to a shared destination.",
    "Sponsor gas top-ups are off.",
  ]);
});

test("treasury policy form labels every numeric input and folds the legal hints", () => {
  const html = readFileSync("src/index.after-style-before-script.html", "utf8");
  const expectedLabels: Array<[string, string]> = [
    ["treasuryPolicyDestinations", "Allowed destinations"],
    ["treasuryPolicyMaxStepEth", "Per-step cap (ETH)"],
    ["treasuryPolicyMaxPlanEth", "Per-plan cap (ETH)"],
    ["treasuryPolicyFreshnessSecs", "Simulation freshness (seconds)"],
    ["treasuryPolicyHotFloorEth", "Hot floor (ETH)"],
    ["treasuryPolicyHotTargetEth", "Hot target (ETH)"],
    ["treasuryPolicyHotOverflowEth", "Hot overflow threshold (ETH)"],
    ["treasuryPolicyMaxGasTopupEth", "Max gas top-up (ETH)"],
    ["treasuryPolicyMaxFeePerGasGwei", "Max fee per gas (gwei)"],
  ];
  for (const [id, label] of expectedLabels) {
    ok(
      html.includes('for="' + id + '"'),
      "expected a visible label for #" + id,
    );
    ok(html.includes(label), "expected label text " + label);
  }
  ok(html.includes("<summary>How this policy protects you</summary>"));
  // The dense legal copy is preserved, just behind the details fold.
  ok(html.includes("Claim execution stays blocked unless ALL of these hold"));
  ok(html.includes("Nothing executes from a consolidation plan step unless"));
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
      chain_id: 8453,
      chain_id_assumed: false,
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
      chain_id: 1,
      chain_id_assumed: true,
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
  ok(html.includes("chain=8453"));
  ok(html.includes("chain=1 (assumed mainnet)"));
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
    "treasuryPolicyHotFloorEth",
    "treasuryPolicyHotTargetEth",
    "treasuryPolicyHotOverflowEth",
    "treasuryPolicyRequireSim",
    "treasuryPolicyAllowClaimExec",
    "treasuryPolicyAllowGasTopups",
    "treasuryPolicyAllowTreasuryAutomation",
    "treasuryPolicyMaxGasTopupEth",
    "treasuryPolicyAllowPlanExec",
    "treasuryPolicyAllowSweepExec",
    "treasuryPolicyAllowRevokeExec",
    "treasuryPolicyAllowExitExec",
    "treasuryPolicyMaxFeePerGasGwei",
  ]);
  const policy: TreasuryPolicy = {
    enabled: true,
    allowed_destinations: [
      { address: "0x2222222222222222222222222222222222222222", label: "cold" },
    ],
    max_step_native_wei_hex: "0x" + (2000000000000000000n).toString(16),
    max_plan_native_wei_hex: "0xde0b6b3a7640000",
    hot_overflow_wei_hex: "0x" + (3000000000000000000n).toString(16),
    require_simulation: true,
    allow_claim_execution: true,
    allow_gas_topups: true,
    allow_treasury_automation: true,
    max_gas_topup_wei_hex: "0x" + (500000000000000000n).toString(16),
    allow_plan_execution: true,
    allow_sweep_execution: true,
    allow_revoke_execution: true,
    allow_exit_execution: true,
    max_fee_per_gas_cap_hex: "0x" + (30000000000n).toString(16),
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
  equal(dom.el("treasuryPolicyAllowClaimExec").checked, true);
  equal(dom.el("treasuryPolicyAllowGasTopups").checked, true);
  equal(dom.el("treasuryPolicyAllowTreasuryAutomation").checked, true);
  equal(dom.el("treasuryPolicyAllowPlanExec").checked, true);
  equal(dom.el("treasuryPolicyAllowSweepExec").checked, true);
  equal(dom.el("treasuryPolicyAllowRevokeExec").checked, true);
  equal(dom.el("treasuryPolicyAllowExitExec").checked, true);
  equal(dom.el("treasuryPolicyMaxFeePerGasGwei").value, "30");
  equal(
    dom.el("treasuryPolicyDestinations").value,
    "0x2222222222222222222222222222222222222222:cold",
  );
  equal(dom.el("treasuryPolicyMaxStepEth").value, "2");
  equal(dom.el("treasuryPolicyMaxPlanEth").value, "1");
  equal(dom.el("treasuryPolicyMaxGasTopupEth").value, "0.5");
  equal(dom.el("treasuryPolicyHotFloorEth").value, "1");
  equal(dom.el("treasuryPolicyHotTargetEth").value, "1");
  equal(dom.el("treasuryPolicyHotOverflowEth").value, "3");
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxPlan=1 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("hotFloor=1 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("hotTarget=1 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("hotOverflow=3 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("allowTreasuryAutomation=true"));

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
    "treasuryPolicyAllowClaimExec",
    "treasuryPolicyAllowGasTopups",
    "treasuryPolicyAllowTreasuryAutomation",
    "treasuryPolicyMaxGasTopupEth",
    "treasuryPolicyAllowPlanExec",
    "treasuryPolicyAllowSweepExec",
    "treasuryPolicyAllowRevokeExec",
    "treasuryPolicyAllowExitExec",
    "treasuryPolicyMaxFeePerGasGwei",
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
    hot_overflow_wei_hex: "0x" + (3000000000000000000n).toString(16),
    require_simulation: false,
    allow_claim_execution: true,
    allow_gas_topups: true,
    allow_treasury_automation: true,
    max_gas_topup_wei_hex: "0x" + (250000000000000000n).toString(16),
    allow_plan_execution: true,
    allow_sweep_execution: true,
    allow_revoke_execution: false,
    allow_exit_execution: false,
    max_fee_per_gas_cap_hex: "0x" + (25000000000n).toString(16),
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
  dom.el("treasuryPolicyAllowClaimExec").checked = true;
  dom.el("treasuryPolicyAllowGasTopups").checked = true;
  dom.el("treasuryPolicyAllowTreasuryAutomation").checked = true;
  dom.el("treasuryPolicyAllowPlanExec").checked = true;
  dom.el("treasuryPolicyAllowSweepExec").checked = true;
  dom.el("treasuryPolicyAllowRevokeExec").checked = false;
  dom.el("treasuryPolicyAllowExitExec").checked = false;
  dom.el("treasuryPolicyDestinations").value =
    "0x2222222222222222222222222222222222222222:cold\n0x3333333333333333333333333333333333333333";
  dom.el("treasuryPolicyMaxStepEth").value = "1.5";
  dom.el("treasuryPolicyMaxPlanEth").value = "not-a-number";
  dom.el("treasuryPolicyHotOverflowEth").value = "3";
  dom.el("treasuryPolicyMaxGasTopupEth").value = "0.25";
  dom.el("treasuryPolicyMaxFeePerGasGwei").value = "25";

  await treasury.updateTreasuryPolicy();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Max per-plan cap must be a decimal ETH amount with up to 18 decimals",
    type: "error",
  });

  dom.el("treasuryPolicyMaxPlanEth").value = "";
  dom.el("treasuryPolicyMaxGasTopupEth").value = "not-a-number";
  await treasury.updateTreasuryPolicy();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Max gas top-up must be a decimal ETH amount with up to 18 decimals",
    type: "error",
  });

  dom.el("treasuryPolicyMaxGasTopupEth").value = "0.25";
  dom.el("treasuryPolicyMaxFeePerGasGwei").value = "not-a-number";
  await treasury.updateTreasuryPolicy();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Max fee per gas must be a decimal gwei amount with up to 9 decimals",
    type: "error",
  });

  dom.el("treasuryPolicyMaxFeePerGasGwei").value = "25";
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
      hot_overflow_wei_hex: "0x" + (3000000000000000000n).toString(16),
      require_simulation: false,
      block_cross_party_linkage: false,
      allow_claim_execution: true,
      allow_gas_topups: true,
      allow_treasury_automation: true,
      max_gas_topup_wei_hex: "0x" + (250000000000000000n).toString(16),
      allow_plan_execution: true,
      allow_sweep_execution: true,
      allow_revoke_execution: false,
      allow_exit_execution: false,
      max_fee_per_gas_cap_hex: "0x" + (25000000000n).toString(16),
    },
  });
  deepEqual(toasts.pop(), { message: "Treasury policy saved", type: undefined });
  ok(dom.el("treasuryPolicyList").innerHTML.includes(">enabled<"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxStep=1.5 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxGasTopup=0.25 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("hotOverflow=3 ETH"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("allowTreasuryAutomation=true"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("allowPlanExecution=true"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("allowSweepExecution=true"));
  ok(dom.el("treasuryPolicyList").innerHTML.includes("maxFeePerGasCap=25 Gwei"));
  equal(dom.el("treasuryPolicyMaxStepEth").value, "1.5");
  equal(dom.el("treasuryPolicyMaxGasTopupEth").value, "0.25");
  equal(dom.el("treasuryPolicyMaxFeePerGasGwei").value, "25");
  ok(calls.some((call) => call.path === "/api/treasury/overview"));
});

test("treasury policy save persists cross-party linkage block toggle", async () => {
  const dom = installDom([
    "treasuryPolicyEnabled",
    "treasuryPolicyRequireSim",
    "treasuryPolicyBlockLinkage",
    "treasuryPolicyAllowClaimExec",
    "treasuryPolicyDestinations",
    "treasuryPolicyMaxStepEth",
    "treasuryPolicyMaxPlanEth",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy/update") {
        return { status: "updated", policy: null };
      }
      return {};
    },
    toast: () => undefined,
  });

  dom.el("treasuryPolicyBlockLinkage").checked = true;
  await treasury.updateTreasuryPolicy();

  const update = calls.find((call) => call.path === "/api/treasury/policy/update");
  equal(update?.body.block_cross_party_linkage, true);
});

test("treasury policy save posts hot refill caps only when provided", async () => {
  const dom = installDom([
    "treasuryPolicyList",
    "treasuryPolicyEnabled",
    "treasuryPolicyRequireSim",
    "treasuryPolicyBlockLinkage",
    "treasuryPolicyAllowClaimExec",
    "treasuryPolicyDestinations",
    "treasuryPolicyMaxStepEth",
    "treasuryPolicyMaxPlanEth",
    "treasuryPolicyFreshnessSecs",
    "treasuryPolicyHotFloorEth",
    "treasuryPolicyHotTargetEth",
    "treasuryPolicyHotOverflowEth",
    "treasuryPolicyAllowTreasuryAutomation",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy/update") {
        return { status: "updated", policy: null };
      }
      return {};
    },
    toast: () => undefined,
  });

  dom.el("treasuryPolicyHotFloorEth").value = "1";
  dom.el("treasuryPolicyHotTargetEth").value = "1";
  dom.el("treasuryPolicyHotOverflowEth").value = "2";
  dom.el("treasuryPolicyAllowTreasuryAutomation").checked = true;
  await treasury.updateTreasuryPolicy();

  const withHotCaps = calls.find(
    (call) => call.path === "/api/treasury/policy/update",
  );
  deepEqual(
    {
      hot_floor_wei_hex: withHotCaps?.body.hot_floor_wei_hex,
      hot_target_wei_hex: withHotCaps?.body.hot_target_wei_hex,
      hot_overflow_wei_hex: withHotCaps?.body.hot_overflow_wei_hex,
      allow_treasury_automation: withHotCaps?.body.allow_treasury_automation,
    },
    {
      hot_floor_wei_hex: "0xde0b6b3a7640000",
      hot_target_wei_hex: "0xde0b6b3a7640000",
      hot_overflow_wei_hex: "0x1bc16d674ec80000",
      allow_treasury_automation: true,
    },
  );

  calls.length = 0;
  dom.el("treasuryPolicyHotFloorEth").value = "";
  dom.el("treasuryPolicyHotTargetEth").value = "";
  dom.el("treasuryPolicyHotOverflowEth").value = "";
  dom.el("treasuryPolicyAllowTreasuryAutomation").checked = false;
  await treasury.updateTreasuryPolicy();

  const withoutHotCaps = calls.find(
    (call) => call.path === "/api/treasury/policy/update",
  );
  ok(!("hot_floor_wei_hex" in withoutHotCaps!.body));
  ok(!("hot_target_wei_hex" in withoutHotCaps!.body));
  ok(!("hot_overflow_wei_hex" in withoutHotCaps!.body));
  equal(withoutHotCaps!.body.allow_treasury_automation, false);
});

test("treasury policy save posts simulation freshness only when provided", async () => {
  const dom = installDom([
    "treasuryPolicyEnabled",
    "treasuryPolicyRequireSim",
    "treasuryPolicyBlockLinkage",
    "treasuryPolicyAllowClaimExec",
    "treasuryPolicyDestinations",
    "treasuryPolicyMaxStepEth",
    "treasuryPolicyMaxPlanEth",
    "treasuryPolicyFreshnessSecs",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/policy/update") {
        return { status: "updated", policy: null };
      }
      return {};
    },
    toast: () => undefined,
  });

  dom.el("treasuryPolicyFreshnessSecs").value = "120";
  await treasury.updateTreasuryPolicy();
  const withFreshness = calls.find(
    (call) => call.path === "/api/treasury/policy/update",
  );
  equal(withFreshness?.body.simulation_freshness_secs, 120);

  calls.length = 0;
  dom.el("treasuryPolicyFreshnessSecs").value = "";
  await treasury.updateTreasuryPolicy();
  const withoutFreshness = calls.find(
    (call) => call.path === "/api/treasury/policy/update",
  );
  ok(!("simulation_freshness_secs" in withoutFreshness!.body));
});

test("treasury receive allocate and rotate dispatch api calls with toasts", async () => {
  const dom = installDom([
    "treasuryReceiveList",
    "treasuryReceiveProfile",
    "treasuryReceivePurpose",
    "treasuryReceiveLabel",
    "treasuryReceiveParty",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const allocation: TreasuryReceiveAllocation = {
    id: "alloc-7",
    wallet_family: "eth-seed",
    wallet_profile: "archive",
    chain_id: 8453,
    chain_id_assumed: false,
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
      if (path === "/api/treasury/parties") {
        return {
          parties: [
            {
              id: "party-1",
              name: "Client One",
              created_at_unix: 1717900000,
            },
          ],
        };
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
  dom.el("treasuryReceiveParty").innerHTML =
    '<option value="">No party (optional)</option><option value="party-1">Client One</option>';
  dom.el("treasuryReceiveParty").value = "party-1";

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
    body: {
      wallet_profile: "archive",
      purpose: "donations",
      label: "fundraiser",
      counterparty_id: "party-1",
    },
  });
  equal(dom.el("treasuryReceiveProfile").value, "");
  equal(dom.el("treasuryReceivePurpose").value, "");
  equal(dom.el("treasuryReceiveLabel").value, "");
  equal(dom.el("treasuryReceiveParty").value, "");
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
  const rotateDispatch = () =>
    dispatchDataAction(rotateButton as any, {
      actions: {
        rotateTreasuryReceiveAddress: (...args: unknown[]) =>
          (pending = treasury.rotateTreasuryReceiveAddress(args[0] as string)),
      },
      toast: () => undefined,
    });

  // Cancelling the rotation dialog never reaches the daemon.
  rotateDispatch();
  await answerConfirm("cancel");
  await pending;
  equal(calls.length, 0);

  rotateDispatch();
  await answerConfirm("action");
  await pending;

  deepEqual(calls[0], {
    method: "POST",
    path: "/api/treasury/receive-addresses/rotate",
    body: { allocation_id: "alloc-7" },
  });
  deepEqual(toasts.pop(), { message: "Receive address rotated", type: undefined });
  ok(calls.some((call) => call.path === "/api/treasury/receive-addresses"));
});

// ── Wallet manager ──────────────────────────────────────────────────────────

const MNEMONIC_WORDS = [
  "abandon",
  "ability",
  "able",
  "about",
  "above",
  "absent",
  "absorb",
  "abstract",
  "absurd",
  "abuse",
  "access",
  "accident",
];

function walletManagerGroup(
  profile: string,
  chainId: number,
  weiHex: string,
): TreasuryGroupSummary {
  return {
    wallet_family: "eth-seed",
    wallet_profile: profile,
    chain_id: chainId,
    address_count: 1,
    funded_address_count: 1,
    native_total_wei_hex: weiHex,
    signer_address_count: 1,
    watch_only_address_count: 0,
    erc20_holding_count: 0,
    nft_holding_count: 0,
    defi_holding_count: 0,
    claimable_holding_count: 0,
    approval_exposure_count: 0,
    dormant_candidate_count: 0,
  };
}

function walletManagerSeedProfile(): EthSeedWalletProfile {
  return {
    name: "main",
    label: "Main treasury",
    project_account: 0,
    provider_profile: "mainnet",
    compartment_id: 0,
    chain_id: 1,
    execution_enabled: false,
    word_count: 24,
    mnemonic_secret_key: "wallet.eth-seed.main.mnemonic",
    account_path: "m/44'/60'/0'",
    receive_path: "m/44'/60'/0'/0",
    receive_xpub: "xpub-main-receive",
    first_receive_address: "0x1111111111111111111111111111111111111111",
  };
}

function installWalletManagerDom() {
  const dom = installDom([
    "walletManagerList",
    "walletManagerCard",
    "walletCreateForm",
    "walletCreateName",
    "walletCreateLabel",
    "walletCreateAccount",
    "walletCreateChainId",
    "walletCreateDestination",
    "walletCreatePassphrase",
    "walletCreateProviderHint",
    "walletQuickProvider",
    "walletQuickProviderName",
    "walletQuickProviderUrl",
    "walletQuickProviderChainId",
    "walletMnemonicReveal",
    "walletReceivePanel",
    "walletReceiveTarget",
    "walletReceivePurpose",
    "walletReceiveLabel",
    "walletImportSeedForm",
    "walletImportXpubForm",
    "walletImportWatchForm",
    "walletImportTabSeed",
    "walletImportTabXpub",
    "walletImportTabWatch",
    "walletImportSeedName",
    "walletImportSeedLabel",
    "walletImportSeedMnemonic",
    "walletImportSeedPassphrase",
    "walletImportSeedAccount",
    "walletImportSeedChainId",
    "walletImportSeedDestination",
    "walletImportXpubName",
    "walletImportXpubAccount",
    "walletImportXpubCompartmentId",
    "walletImportXpubChainId",
    "walletImportXpubDestination",
    "walletImportExternalReceiveXpub",
    "walletImportExternalReceivePath",
    "walletImportExternalAccountXpub",
    "walletImportExternalAccountPath",
    "walletImportWatchAddress",
    "walletImportWatchLabel",
  ]);
  dom.el("walletCreateSubmit", "BUTTON");
  dom.el("walletCreateProvider", "SELECT");
  dom.el("walletImportSeedProvider", "SELECT");
  dom.el("walletImportXpubProvider", "SELECT");
  dom.el("walletCreateWords12", "INPUT");
  dom.el("walletCreateWords24", "INPUT");
  dom.el("walletCreateWords24").checked = true;
  dom.el("walletMnemonicReveal").classList.add("hidden");
  dom.el("walletReceivePanel").classList.add("hidden");
  dom.el("walletImportXpubForm").classList.add("hidden");
  dom.el("walletImportWatchForm").classList.add("hidden");
  dom.el("walletImportTabSeed").classList.add("active");
  return dom;
}

test("wallet row meta helpers summarize identity, balances, and xpub display", () => {
  installDom();
  const groups = [
    walletManagerGroup("main", 1, "0x" + (1500000000000000000n).toString(16)),
    walletManagerGroup("main", 8453, "0x" + (200000000000000000n).toString(16)),
    walletManagerGroup("other", 1, "0xde0b6b3a7640000"),
  ];
  equal(
    walletNativeBalanceFromGroups("main", groups),
    "1.5 ETH on chain 1 · 0.2 on 8453",
  );
  equal(walletNativeBalanceFromGroups("missing", groups), "not scanned yet");
  equal(walletNativeBalanceFromGroups("main", null), "not scanned yet");
  equal(
    walletNativeBalanceFromGroups("other", [
      walletManagerGroup("other", 1, "0xde0b6b3a7640000"),
      walletManagerGroup("other", 1, "0xde0b6b3a7640000"),
    ]),
    "2 ETH on chain 1",
  );

  equal(
    walletRowMeta(walletManagerSeedProfile(), groups, 2),
    "0x1111111111111111111111111111111111111111\n" +
      "provider=mainnet · chain=1 · account=0 · words=24\n" +
      "balance=1.5 ETH on chain 1 · 0.2 on 8453\n" +
      "receive allocations=2",
  );

  const xpubProfile: EthXpubWalletProfile = {
    name: "cold-watch",
    project_account: 3,
    provider_profile: "base",
    compartment_id: 0,
    execution_enabled: false,
  };
  equal(xpubDisplay(xpubProfile), "receive path m/44'/60'/3'/0");
  equal(
    walletRowMeta(xpubProfile, [], null),
    "receive path m/44'/60'/3'/0\n" +
      "provider=base · chain=- · account=3\n" +
      "balance=not scanned yet",
  );
  const accountXpubProfile = {
    ...xpubProfile,
    external_account_xpub: "xpub-account",
    external_account_path: "m/44'/60'/8'",
  };
  equal(xpubDisplay(accountXpubProfile), "external account path m/44'/60'/8'/0");
  equal(
    walletRowMeta(accountXpubProfile, [], null),
    "external account path m/44'/60'/8'/0\n" +
      "provider=base · chain=- · account=3 · source=external custom account xpub\n" +
      "balance=not scanned yet",
  );
  const defaultAccountXpubProfile = { ...xpubProfile, external_account_xpub: "xpub-account" };
  equal(xpubDisplay(defaultAccountXpubProfile), "external receive path m/44'/60'/3'/0");
  equal(
    walletRowMeta(defaultAccountXpubProfile, [], null),
    "external receive path m/44'/60'/3'/0\n" +
      "provider=base · chain=- · account=3 · source=external account xpub\n" +
      "balance=not scanned yet",
  );
  const customXpubProfile = {
    ...xpubProfile,
    external_receive_xpub: "xpub-custom",
    external_receive_path: "m/44'/60'/3'/1",
  };
  equal(xpubDisplay(customXpubProfile), "external receive path m/44'/60'/3'/1");
  equal(
    walletRowMeta(customXpubProfile, [], null),
    "external receive path m/44'/60'/3'/1\n" +
      "provider=base · chain=- · account=3 · source=external custom xpub\n" +
      "balance=not scanned yet",
  );
});

test("wallet manager list renders unified wallets with balances and fallbacks", async () => {
  const dom = installWalletManagerDom();
  let empty = false;
  const manager = createWalletManagerActions({
    api: async (_method, path) => {
      if (empty) return {};
      if (path === "/api/profiles/eth-seed") {
        return { profiles: [walletManagerSeedProfile()] };
      }
      if (path === "/api/profiles/eth-xpub") {
        return {
          profiles: [
            {
              name: "cold-watch",
              project_account: 3,
              provider_profile: "mainnet",
              compartment_id: 0,
            },
          ],
        };
      }
      if (path === "/api/profiles/evm") {
        return { profiles: [{ name: "mainnet", chain_id: 1 }] };
      }
      if (path === "/api/treasury/overview") {
        return {
          groups: [
            walletManagerGroup("main", 1, "0x" + (1500000000000000000n).toString(16)),
            walletManagerGroup("main", 8453, "0x" + (200000000000000000n).toString(16)),
          ],
        };
      }
      if (path === "/api/treasury/receive-addresses") {
        return {
          allocations: [
            { wallet_profile: "main", status: "active" },
            { wallet_profile: "main", status: "retired" },
          ],
        };
      }
      return {};
    },
    toast: () => undefined,
  });

  await manager.loadWalletManager();
  const html = dom.el("walletManagerList").innerHTML;
  ok(html.includes("Main treasury"));
  ok(html.includes(">signer<"));
  ok(html.includes(">watch-only<"));
  ok(html.includes("0x1111111111111111111111111111111111111111"));
  ok(html.includes("balance=1.5 ETH on chain 1 · 0.2 on 8453"));
  ok(html.includes("receive allocations=1"));
  ok(html.includes("receive path m/44'/60'/3'/0"));
  ok(html.includes("balance=not scanned yet"));
  equal(html.split('data-action="copyWalletAddress"').length - 1, 1);
  equal(html.split('data-action="promptWalletReceiveAddress"').length - 1, 2);
  equal(html.split('data-action="deleteManagedWallet"').length - 1, 2);
  ok(dom.el("walletCreateProvider").innerHTML.includes("mainnet · chain 1"));
  equal(dom.el("walletCreateSubmit").disabled, false);
  equal(dom.el("walletCreateProviderHint").classList.contains("hidden"), true);
  equal(dom.el("walletQuickProvider").classList.contains("hidden"), true);

  empty = true;
  await manager.loadWalletManager();
  ok(
    dom
      .el("walletManagerList")
      .innerHTML.includes("No wallets yet. Create one below or import an existing wallet."),
  );
  // The empty state is actionable: it carries a focus action, not a dead end.
  ok(dom.el("walletManagerList").innerHTML.includes('data-action="focusWalletCreate"'));
  ok(dom.el("walletManagerList").innerHTML.includes("empty-state-action"));
  equal(dom.el("walletCreateSubmit").disabled, true);
  equal(dom.el("walletCreateProviderHint").classList.contains("hidden"), false);
  // With no providers, the inline quick-add is the visible path forward.
  equal(dom.el("walletQuickProvider").classList.contains("hidden"), false);
});

test("provider profile editor posts fee estimation opt-in", async () => {
  const dom = installDom([
    "providerProfileList",
    "providerName",
    "providerRpcUrl",
    "providerChainId",
    "providerAuthTokenKey",
    "providerCompartmentId",
    "providerMaxPriorityFee",
    "providerMaxFee",
    "providerNativeGasLimit",
    "providerErc20GasLimit",
    "providerFeeEstimation",
  ]);
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  let refreshes = 0;
  const wallets = createWalletActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return { status: "ok" };
    },
    toast: () => undefined,
    refresh: () => {
      refreshes += 1;
    },
    copyText: async () => undefined,
  });

  dom.el("providerName").value = "mainnet";
  dom.el("providerRpcUrl").value = "https://rpc.example.test";
  dom.el("providerChainId").value = "1";
  dom.el("providerFeeEstimation").checked = true;

  await wallets.upsertProviderProfile();

  const upsert = calls.find((call) => call.path === "/api/profiles/evm/upsert");
  equal(upsert?.body.fee_estimation_enabled, true);
  equal(dom.el("providerFeeEstimation").checked, false);
  equal(refreshes, 1);

  wallets.renderProviderProfiles([
    {
      name: "mainnet",
      rpc_url: "https://rpc.example.test",
      chain_id: 1,
      compartment_id: null,
      fee_estimation_enabled: true,
    },
  ]);
  ok(dom.el("providerProfileList").innerHTML.includes("feeEstimation=on"));
});

test("wallet manager quick-add provider validates, posts, and reloads", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/profiles/eth-seed") return { profiles: [] };
      if (path === "/api/profiles/eth-xpub") return { profiles: [] };
      if (path === "/api/profiles/evm") return { profiles: [] };
      if (path === "/api/treasury/overview") return { groups: [] };
      if (path === "/api/treasury/receive-addresses") return { allocations: [] };
      return { status: "ok" };
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  // Invalid URL: error toast, no POST.
  dom.el("walletQuickProviderName").value = "mainnet";
  dom.el("walletQuickProviderUrl").value = "not-a-url";
  dom.el("walletQuickProviderChainId").value = "1";
  await manager.quickAddWalletProvider();
  equal(calls.filter((call) => call.method === "POST").length, 0);
  equal(toasts.pop()?.type, "error");

  dom.el("walletQuickProviderUrl").value = "https://rpc.example.test";
  dom.el("walletQuickProviderChainId").value = "8453";
  await manager.quickAddWalletProvider();
  const upsert = calls.find((call) => call.path === "/api/profiles/evm/upsert");
  deepEqual(upsert?.body, {
    name: "mainnet",
    rpc_url: "https://rpc.example.test",
    chain_id: 8453,
  });
  // Successful add reloads the manager so the select repopulates.
  ok(calls.some((call) => call.path === "/api/profiles/evm" && call.method === "GET"));
  equal(dom.el("walletQuickProviderUrl").value, "");
  ok(toasts.some((entry) => entry.message.includes("Provider 'mainnet' saved.")));
});

test("wallet manager create flow reveals the one-time mnemonic and scrubs it on confirm", async () => {
  const dom = installWalletManagerDom();
  const mnemonic = MNEMONIC_WORDS.join(" ");
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/profiles/eth-seed/create") {
        return {
          status: "created",
          mnemonic,
          profile: { name: "treasury-main", word_count: 12 },
        };
      }
      if (path === "/api/profiles/evm") {
        return { profiles: [{ name: "mainnet", chain_id: 1 }] };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  dom.el("walletCreateName").value = "treasury-main";
  dom.el("walletCreateLabel").value = "Treasury Main";
  dom.el("walletCreateProvider").value = "mainnet";
  dom.el("walletCreateAccount").value = "0";
  dom.el("walletCreatePassphrase").value = "tail-passphrase";
  dom.el("walletCreateWords12").checked = true;
  dom.el("walletCreateWords24").checked = false;

  await manager.createWallet();

  deepEqual(calls, [
    {
      method: "POST",
      path: "/api/profiles/eth-seed/create",
      body: {
        name: "treasury-main",
        word_count: 12,
        project_account: 0,
        provider_profile: "mainnet",
        label: "Treasury Main",
        mnemonic_passphrase: "tail-passphrase",
      },
    },
  ]);
  const reveal = dom.el("walletMnemonicReveal");
  equal(manager.hasPendingMnemonic(), true);
  equal(reveal.classList.contains("hidden"), false);
  equal(reveal.innerHTML.split("mnemonic-word").length - 1, 12);
  MNEMONIC_WORDS.forEach((word) => ok(reveal.innerHTML.includes(">" + word + "<")));
  ok(reveal.innerHTML.includes("Written down? It will never be shown again."));
  equal(dom.el("walletCreateName").disabled, true);
  equal(dom.el("walletCreateSubmit").disabled, true);
  equal(dom.el("walletCreateForm").classList.contains("form-disabled"), true);
  equal(dom.el("walletCreatePassphrase").value, "");
  ok(toasts.every((entry) => !entry.message.includes("abandon")));

  let copied: string | null = null;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        writeText: async (value: string) => {
          copied = value;
        },
      },
    },
  });
  await manager.copyMnemonicPhrase();
  equal(copied, mnemonic);

  calls.length = 0;
  await manager.confirmMnemonicSaved();
  equal(manager.hasPendingMnemonic(), false);
  equal(reveal.innerHTML, "");
  equal(reveal.classList.contains("hidden"), true);
  MNEMONIC_WORDS.forEach((word) => ok(!reveal.innerHTML.includes(word)));
  equal(dom.el("walletCreateName").disabled, false);
  equal(dom.el("walletCreateName").value, "");
  equal(dom.el("walletCreateAccount").value, "0");
  equal(dom.el("walletCreateWords24").checked, true);
  equal(dom.el("walletCreateWords12").checked, false);
  equal(dom.el("walletCreateForm").classList.contains("form-disabled"), false);
  equal(dom.el("walletCreateSubmit").disabled, false);
  ok(calls.some((call) => call.path === "/api/profiles/eth-seed"));
  ok(toasts.every((entry) => !entry.message.includes("abandon")));

  copied = null;
  await manager.copyMnemonicPhrase();
  equal(copied, null);
  deepEqual(toasts.pop(), {
    message: "No seed phrase is being shown",
    type: "error",
  });
});

test("wallet manager create surfaces conflicts and bad names without revealing", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return {
        error: "Seed wallet profile already exists. Use upsert to replace it.",
      };
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  dom.el("walletCreateName").value = "bad name!";
  dom.el("walletCreateProvider").value = "mainnet";
  await manager.createWallet();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Wallet name may only contain letters, digits, '-' and '_'",
    type: "error",
  });

  dom.el("walletCreateName").value = "treasury-main";
  await manager.createWallet();
  equal(calls.length, 1);
  deepEqual(toasts.pop(), {
    message: "Seed wallet profile already exists. Use upsert to replace it.",
    type: "error",
  });
  equal(manager.hasPendingMnemonic(), false);
  equal(dom.el("walletMnemonicReveal").classList.contains("hidden"), true);
  equal(dom.el("walletMnemonicReveal").innerHTML, "");
  equal(dom.el("walletCreateName").disabled, false);
  equal(dom.el("walletCreateName").value, "treasury-main");
});

test("wallet manager seed import validates words and posts the upsert contract", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/profiles/eth-seed/upsert") return { status: "saved" };
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  dom.el("walletImportSeedName").value = "imported-main";
  dom.el("walletImportSeedProvider").value = "mainnet";
  dom.el("walletImportSeedMnemonic").value = MNEMONIC_WORDS.slice(0, 11).join(" ");
  await manager.importSeedWallet();
  equal(calls.length, 0);
  deepEqual(toasts.pop(), {
    message: "Seed phrase must contain exactly 12 or 24 words",
    type: "error",
  });

  dom.el("walletImportSeedMnemonic").value =
    "  " + MNEMONIC_WORDS.join("\n  ") + "  ";
  dom.el("walletImportSeedLabel").value = "Imported Main";
  dom.el("walletImportSeedPassphrase").value = "extra secret";
  dom.el("walletImportSeedAccount").value = "3";
  dom.el("walletImportSeedChainId").value = "8453";
  dom.el("walletImportSeedDestination").value =
    "0x9999999999999999999999999999999999999999";
  await manager.importSeedWallet();

  const upsert = calls.find((call) => call.path === "/api/profiles/eth-seed/upsert");
  deepEqual(upsert, {
    method: "POST",
    path: "/api/profiles/eth-seed/upsert",
    body: {
      name: "imported-main",
      mnemonic: MNEMONIC_WORDS.join(" "),
      project_account: 3,
      provider_profile: "mainnet",
      label: "Imported Main",
      mnemonic_passphrase: "extra secret",
      chain_id: 8453,
      default_destination_address: "0x9999999999999999999999999999999999999999",
    },
  });
  equal(dom.el("walletImportSeedMnemonic").value, "");
  equal(dom.el("walletImportSeedPassphrase").value, "");
  equal(dom.el("walletImportSeedName").value, "");
  equal(dom.el("walletImportSeedAccount").value, "0");
  ok(toasts.some((entry) => entry.message === 'Seed wallet "imported-main" imported'));
  ok(calls.some((call) => call.path === "/api/profiles/eth-seed"));
});

test("wallet manager import tabs switch forms, scrub seed input, and post contracts", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      return { status: "ok" };
    },
    toast: () => undefined,
  });

  dom.el("walletImportSeedMnemonic").value = "leak words here";
  dom.el("walletImportSeedPassphrase").value = "leak-pass";
  manager.setWalletImportTab("xpub");
  equal(dom.el("walletImportSeedForm").classList.contains("hidden"), true);
  equal(dom.el("walletImportXpubForm").classList.contains("hidden"), false);
  equal(dom.el("walletImportWatchForm").classList.contains("hidden"), true);
  equal(dom.el("walletImportTabXpub").classList.contains("active"), true);
  equal(dom.el("walletImportTabSeed").classList.contains("active"), false);
  equal(dom.el("walletImportSeedMnemonic").value, "");
  equal(dom.el("walletImportSeedPassphrase").value, "");

  dom.el("walletImportXpubName").value = "cold-watch";
  dom.el("walletImportXpubAccount").value = "2";
  dom.el("walletImportXpubProvider").value = "mainnet";
  dom.el("walletImportXpubCompartmentId").value = "1";
  dom.el("walletImportXpubChainId").value = "10";
  dom.el("walletImportXpubDestination").value =
    "0x8888888888888888888888888888888888888888";
  dom.el("walletImportExternalReceiveXpub").value = "xpub-receive";
  dom.el("walletImportExternalReceivePath").value = "m/44'/60'/9'/1";
  dom.el("walletImportExternalAccountPath").value = "";
  await manager.importXpubWallet();
  const xpubUpsert = calls.find(
    (call) => call.path === "/api/profiles/eth-xpub/upsert",
  );
  deepEqual(xpubUpsert, {
    method: "POST",
    path: "/api/profiles/eth-xpub/upsert",
    body: {
      name: "cold-watch",
      project_account: 2,
      provider_profile: "mainnet",
      compartment_id: 1,
      chain_id: 10,
      external_receive_xpub: "xpub-receive",
      external_receive_path: "m/44'/60'/9'/1",
      default_destination_address: "0x8888888888888888888888888888888888888888",
    },
  });
  equal(dom.el("walletImportXpubName").value, "");
  equal(dom.el("walletImportXpubAccount").value, "0");
  equal(dom.el("walletImportExternalReceiveXpub").value, "");
  equal(dom.el("walletImportExternalReceivePath").value, "");
  equal(dom.el("walletImportExternalAccountXpub").value, "");
  equal(dom.el("walletImportExternalAccountPath").value, "");

  manager.setWalletImportTab("watch");
  equal(dom.el("walletImportWatchForm").classList.contains("hidden"), false);
  equal(dom.el("walletImportXpubForm").classList.contains("hidden"), true);
  equal(dom.el("walletImportTabWatch").classList.contains("active"), true);

  dom.el("walletImportWatchAddress").value =
    "0x7777777777777777777777777777777777777777";
  dom.el("walletImportWatchLabel").value = "client-vault";
  await manager.importWatchAddress();
  const watchUpsert = calls.find(
    (call) => call.path === "/api/inventory/watch-addresses/upsert",
  );
  deepEqual(watchUpsert, {
    method: "POST",
    path: "/api/inventory/watch-addresses/upsert",
    body: {
      address: "0x7777777777777777777777777777777777777777",
      label: "client-vault",
      tags: [],
      enabled: true,
    },
  });
  equal(dom.el("walletImportWatchAddress").value, "");
});

test("wallet manager delete is gated by the shared confirmation dialog", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/profiles/eth-seed") {
        return { profiles: [walletManagerSeedProfile()] };
      }
      if (path === "/api/profiles/eth-xpub") {
        return {
          profiles: [
            {
              name: "watcher",
              project_account: 0,
              provider_profile: "mainnet",
              compartment_id: 0,
            },
          ],
        };
      }
      if (path === "/api/profiles/evm") {
        return { profiles: [{ name: "mainnet", chain_id: 1 }] };
      }
      return { status: "ok" };
    },
    toast: () => undefined,
  });

  await manager.loadWalletManager();
  const list = dom.el("walletManagerList");
  equal(list.innerHTML.split('data-action="deleteManagedWallet"').length - 1, 2);

  // Cancelling the dialog deletes nothing.
  let pendingDelete = manager.deleteManagedWallet("seed", "main");
  await answerConfirm("cancel");
  await pendingDelete;
  equal(calls.filter((call) => call.path.endsWith("/delete")).length, 0);

  // Confirming the danger action posts the delete for that exact profile.
  pendingDelete = manager.deleteManagedWallet("xpub", "watcher");
  await tick();
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes('"watcher"'),
    "dialog names the xpub profile being deleted",
  );
  await answerConfirm("action");
  await pendingDelete;
  deepEqual(
    calls.find((call) => call.path === "/api/profiles/eth-xpub/delete"),
    {
      method: "POST",
      path: "/api/profiles/eth-xpub/delete",
      body: { name: "watcher" },
    },
  );

  pendingDelete = manager.deleteManagedWallet("seed", "main");
  await tick();
  ok(
    confirmPart("[data-confirm-body]")?.textContent.includes("no longer sign"),
    "seed delete copy keeps the signing consequence",
  );
  await answerConfirm("action");
  await pendingDelete;
  deepEqual(
    calls.find((call) => call.path === "/api/profiles/eth-seed/delete"),
    {
      method: "POST",
      path: "/api/profiles/eth-seed/delete",
      body: { name: "main" },
    },
  );
});

test("wallet manager copy and receive-allocation flows hit clipboard and treasury", async () => {
  const dom = installWalletManagerDom();
  const calls: Array<{ method: string; path: string; body?: any }> = [];
  const toasts: Array<{ message: string; type?: string }> = [];
  let copied: string | null = null;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        writeText: async (value: string) => {
          copied = value;
        },
      },
    },
  });
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/receive-addresses/allocate") {
        return {
          status: "allocated",
          allocation: {
            address: "0x6666666666666666666666666666666666666666",
          },
        };
      }
      return {};
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  await manager.copyWalletAddress(
    "0x1111111111111111111111111111111111111111",
    "First receive address",
  );
  equal(copied, "0x1111111111111111111111111111111111111111");
  deepEqual(toasts.pop(), {
    message: "First receive address copied",
    type: undefined,
  });

  manager.promptWalletReceiveAddress("main");
  equal(dom.el("walletReceivePanel").classList.contains("hidden"), false);
  equal(dom.el("walletReceiveTarget").textContent, "main");

  await manager.allocateWalletReceiveAddress();
  equal(
    calls.filter((call) => call.path === "/api/treasury/receive-addresses/allocate")
      .length,
    0,
  );
  deepEqual(toasts.pop(), {
    message: "Purpose is required (e.g. invoices)",
    type: "error",
  });

  dom.el("walletReceivePurpose").value = "invoices";
  dom.el("walletReceiveLabel").value = "client-a";
  await manager.allocateWalletReceiveAddress();
  deepEqual(
    calls.find((call) => call.path === "/api/treasury/receive-addresses/allocate"),
    {
      method: "POST",
      path: "/api/treasury/receive-addresses/allocate",
      body: { wallet_profile: "main", purpose: "invoices", label: "client-a" },
    },
  );
  equal(dom.el("walletReceivePanel").classList.contains("hidden"), true);
  equal(dom.el("walletReceivePurpose").value, "");
  ok(
    toasts.some(
      (entry) =>
        entry.message ===
        "Receive address allocated: 0x6666666666666666666666666666666666666666",
    ),
  );
});

// ── Guided journey + status strip ───────────────────────────────────────────

interface JourneyStubState {
  providers: any[];
  seedProfiles: any[];
  xpubProfiles: any[];
  trackedAddressCount: number;
  reviewRequiredSteps: number;
  highFindings: number;
  criticalFindings: number;
  policy: any;
}

function journeyApiStub(state: JourneyStubState) {
  return async (_method: string, path: string) => {
    if (path === "/api/profiles/evm") return { profiles: state.providers };
    if (path === "/api/profiles/eth-seed") return { profiles: state.seedProfiles };
    if (path === "/api/profiles/eth-xpub") return { profiles: state.xpubProfiles };
    if (path === "/api/treasury/overview") {
      return {
        generated_at_unix: 1,
        tracked_address_count: state.trackedAddressCount,
        funded_address_count: 0,
        watch_only_address_count: 0,
        signer_address_count: 0,
        risk: {
          total_findings: state.highFindings + state.criticalFindings,
          critical_findings: state.criticalFindings,
          high_findings: state.highFindings,
          medium_findings: 0,
          low_findings: 0,
        },
        plans: {
          total_plans: 0,
          latest_review_required_steps: state.reviewRequiredSteps,
          latest_approved_steps: 0,
          latest_executable_steps: 0,
          latest_blocked_steps: 0,
        },
      };
    }
    if (path === "/api/treasury/policy") return { policy: state.policy };
    return {};
  };
}

test("journey card renders done/pending steps and collapses when all complete", async () => {
  const dom = installDom([
    "journeyCard",
    "journeyList",
    "journeyProgress",
    "journeyComplete",
    "statusStrip",
  ]);
  document.body.dataset.mode = "unlocked";
  const state: JourneyStubState = {
    providers: [{ name: "mainnet", chain_id: 1 }],
    seedProfiles: [],
    xpubProfiles: [],
    trackedAddressCount: 0,
    reviewRequiredSteps: 0,
    highFindings: 0,
    criticalFindings: 0,
    policy: null,
  };
  const journey = createJourneyActions({
    api: journeyApiStub(state),
    toast: () => undefined,
    jumpToCard: () => undefined,
    refreshTreasury: () => undefined,
  });

  await journey.loadJourney();
  const html = dom.el("journeyList").innerHTML;
  equal(html.split("journey-step-title").length - 1, 4);
  equal(html.split("journey-step-num").length - 1, 3);
  equal(html.split("journey-step-done").length - 1, 1);
  equal(html.split("journey-step-check").length - 1, 1);
  ok(html.includes("✓"));
  // Pending steps keep their number and an action; the done step hides its action.
  equal(html.split("journey-step-action").length - 1, 3);
  ok(html.includes("Add an RPC provider"));
  ok(html.includes("Create or import a wallet"));
  ok(html.includes("Run a balance scan"));
  ok(html.includes("Set treasury guardrails"));
  ok(html.includes('data-action="journeyRunScan"'));
  ok(html.includes('data-action="journeyJump" data-arg0="walletManagerCard"'));
  ok(html.includes('data-action="journeyJump" data-arg0="policyCard"'));
  ok(html.includes("The endpoint Sigillum uses to read balances"));
  equal(dom.el("journeyProgress").textContent, "1 of 4");
  equal(dom.el("journeyComplete").classList.contains("hidden"), true);
  // Incomplete: the card stays expanded, no collapsed-state classes.
  equal(dom.el("journeyCard").classList.contains("journey-card-complete"), false);
  equal(dom.el("journeyComplete").classList.contains("journey-complete"), false);

  // Pure step computation mirrors the rendered done flags.
  const steps = computeJourneySteps({
    providerCount: 1,
    walletCount: 0,
    trackedAddressCount: 0,
    policyConfigured: false,
    reviewNeededCount: 0,
  });
  deepEqual(steps.map((step) => step.done), [true, false, false, false]);

  // Everything finished: the card collapses into one compact ready line.
  state.seedProfiles = [{ name: "main" }];
  state.trackedAddressCount = 4;
  state.policy = { enabled: true, require_simulation: true };
  await journey.loadJourney();
  equal(dom.el("journeyList").innerHTML, "");
  equal(dom.el("journeyList").classList.contains("hidden"), true);
  equal(
    dom.el("journeyComplete").textContent,
    "Treasury ready — all setup steps complete",
  );
  equal(dom.el("journeyComplete").classList.contains("hidden"), false);
  equal(dom.el("journeyComplete").classList.contains("journey-complete"), true);
  equal(dom.el("journeyCard").classList.contains("journey-card-complete"), true);
  equal(dom.el("journeyProgress").textContent, "4 of 4");

  // A step slipping back to pending restores the full checklist card.
  state.policy = null;
  await journey.loadJourney();
  equal(dom.el("journeyCard").classList.contains("journey-card-complete"), false);
  equal(dom.el("journeyComplete").classList.contains("journey-complete"), false);
  equal(dom.el("journeyComplete").classList.contains("hidden"), true);
  equal(dom.el("journeyList").classList.contains("hidden"), false);
  equal(dom.el("journeyProgress").textContent, "3 of 4");
  ok(dom.el("journeyList").innerHTML.includes("Set treasury guardrails"));
});

test("status strip chips carry values, warn/danger tones, and jump targets", async () => {
  const dom = installDom(["journeyList", "journeyProgress", "journeyComplete", "statusStrip"]);
  document.body.dataset.mode = "unlocked";
  const state: JourneyStubState = {
    providers: [],
    seedProfiles: [],
    xpubProfiles: [],
    trackedAddressCount: 6,
    reviewRequiredSteps: 1,
    highFindings: 1,
    criticalFindings: 1,
    policy: null,
  };
  const journey = createJourneyActions({
    api: journeyApiStub(state),
    toast: () => undefined,
    jumpToCard: () => undefined,
    refreshTreasury: () => undefined,
  });

  await journey.loadJourney();
  const html = dom.el("statusStrip").innerHTML;
  equal(html.split('<button').length - 1, 4);
  // Providers and Wallets at zero warn; Review needed (1+1+1=3) is danger.
  equal(html.split("status-chip-warn").length - 1, 2);
  equal(html.split("status-chip-danger").length - 1, 1);
  ok(html.includes('<span class="status-chip-value">6</span>'));
  ok(html.includes('<span class="status-chip-value">3</span>'));
  ok(html.includes('<span class="status-chip-label">Providers</span>'));
  ok(html.includes('<span class="status-chip-label">Review needed</span>'));
  ok(html.includes('data-action="journeyJump" data-arg0="walletManagerCard"'));
  ok(html.includes('data-action="journeyJump" data-arg0="inventoryCard"'));
  ok(html.includes('data-action="journeyJump" data-arg0="treasuryCard"'));

  // Healthy counts drop the warn/danger tones.
  state.providers = [{ name: "mainnet" }];
  state.seedProfiles = [{ name: "main" }];
  state.reviewRequiredSteps = 0;
  state.highFindings = 0;
  state.criticalFindings = 0;
  await journey.loadJourney();
  const healthy = dom.el("statusStrip").innerHTML;
  equal(healthy.split("status-chip-warn").length - 1, 0);
  equal(healthy.split("status-chip-danger").length - 1, 0);

  // Outside the unlocked workspace the strip must stay empty.
  document.body.dataset.mode = "locked";
  await journey.loadJourney();
  equal(dom.el("statusStrip").innerHTML, "");
});

test("status strip gains a TTL-cached self-check chip", async () => {
  const dom = installDom(["journeyList", "journeyProgress", "journeyComplete", "statusStrip"]);
  document.body.dataset.mode = "unlocked";
  const state: JourneyStubState = {
    providers: [{ name: "mainnet" }],
    seedProfiles: [{ name: "main" }],
    xpubProfiles: [],
    trackedAddressCount: 1,
    reviewRequiredSteps: 0,
    highFindings: 0,
    criticalFindings: 0,
    policy: { enabled: true },
  };
  let ensureCalls = 0;
  let summary: { status: string; failCount: number; warnCount: number } | null = {
    status: "fail",
    failCount: 2,
    warnCount: 1,
  };
  const journey = createJourneyActions({
    api: journeyApiStub(state),
    toast: () => undefined,
    jumpToCard: () => undefined,
    refreshTreasury: () => undefined,
    ensureSelfCheck: async () => {
      ensureCalls += 1;
      return summary;
    },
  });

  await journey.loadJourney();
  // The ensure promise resolves on a microtask after render; flush it.
  await Promise.resolve();
  let html = dom.el("statusStrip").innerHTML;
  ok(html.includes('<span class="status-chip-label">Self-check issues</span>'));
  ok(html.includes('<span class="status-chip-value">3</span>'));
  ok(html.includes('data-action="journeyJump" data-arg0="diagCard"'));
  equal(html.split("status-chip-danger").length - 1, 1);
  equal(ensureCalls, 1);

  // A passing summary drops the tone but keeps the chip visible at zero.
  summary = { status: "pass", failCount: 0, warnCount: 0 };
  await journey.loadJourney();
  await Promise.resolve();
  html = dom.el("statusStrip").innerHTML;
  ok(html.includes('<span class="status-chip-label">Self-check issues</span>'));
  equal(html.split("status-chip-danger").length - 1, 0);
  equal(ensureCalls, 2);
});

test("ensureFreshSelfCheck caches within TTL and shares in-flight runs", async () => {
  installDom(["selfCheckSummary", "selfCheckList"]);
  let posts = 0;
  const actions = createSelfCheckActions({
    api: async (method, path) => {
      if (method === "POST" && path === "/api/selfcheck/run") {
        posts += 1;
        return {
          status: "warn",
          generated_at_unix: 1781125191,
          checks: [
            {
              id: "policy:treasury",
              domain: "policy",
              subject: "treasury",
              status: "warn",
              detail: "No treasury policy configured — sweeps are unguarded",
            },
          ],
        };
      }
      return {};
    },
    toast: () => undefined,
  });

  // Concurrent callers share one run; later callers hit the TTL cache.
  const [first, second] = await Promise.all([
    actions.ensureFreshSelfCheck(),
    actions.ensureFreshSelfCheck(),
  ]);
  equal(posts, 1);
  equal(first?.status, "warn");
  equal(second?.warnCount, 1);
  const third = await actions.ensureFreshSelfCheck();
  equal(posts, 1);
  equal(third?.failCount, 0);
  equal(actions.lastSelfCheckSummary()?.status, "warn");
});

test("renderEntityList object empty state renders an actionable button", () => {
  const dom = installDom(["emptyWithArg", "emptyWithoutArg"]);
  renderEntityList(
    "emptyWithArg",
    [],
    {
      message: "Nothing tracked yet.",
      actionLabel: "Run balance scan",
      action: "journeyRunScan",
      actionArg: "all",
    },
    () => "",
  );
  const html = dom.el("emptyWithArg").innerHTML;
  ok(html.includes('class="empty-state"'));
  ok(html.includes("Nothing tracked yet."));
  ok(html.includes('class="btn-ghost empty-state-action"'));
  ok(html.includes('data-action="journeyRunScan"'));
  ok(html.includes('data-arg0="all"'));
  ok(html.includes(">Run balance scan</button>"));

  renderEntityList(
    "emptyWithoutArg",
    [],
    { message: "Empty.", actionLabel: "Go", action: "focusWalletCreate" },
    () => "",
  );
  const noArgHtml = dom.el("emptyWithoutArg").innerHTML;
  ok(noArgHtml.includes('data-action="focusWalletCreate"'));
  ok(!noArgHtml.includes("data-arg0"));
});

test("renderEntityList plain-string empty state stays byte-identical (regression)", () => {
  const dom = installDom(["plainEmpty"]);
  renderEntityList("plainEmpty", [], "No entries yet.", () => "");
  equal(dom.el("plainEmpty").innerHTML, '<p class="empty-state">No entries yet.</p>');
  ok(!dom.el("plainEmpty").innerHTML.includes("empty-state-action"));

  renderEntityList("plainEmpty", [1, 2], "No entries yet.", (item) => "<li>" + item + "</li>");
  equal(
    dom.el("plainEmpty").innerHTML,
    '<ul class="entity-list"><li>1</li><li>2</li></ul>',
  );
});

// ── Self-check ──────────────────────────────────────────────────────────────

function installSelfCheckDom() {
  const dom = installDom(["selfCheckSummary", "selfCheckList"]);
  dom.el("selfCheckRunDiag", "BUTTON");
  dom.el("selfCheckRunTreasury", "BUTTON");
  return dom;
}

test("self-check run renders domain-grouped rows, latency, and summary counts", async () => {
  const dom = installSelfCheckDom();
  const toasts: Array<{ message: string; type?: string }> = [];
  // Deliberately out of contract order: rendering must follow the stable
  // domain order (provider before policy before fido2), not response order.
  const response: SelfCheckRunResponse = {
    status: "pass",
    generated_at_unix: 1749700000,
    checks: [
      {
        id: "fido2-1",
        domain: "fido2",
        subject: "yubikey-a",
        status: "pass",
        detail: "credential present",
      },
      {
        id: "provider-1",
        domain: "provider",
        subject: "mainnet",
        status: "pass",
        detail: "chain 1 · block 19999999",
        latency_ms: 42,
      },
      {
        id: "provider-2",
        domain: "provider",
        subject: "base",
        status: "pass",
        detail: "chain 8453",
        latency_ms: 7,
      },
      {
        id: "policy-1",
        domain: "policy",
        subject: "treasury policy",
        status: "pass",
        detail: "destinations valid",
      },
    ],
  };
  let requested: { method: string; path: string; body?: any } | null = null;
  const selfCheck = createSelfCheckActions({
    api: async (method, path, body) => {
      requested = { method, path, body };
      return response;
    },
    toast: (message, type) => toasts.push({ message, type }),
  });

  await selfCheck.runSelfCheck();

  deepEqual(requested, { method: "POST", path: "/api/selfcheck/run", body: {} });
  equal(
    dom.el("selfCheckSummary").textContent,
    "4 pass · 0 warn · 0 fail · ran " + formatClockTime(1749700000),
  );
  const html = dom.el("selfCheckList").innerHTML;
  ok(
    html.includes(
      '<div class="section-title">provider <span class="text-meta">· 2</span></div>',
    ),
  );
  ok(
    html.includes(
      '<div class="section-title">policy <span class="text-meta">· 1</span></div>',
    ),
  );
  ok(
    html.includes(
      '<div class="section-title">fido2 <span class="text-meta">· 1</span></div>',
    ),
  );
  ok(html.indexOf(">provider <") < html.indexOf(">policy <"));
  ok(html.indexOf(">policy <") < html.indexOf(">fido2 <"));
  ok(html.includes("mainnet"));
  ok(html.includes('class="pill pill-good">pass<'));
  // Latency renders as a meta suffix; checks without latency omit it.
  ok(html.includes("chain 1 · block 19999999 · 42ms"));
  ok(html.includes("chain 8453 · 7ms"));
  ok(html.includes("credential present</div>"));
  deepEqual(toasts.pop(), {
    message: "Self-check passed: 4 checks green.",
    type: undefined,
  });
});

test("self-check failures toast an error and surface pill counts in the summary", async () => {
  const dom = installSelfCheckDom();
  const toasts: Array<{ message: string; type?: string }> = [];
  let overall: "warn" | "fail" = "fail";
  const selfCheck = createSelfCheckActions({
    api: async () => ({
      status: overall,
      generated_at_unix: 1749700000,
      checks: [
        {
          id: "provider-1",
          domain: "provider",
          subject: "mainnet",
          status: "pass",
          detail: "chain 1",
          latency_ms: 12,
        },
        {
          id: "seed-1",
          domain: "seed-wallet",
          subject: "main",
          status: "warn",
          detail: "provider unreachable for balance read",
        },
        {
          id: "xpub-1",
          domain: "xpub-wallet",
          subject: "cold-watch",
          status: "fail",
          detail: "receive xpub mismatch",
        },
      ],
    }),
    toast: (message, type) => toasts.push({ message, type }),
  });

  await selfCheck.runSelfCheck();
  deepEqual(toasts.pop(), {
    message: "Self-check: 2 issue(s) found — see System section",
    type: "error",
  });
  const summary = dom.el("selfCheckSummary").innerHTML;
  ok(summary.includes('class="pill pill-good">1 pass<'));
  ok(summary.includes('class="pill pill-warn">1 warn<'));
  ok(summary.includes('class="pill pill-danger">1 fail<'));
  ok(summary.includes("ran " + formatClockTime(1749700000)));
  const list = dom.el("selfCheckList").innerHTML;
  ok(list.includes('class="pill pill-danger">fail<'));
  ok(list.includes('class="pill pill-warn">warn<'));
  ok(list.includes("receive xpub mismatch"));
  ok(list.includes(">seed-wallet <"));

  // Overall warn (no hard failures) keeps the default toast tone.
  overall = "warn";
  await selfCheck.runSelfCheck();
  deepEqual(toasts.pop(), {
    message: "Self-check: 2 issue(s) found — see System section",
    type: undefined,
  });
});

test("self-check panel offers a run action before the first run of a session", () => {
  const dom = installSelfCheckDom();
  const selfCheck = createSelfCheckActions({
    api: async () => ({}),
    toast: () => undefined,
  });

  selfCheck.renderSelfCheckPanel();
  const html = dom.el("selfCheckList").innerHTML;
  ok(html.includes('class="empty-state"'));
  ok(html.includes("Not run yet in this session."));
  ok(html.includes('class="btn-ghost empty-state-action"'));
  ok(html.includes('data-action="runSelfCheck"'));
  ok(html.includes(">Run Self-Check</button>"));
  equal(dom.el("selfCheckSummary").textContent, "");
});

test("self-check toggles btn-busy on both run buttons while the request is in flight", async () => {
  const dom = installSelfCheckDom();
  let release: (value: SelfCheckRunResponse) => void = () => undefined;
  const gate = new Promise<SelfCheckRunResponse>((resolve) => {
    release = resolve;
  });
  let busyMidFlight: boolean | null = null;
  const selfCheck = createSelfCheckActions({
    api: () => {
      // Inspect the buttons mid-flight, before the deferred response lands.
      busyMidFlight =
        dom.el("selfCheckRunDiag").classList.contains("btn-busy") &&
        dom.el("selfCheckRunTreasury").classList.contains("btn-busy");
      return gate;
    },
    toast: () => undefined,
  });

  const pending = selfCheck.runSelfCheck();
  equal(busyMidFlight, true);
  equal(dom.el("selfCheckRunDiag").classList.contains("btn-busy"), true);
  equal(dom.el("selfCheckRunTreasury").classList.contains("btn-busy"), true);
  release({ status: "pass", generated_at_unix: 1, checks: [] });
  await pending;
  equal(dom.el("selfCheckRunDiag").classList.contains("btn-busy"), false);
  equal(dom.el("selfCheckRunTreasury").classList.contains("btn-busy"), false);
});

test("pillClass maps self-check statuses onto existing pill buckets", () => {
  equal(pillClass("pass"), "pill-good");
  equal(pillClass("fail"), "pill-danger");
  equal(pillClass("warn"), "pill-warn");
  equal(pillClass("underfunded"), "pill-warn");
  equal(pillClass("something-else"), "pill-neutral");
});
