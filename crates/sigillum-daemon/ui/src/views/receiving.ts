import type {
  Counterparty,
  ReceivingItem,
  ReceivingOverviewResponse,
  ReceivingPartyGroup,
  ReceivingDepositTagRequest,
  ReceivingRefreshResponse,
} from "../contracts";
import { setTextById as setText } from "../render/dom";
import { esc, escAttr, formatTs, pillClass, statBox, statusPill } from "../render/html";
import { formatWeiHexAsEth } from "./treasury";

export interface ReceivingActionsDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  jumpToField: (cardId: string, inputId: string) => void;
  jumpToCard: (cardId: string) => void;
}

export function truncateAddress(addr: string): string {
  if (!addr.startsWith("0x") || addr.length <= 10) return addr;
  return addr.slice(0, 6) + "…" + addr.slice(-4);
}

function sourceBadge(item: ReceivingItem): string {
  const label =
    item.source_type === "hd"
      ? "HD"
      : item.source_type === "stealth"
        ? "Stealth"
        : item.source_type || "unknown";
  return '<span class="pill ' + pillClass(item.source_type) + '">' + esc(label) + "</span>";
}

function itemPurposeLine(item: ReceivingItem): string {
  const parts: string[] = [];
  if (item.purpose) parts.push("purpose=" + esc(item.purpose));
  if (item.label) parts.push("label=" + esc(item.label));
  return parts.length ? parts.join(" · ") + "<br>" : "";
}

function itemBalanceLine(item: ReceivingItem): string {
  if (!item.balance_known) return "balance unknown — refresh in B2";
  return "balance=" + esc(formatWeiHexAsEth(item.balance_native_wei_hex || "0x0")) + " ETH";
}

function renderLinkageWarning(item: ReceivingItem): string {
  if (!item.linkage_warning) return "";
  return (
    '<div class="plan-linkage-banner"><strong>Caution</strong><br>' +
    '<span class="linkage-warning">' +
    esc(item.linkage_warning) +
    '</span><br><span class="linkage-warning">Scope: flags payers that would sweep to the same destination. Does not cover gas-funding links, amount/timing correlation, downstream re-merging, or multi-hop flows — keep per-party destinations separate.' +
    "</span></div>"
  );
}

function renderStealthCounterpartySelect(
  item: ReceivingItem,
  receivingParties: Counterparty[],
  stealthDepositIdByAddress: Record<string, string>,
): string {
  const addressKey = item.address.toLowerCase();
  let html =
    '<select class="select-inline" data-action="tagStealthDeposit" data-arg0="' +
    escAttr(addressKey) +
    '" data-deposit-known="' +
    escAttr(stealthDepositIdByAddress[addressKey] ? "true" : "false") +
    '" data-self="append" aria-label="Counterparty for ' +
    escAttr(truncateAddress(item.address)) +
    '">' +
    '<option value=""' +
    (item.counterparty_id ? "" : " selected") +
    ">Unassigned</option>";
  receivingParties.forEach((party) => {
    html +=
      '<option value="' +
      escAttr(party.id) +
      '"' +
      (item.counterparty_id === party.id ? " selected" : "") +
      ">" +
      esc(party.name) +
      "</option>";
  });
  return html + "</select>";
}

function renderReceivingItem(
  item: ReceivingItem,
  receivingParties: Counterparty[],
  stealthDepositIdByAddress: Record<string, string>,
): string {
  let actions =
    '<button class="btn-ghost" data-action="copyText" data-arg0="' +
    escAttr(item.address) +
    '" data-arg1="Receiving address">Copy</button>';
  if (item.source_type === "stealth") {
    actions += renderStealthCounterpartySelect(item, receivingParties, stealthDepositIdByAddress);
  }

  return (
    '<li><div class="entity-main">' +
    '<div class="entity-title">' +
    sourceBadge(item) +
    ' <span class="mono">' +
    esc(truncateAddress(item.address)) +
    "</span> " +
    statusPill(item.status) +
    "</div>" +
    '<div class="entity-meta">' +
    itemPurposeLine(item) +
    itemBalanceLine(item) +
    "<br>" +
    "chain=" +
    esc(String(item.chain_id)) +
    (item.derivation_path ? " · path=" + esc(item.derivation_path) : "") +
    " · created=" +
    esc(formatTs(item.created_at_unix)) +
    "</div>" +
    renderLinkageWarning(item) +
    "</div>" +
    '<div class="entity-actions">' +
    actions +
    "</div></li>"
  );
}

function renderReceivingGroup(
  group: ReceivingPartyGroup,
  receivingParties: Counterparty[],
  stealthDepositIdByAddress: Record<string, string>,
): string {
  const party = group.counterparty || null;
  let html =
    '<div class="section-title">' +
    esc(party ? party.name : "Unassigned") +
    "</div>" +
    '<p class="text-meta">' +
    (party?.note ? esc(party.note) + " · " : "") +
    esc(formatWeiHexAsEth(group.native_total_wei_hex)) +
    " ETH · " +
    esc(String(group.item_count)) +
    " items</p>" +
    '<ul class="entity-list">';
  (group.items || []).forEach((item) => {
    html += renderReceivingItem(item, receivingParties, stealthDepositIdByAddress);
  });
  html += "</ul>";
  return html;
}

function renderReceivingEmptyState(): string {
  return (
    '<p class="empty-state">' +
    "No private receiving surfaces yet. " +
    '<button type="button" class="btn-ghost empty-state-action" data-action="focusReceivingAllocate">Allocate a receiving address</button> ' +
    '<button type="button" class="btn-ghost empty-state-action" data-action="focusReceivingStealth">Create a stealth deposit</button>' +
    "</p>"
  );
}

function renderReceivingQuickActions(): void {
  const el = document.getElementById("receivingQuickActions");
  if (!el) return;
  el.innerHTML =
    '<button class="btn-ghost btn-small" data-action="focusReceivingAllocate">Allocate fresh address</button>' +
    '<button class="btn-ghost btn-small" data-action="focusReceivingStealth">Stealth meta-address &amp; scan</button>';
}

export function renderReceivingOverview(
  overview: ReceivingOverviewResponse,
  receivingParties: Counterparty[] = [],
  stealthDepositIdByAddress: Record<string, string> = {},
): void {
  const statsEl = document.getElementById("receivingOverviewStats");
  if (statsEl) {
    statsEl.innerHTML = [
      statBox(formatWeiHexAsEth(overview.totals.native_total_wei_hex) + " ETH", "Total Received"),
      statBox(String(overview.totals.item_count || 0), "Receiving Surfaces"),
      statBox(String(overview.totals.hd_count || 0), "HD Addresses"),
      statBox(String(overview.totals.stealth_count || 0), "Stealth Deposits"),
    ].join("");
  }

  setText(
    "receivingCoverage",
    String(overview.coverage.addresses_with_known_balance || 0) +
      " of " +
      String(overview.coverage.addresses_total || 0) +
      " addresses have a known balance — " +
      overview.coverage.note,
  );
  renderReceivingQuickActions();

  const groupsEl = document.getElementById("receivingGroupList");
  if (!groupsEl) return;
  const groups = overview.groups || [];
  if (!groups.length || overview.totals.item_count === 0) {
    groupsEl.innerHTML = renderReceivingEmptyState();
    return;
  }
  let html = "";
  groups.forEach((group) => {
    html += renderReceivingGroup(group, receivingParties, stealthDepositIdByAddress);
  });
  groupsEl.innerHTML = html;
}

export function createReceivingActions(deps: ReceivingActionsDeps) {
  let receivingParties: Counterparty[] = [];
  let stealthDepositIdByAddress: Record<string, string> = {};

  async function loadReceivingOverview(): Promise<void> {
    try {
      const r = await deps.api("GET", "/api/receiving/overview");
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      const overview = r as ReceivingOverviewResponse;
      renderReceivingOverview(overview, receivingParties, stealthDepositIdByAddress);
      try {
        const parties = await deps.api("GET", "/api/treasury/parties");
        if (!parties.error) receivingParties = parties.parties || [];
      } catch (_) {
        // Overview rendering should not depend on optional selector metadata.
      }
      try {
        const deposits = await deps.api("GET", "/api/deposits/eth-stealth");
        if (!deposits.error) {
          stealthDepositIdByAddress = {};
          (deposits.deposits || []).forEach((deposit: any) => {
            if (deposit?.id && deposit?.stealth_address) {
              stealthDepositIdByAddress[String(deposit.stealth_address).toLowerCase()] = String(
                deposit.id,
              );
            }
          });
        }
      } catch (_) {
        // Keep the overview usable even if the deposit registry is unavailable.
      }
      renderReceivingOverview(overview, receivingParties, stealthDepositIdByAddress);
    } catch (_) {
      deps.toast("Receiving overview unavailable", "error");
    }
  }

  async function refreshReceivingBalances(): Promise<void> {
    try {
      const r = await deps.api("POST", "/api/receiving/refresh-balances");
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      const response = r as ReceivingRefreshResponse;
      if (response.provider_status === "no_provider") {
        deps.toast("Configure an RPC provider before refreshing receiving balances.", "error");
      } else if (response.provider_status === "partial") {
        const firstError = response.errors && response.errors.length ? ": " + response.errors[0] : "";
        deps.toast(
          "Receiving balances partially refreshed; some addresses failed" + firstError,
          "warning",
        );
      } else {
        deps.toast(
          "Receiving balances refreshed: " +
            String(response.addresses_refreshed || 0) +
            " addresses" +
            (response.addresses_skipped
              ? ", " + String(response.addresses_skipped) + " skipped by cap"
              : ""),
        );
      }
      await loadReceivingOverview();
    } catch (_) {
      deps.toast("Receiving balance refresh unavailable", "error");
    }
  }

  function focusReceivingAllocate(): void {
    deps.jumpToField("treasuryCard", "treasuryReceivePurpose");
  }

  function focusReceivingStealth(): void {
    deps.jumpToCard("depositsCard");
  }

  async function tagStealthDeposit(address: unknown, selectEl: unknown): Promise<void> {
    const select = selectEl as { value?: unknown } | null;
    if (!select || typeof select.value !== "string") {
      deps.toast("Counterparty selector unavailable", "error");
      return;
    }
    const depositId = stealthDepositIdByAddress[String(address).toLowerCase()];
    if (!depositId) {
      deps.toast("Deposit id unavailable for this stealth address", "error");
      return;
    }
    const value = select.value;
    const body: ReceivingDepositTagRequest = {
      deposit_id: depositId,
      counterparty_id: value || null,
    };
    const r = await deps.api("POST", "/api/receiving/deposits/tag", body);
    if (r.error) {
      deps.toast(r.error, "error");
      return;
    }
    deps.toast("Counterparty updated");
    await loadReceivingOverview();
  }

  return {
    loadReceivingOverview,
    refreshReceivingBalances,
    renderReceivingOverview,
    focusReceivingAllocate,
    focusReceivingStealth,
    tagStealthDeposit,
  };
}
