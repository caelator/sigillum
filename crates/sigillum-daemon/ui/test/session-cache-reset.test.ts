import { deepEqual, equal, ok } from "node:assert/strict";
import { test } from "node:test";

import {
  isSessionContextChangedError,
  SessionContextChangedError,
} from "../src/api/session";
import { createFido2Actions } from "../src/views/fido2";
import { createOperationsActions } from "../src/views/operations";
import { createReceivingActions } from "../src/views/receiving";
import { createTreasuryActions } from "../src/views/treasury";
import { createWalletManagerActions } from "../src/views/walletManager";
import { createWalletActions } from "../src/views/wallets";
import { installDom } from "./dom-fixture";

async function expectSessionContextChanged(work: Promise<unknown>): Promise<void> {
  try {
    await work;
  } catch (error) {
    ok(isSessionContextChangedError(error));
    return;
  }
  throw new Error("expected SessionContextChangedError");
}

test("session reset empties profile, operation, and hardware caches", async () => {
  installDom(["fido2Card", "fido2DeviceStatus", "fido2KeyListSection"]);

  const wallets = createWalletActions({
    api: async (_method, path) => {
      if (path === "/api/profiles/evm") return { profiles: [{ name: "old-provider" }] };
      if (path === "/api/profiles/eth-stealth") return { profiles: [{ name: "old-stealth" }] };
      if (path === "/api/profiles/eth-xpub") return { profiles: [{ name: "old-xpub" }] };
      if (path === "/api/profiles/eth-seed") return { profiles: [{ name: "old-seed" }] };
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    copyText: async () => undefined,
  });
  await wallets.loadProfiles();
  deepEqual(
    Object.values(wallets.getState()).map((profiles) => profiles.length),
    [1, 1, 1, 1],
  );
  wallets.resetSession();
  deepEqual(wallets.getState(), {
    providerProfiles: [],
    walletProfiles: [],
    xpubWalletProfiles: [],
    seedWalletProfiles: [],
  });

  const operations = createOperationsActions({
    api: async (_method, path) => {
      if (path === "/api/deposits/eth-stealth") return { deposits: [{ id: "old-deposit" }] };
      if (path === "/api/queue/jobs") return { jobs: [{ id: "old-job" }] };
      if (path === "/api/treasury/policy") return { policy: { execution_paused: false } };
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  await Promise.all([operations.loadDepositRegistry(), operations.loadQueueJobs()]);
  deepEqual(operations.getState(), {
    deposits: [{ id: "old-deposit" }],
    queueJobs: [{ id: "old-job" }],
  });
  operations.resetSession();
  deepEqual(operations.getState(), { deposits: [], queueJobs: [] });

  const fido2 = createFido2Actions({
    api: async (_method, path) =>
      path === "/api/fido2/detect"
        ? { device_present: true, device_count: 1 }
        : { keys: [{ label: "old-key" }] },
    toast: () => undefined,
    refresh: () => undefined,
    currentStatus: () => ({ locked: false }),
  });
  await fido2.loadFido2();
  equal(fido2.getState().detect.device_count, 1);
  deepEqual(fido2.getState().keys, [{ label: "old-key" }]);
  fido2.resetSession();
  deepEqual(fido2.getState(), { detect: null, keys: [] });
});

test("wallet-manager reset destroys mnemonic and disarms pending actions", async () => {
  const dom = installDom([
    "walletCreateSubmit",
    "walletCreateName",
    "walletCreateProvider",
    "walletCreateAccount",
    "walletCreateWords12",
    "walletCreateWords24",
    "walletCreateLabel",
    "walletCreateChainId",
    "walletCreateDestination",
    "walletCreatePassphrase",
    "walletMnemonicReveal",
    "walletManagerList",
    "walletReceiveTarget",
    "walletReceivePanel",
    "walletReceivePurpose",
    "walletReceiveLabel",
  ]);
  const calls: Array<{ method: string; path: string; body?: unknown }> = [];
  const toasts: string[] = [];
  const mnemonic = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
  const manager = createWalletManagerActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/profiles/eth-seed/create") return { mnemonic };
      if (path === "/api/profiles/eth-seed") return { profiles: [{ name: "old-seed" }] };
      if (path === "/api/profiles/eth-xpub") return { profiles: [] };
      if (path === "/api/profiles/evm") return { profiles: [{ name: "old-provider", chain_id: 1 }] };
      if (path === "/api/treasury/overview") return { groups: [] };
      if (path === "/api/treasury/receive-addresses") return { allocations: [] };
      return {};
    },
    toast: (message) => toasts.push(message),
  });

  await manager.loadWalletManager();
  manager.resetSession();
  manager.renderWalletManagerList();
  ok(/No wallets yet/.test(dom.el("walletManagerList").innerHTML));

  dom.el("walletCreateName").value = "new-wallet";
  dom.el("walletCreateProvider").value = "old-provider";
  dom.el("walletCreateAccount").value = "0";
  dom.el("walletCreateWords24").checked = true;
  await manager.createWallet();
  equal(manager.hasPendingMnemonic(), true);
  ok(/alpha/.test(dom.el("walletMnemonicReveal").innerHTML));

  manager.promptWalletReceiveAddress("old-seed");
  await manager.deleteManagedWallet("seed", "old-seed");
  manager.resetSession();

  equal(manager.hasPendingMnemonic(), false);
  equal(dom.el("walletMnemonicReveal").innerHTML, "");
  equal(dom.el("walletMnemonicReveal").classList.contains("hidden"), true);
  await manager.allocateWalletReceiveAddress();
  equal(toasts.at(-1), 'Pick a wallet via its "Receive address" action first');

  const deleteCallsBefore = calls.filter((call) => call.path.endsWith("/delete")).length;
  await manager.deleteManagedWallet("seed", "old-seed");
  equal(
    calls.filter((call) => call.path.endsWith("/delete")).length,
    deleteCallsBefore,
  );
  manager.resetSession();
});

test("session reset invalidates receiving and treasury party lookups", async () => {
  installDom();
  const calls: Array<{ method: string; path: string; body?: unknown }> = [];
  const toasts: string[] = [];
  const party = { id: "old-party", name: "Old payer", created_at_unix: 1 };

  const receiving = createReceivingActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/receiving/overview") {
        return {
          generated_at_unix: 1,
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
            note: "fixture",
          },
        };
      }
      if (path === "/api/treasury/parties") return { parties: [party] };
      if (path === "/api/deposits/eth-stealth") {
        return { deposits: [{ id: "old-deposit", stealth_address: "0xabc" }] };
      }
      return {};
    },
    toast: (message) => toasts.push(message),
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });
  await receiving.loadReceivingOverview();
  receiving.resetSession();
  await receiving.tagStealthDeposit("0xabc", { value: "old-party" });
  equal(toasts.at(-1), "Deposit id unavailable for this stealth address");
  equal(calls.some((call) => call.path === "/api/receiving/deposits/tag"), false);

  const treasury = createTreasuryActions({
    api: async (method, path, body) => {
      calls.push({ method, path, body });
      if (path === "/api/treasury/parties") return { parties: [party] };
      return {};
    },
    toast: (message) => toasts.push(message),
  });
  await treasury.loadTreasuryParties();
  treasury.resetSession();
  await treasury.clearTreasuryPartySweepDest("old-party");
  equal(toasts.at(-1), "Counterparty not found");
  equal(
    calls.some(
      (call) => call.path === "/api/treasury/parties/update" && call.method === "POST",
    ),
    false,
  );
});

test("session context changes reject atomic FIDO, queue, and wallet-manager loads", async () => {
  const dom = installDom([
    "fido2Card",
    "fido2DeviceStatus",
    "fido2KeyListSection",
    "queueList",
    "queuePausedBanner",
    "queuePauseBtn",
    "queueResumeBtn",
    "walletManagerList",
    "walletCreateProvider",
    "walletImportSeedProvider",
    "walletImportXpubProvider",
    "walletCreateProviderHint",
    "walletQuickProvider",
    "walletCreateSubmit",
  ]);

  let fidoPhase: "old" | "changed" = "old";
  const fidoCalls: string[] = [];
  const fido2 = createFido2Actions({
    api: async (_method, path) => {
      fidoCalls.push(path);
      if (fidoPhase === "changed" && path === "/api/fido2/list") {
        throw new SessionContextChangedError();
      }
      if (path === "/api/fido2/detect") {
        return {
          device_present: true,
          device_count: fidoPhase === "old" ? 1 : 2,
        };
      }
      return {
        keys: [{ label: fidoPhase === "old" ? "old-key" : "new-key" }],
      };
    },
    toast: () => undefined,
    refresh: () => undefined,
    currentStatus: () => ({ locked: false }),
  });
  await fido2.loadFido2();
  fidoPhase = "changed";
  await expectSessionContextChanged(fido2.loadFido2());
  deepEqual(fidoCalls.slice(-2), ["/api/fido2/detect", "/api/fido2/list"]);
  equal(fido2.getState().detect.device_count, 1);
  deepEqual(fido2.getState().keys, [{ label: "old-key" }]);

  let operationPhase: "old" | "queue-changed" | "deposit-changed" = "old";
  const operations = createOperationsActions({
    api: async (_method, path) => {
      if (operationPhase === "queue-changed" && path === "/api/treasury/policy") {
        throw new SessionContextChangedError();
      }
      if (
        operationPhase === "deposit-changed" &&
        path === "/api/deposits/eth-stealth"
      ) {
        throw new SessionContextChangedError();
      }
      if (path === "/api/deposits/eth-stealth") {
        return { deposits: [{ id: "old-deposit" }] };
      }
      if (path === "/api/queue/jobs") {
        return {
          jobs: [{ id: operationPhase === "old" ? "old-job" : "new-job" }],
        };
      }
      if (path === "/api/treasury/policy") {
        return { policy: { execution_paused: false } };
      }
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  await Promise.all([operations.loadDepositRegistry(), operations.loadQueueJobs()]);
  operationPhase = "queue-changed";
  await expectSessionContextChanged(operations.loadQueueJobs());
  deepEqual(operations.getState().queueJobs, [{ id: "old-job" }]);
  operationPhase = "deposit-changed";
  await expectSessionContextChanged(operations.loadDepositRegistry());
  deepEqual(operations.getState().deposits, [{ id: "old-deposit" }]);

  let walletPhase: "old" | "changed" = "old";
  const walletToasts: string[] = [];
  const manager = createWalletManagerActions({
    api: async (_method, path) => {
      if (walletPhase === "changed" && path === "/api/treasury/receive-addresses") {
        throw new SessionContextChangedError();
      }
      if (path === "/api/profiles/eth-seed" || path === "/api/profiles/eth-xpub") {
        return { profiles: [] };
      }
      if (path === "/api/profiles/evm") {
        return {
          profiles: [
            {
              name: walletPhase === "old" ? "old-provider" : "new-provider",
              chain_id: 1,
            },
          ],
        };
      }
      if (path === "/api/treasury/overview") return { groups: [] };
      if (path === "/api/treasury/receive-addresses") return { allocations: [] };
      return {};
    },
    toast: (message) => walletToasts.push(message),
  });
  await manager.loadWalletManager();
  ok(dom.el("walletCreateProvider").innerHTML.includes("old-provider"));
  walletPhase = "changed";
  await expectSessionContextChanged(manager.refreshWalletManager());
  ok(dom.el("walletCreateProvider").innerHTML.includes("old-provider"));
  equal(dom.el("walletCreateProvider").innerHTML.includes("new-provider"), false);
  deepEqual(walletToasts, []);
});

test("session context changes escape receiving refresh and operation follow-up loaders", async () => {
  installDom([
    "receivingOverviewStats",
    "receivingCoverage",
    "receivingQuickActions",
    "receivingGroupList",
    "depositScanWalletProfile",
    "depositScanFromBlock",
    "depositScanToBlock",
    "depositScanTokenAddress",
    "depositScanLimit",
    "depositScanAutoQueue",
    "depositScanDestination",
    "depositScanMinSweep",
    "depositScanNote",
    "depositRefreshLimit",
    "depositRefreshAutoEnqueue",
    "depositList",
    "queueList",
  ]);
  const overview = {
    generated_at_unix: 1,
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
      note: "fixture",
    },
  };
  let receivingPhase: "overview" | "refresh" = "overview";
  const receivingToasts: string[] = [];
  const receiving = createReceivingActions({
    api: async (method, path) => {
      if (path === "/api/treasury/parties") {
        if (receivingPhase === "overview") throw new SessionContextChangedError();
        return { parties: [] };
      }
      if (path === "/api/deposits/eth-stealth") return { deposits: [] };
      if (path === "/api/receiving/refresh-balances") {
        return {
          provider_status: "ok",
          addresses_refreshed: 1,
          addresses_skipped: 0,
        };
      }
      if (method === "GET" && path === "/api/receiving/overview") {
        if (receivingPhase === "refresh") throw new SessionContextChangedError();
        return overview;
      }
      return {};
    },
    toast: (message) => receivingToasts.push(message),
    jumpToField: () => undefined,
    jumpToCard: () => undefined,
  });
  await expectSessionContextChanged(receiving.loadReceivingOverview());
  receivingPhase = "refresh";
  await expectSessionContextChanged(receiving.refreshReceivingBalances());
  equal(receivingToasts.includes("Receiving overview unavailable"), false);
  equal(receivingToasts.includes("Receiving balance refresh unavailable"), false);

  const scanWallet = document.getElementById("depositScanWalletProfile") as HTMLInputElement;
  const scanFrom = document.getElementById("depositScanFromBlock") as HTMLInputElement;
  scanWallet.value = "main";
  scanFrom.value = "1";
  let operationPhase: "scan" | "refresh" = "scan";
  const operations = createOperationsActions({
    api: async (method, path) => {
      if (method === "POST" && path === "/api/deposits/eth-stealth/scan-announcements") {
        return { scanned: 1, matched: 1, created: 1, existing: 0 };
      }
      if (method === "POST" && path === "/api/deposits/eth-stealth/refresh") {
        return { processed: 1, detected: 1, queued: 0, deposits: [] };
      }
      if (path === "/api/deposits/eth-stealth") {
        throw new SessionContextChangedError();
      }
      if (operationPhase === "refresh" && path === "/api/queue/jobs") {
        throw new SessionContextChangedError();
      }
      if (path === "/api/treasury/policy") {
        return { policy: { execution_paused: false } };
      }
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    showResultBox: () => undefined,
    updateNextStepCard: () => undefined,
  });
  await expectSessionContextChanged(operations.scanEthStealthAnnouncements());
  operationPhase = "refresh";
  await expectSessionContextChanged(operations.refreshDepositRegistry());
});
