/**
 * destinations/Receiving.ts — the rebuilt Receive destination (plan §4.3.3).
 *
 * The operator question this screen answers: "how do I get paid privately,
 * and what is happening with the payments coming in?"
 *
 *   (a) Address cards grid — every allocation/stealth surface as a card:
 *       middle-truncated address with copy, purpose, counterparty, balance in
 *       ETH, freshness, one-time badge + lifecycle pill (plan task 3.3).
 *       Allocate/rotate keep the legacy endpoints and DTOs.
 *   (b) One-time-address mode as a first-class flow: sweep destination,
 *       threshold, and purge-after-sweep in plain language with an explainer.
 *   (c) Stealth deposits rendered as a guided lifecycle (announced → funded →
 *       gas-ready → swept) with the 2.4 gas story (requested payer gas,
 *       sponsor top-up, funded_needs_gas explainer), a sweep action behind
 *       the shared confirm dialog, and counterparty tagging.
 *   (d) Payer instructions panel: the wallet's stealth meta-address with copy
 *       plus exactly what a payer attaches (metadata, gas option) — "how do I
 *       get paid privately?" answered on the screen.
 *   (e) Refresh balances with progress feedback.
 *
 * Design-system v2 rules hold throughout (ui/DESIGN.md): textContent-only
 * data flow, keyed lists, human units in the default view (raw values only
 * behind details disclosures), no inline styles, no key=value meta lines.
 *
 * The typed core client covers the overview and deposit lists; everything
 * else is a thin wrapper around `requestWithSession` kept LOCAL to this
 * module (core/api.ts is shared and not ours to edit).
 */

import type {
  Counterparty,
  EthStealthDeposit,
  EthStealthDepositListQuery,
  PaginationInfo,
  ReceivingOverviewResponse,
  ReceivingRefreshResponse,
  TreasuryReceiveAllocation,
} from "../contracts";
import { ApiError, apiFailure } from "../core/api";
import { el, renderList } from "../core/dom";
import type { CoreRuntime } from "../core/live";
import type { DestinationController, Route } from "../core/router";
import type { Unsubscribe } from "../core/store";
import { requestWithSession } from "../api/session";
import { confirmDangerDialog, informDialog } from "../render/confirm";
import { formatEthAmount, formatTimestamp } from "../render/format";
// Legacy conversions reused unchanged so the wire DTOs stay identical to the
// views this destination replaces (allocate one_time threshold in wei hex).
import { parseEthToWeiHex } from "../views/treasury";
import { truncateAddress } from "../views/receiving";

// ── Local API wrappers (thin, around requestWithSession) ─────────────

type Method = "GET" | "POST" | "DELETE";

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  let payload: {
    code?: string;
    error?: string;
    action?: string;
    fields?: { field: string; message: string }[];
  } | null;
  try {
    payload = await requestWithSession(method, path, body);
  } catch (error) {
    throw new ApiError({
      code: "unavailable",
      error: error instanceof Error ? error.message : String(error),
    });
  }
  if (payload && payload.error != null) {
    throw new ApiError({
      code: payload.code ?? "unknown",
      error: payload.error,
      action: payload.action,
      fields: payload.fields,
    });
  }
  return payload as T;
}

interface AllocationListResponse {
  allocations?: TreasuryReceiveAllocation[];
}

interface PartyListResponse {
  parties?: Counterparty[];
}

interface PartyMutationResponse {
  party?: Counterparty;
}

interface StealthProfileListResponse {
  profiles?: { name?: string; wallet?: string }[];
}

interface DepositRefreshResult {
  processed?: number;
  detected?: number;
  queued?: number;
}

interface DepositMutationResponse {
  deposit?: EthStealthDeposit;
}

interface ScanAnnouncementsResult {
  scanned?: number;
  matched?: number;
  created?: number;
  existing?: number;
}

interface CreateDepositResult {
  deposit?: EthStealthDeposit;
  warnings?: string[];
}

interface EnqueueSweepResult {
  job?: { id?: string };
}

interface MetaAddressExportResult {
  stealth_meta_address?: string;
  scheme_id?: number;
}

// ── Pure presentation helpers (exported for tests) ────────────────────

/** Deposit lifecycle stages: announced → funded → gas-ready → swept. */
export const DEPOSIT_STAGES = ["Announced", "Funded", "Gas ready", "Swept"] as const;

export interface DepositLifecycle {
  /** Count of completed stages (1–4). 4 = swept (terminal good state). */
  completed: number;
  /** The current stage needs the operator (gas, partial payment, failed sweep). */
  attention: boolean;
}

/** Map a deposit status onto the guided lifecycle (plan §4.3.3c). */
export function depositLifecycle(deposit: EthStealthDeposit): DepositLifecycle {
  switch (deposit.status) {
    case "sweep_confirmed":
      return { completed: 4, attention: false };
    case "funded":
    case "sweep_queued":
    case "sweep_prepared":
    case "sweep_retrying":
    case "sweep_submitted_unknown":
    case "sweep_sent":
      return { completed: 3, attention: false };
    case "sweep_blocked":
    case "sweep_failed":
    case "sweep_operator_action_required":
      return { completed: 3, attention: true };
    case "funded_needs_gas":
      return { completed: 2, attention: true };
    case "underfunded":
      return { completed: 1, attention: true };
    case "pending":
    default:
      return { completed: 1, attention: false };
  }
}

/** One plain-language sentence for where a deposit stands right now. */
export function depositStatusLine(deposit: EthStealthDeposit): string {
  switch (deposit.status) {
    case "pending":
      return "Address ready — waiting for the payment to arrive.";
    case "underfunded":
      return "A partial payment arrived — below the expected amount.";
    case "funded_needs_gas":
      return deposit.gas_topup_job_id
        ? "Payment received. Waiting for the sponsor gas top-up to confirm before the sweep can run."
        : "Payment received, but the address holds no native gas for the sweep — ask the payer to attach gas, or fund the address manually.";
    case "funded":
      return "Payment received and gas is ready — the deposit can be swept.";
    case "sweep_queued":
      return "Sweep is queued and will run when the queue processes it.";
    case "sweep_prepared":
      return "Sweep transaction prepared — waiting to be sent.";
    case "sweep_retrying":
      return "Sweep hit a retryable failure — the queue is retrying it.";
    case "sweep_submitted_unknown":
      return "Sweep was broadcast but its receipt is unknown — checking before any retry.";
    case "sweep_sent":
      return "Sweep broadcast — waiting for confirmation.";
    case "sweep_confirmed":
      return "Swept. Funds reached the destination.";
    case "sweep_blocked":
      return "Sweep is blocked — check the queue in Move for the reason.";
    case "sweep_failed":
      return "Sweep failed terminally — review the job in Move before retrying.";
    case "sweep_operator_action_required":
      return "Sweep needs a decision from you — open the queue in Move.";
    default:
      return "Status: " + String(deposit.status || "unknown").replace(/_/g, " ") + ".";
  }
}

/** The 2.4 gas story in human terms (ported from the legacy depositGasLine). */
export function depositGasNotes(deposit: EthStealthDeposit): string[] {
  const notes: string[] = [];
  if (deposit.requested_gas_wei_hex) {
    const amount = formatEthAmount(deposit.requested_gas_wei_hex, "ETH");
    notes.push("The payer was asked to attach " + (amount ?? "gas") + " for the sweep.");
  }
  if (deposit.gas_topup_job_id) {
    notes.push(
      "A sponsor gas top-up is " +
        String(deposit.gas_topup_job_state || "queued").replace(/_/g, " ") +
        ".",
    );
  }
  return notes;
}

/** Plain-English reason a watching one-time allocation has not swept yet. */
export function oneTimeBlockerText(blocker: string): string {
  switch (blocker) {
    case "awaiting_balance":
      return "waiting for a balance check";
    case "below_threshold":
      return "below the sweep threshold";
    case "execution_gates":
      return "waiting on execution gates";
    case "destination_policy":
      return "destination blocked by treasury policy";
    case "step_cap":
      return "above the per-step cap";
    case "cross_party_linkage":
      return "blocked: the shared destination would link payers";
    case "sweep_failed":
      return "last sweep failed";
    case "sweep_attention":
      return "sweep needs attention";
    default:
      return blocker.replace(/_/g, " ");
  }
}

/** One card in the address grid (allocations merged with overview balances). */
export interface AddressCardModel {
  key: string;
  address: string;
  sourceLabel: string;
  purpose: string | null;
  label: string | null;
  counterpartyName: string | null;
  balanceEth: string | null;
  balanceKnown: boolean;
  balanceCheckedAt: number | null;
  status: string;
  createdAt: number;
  linkageWarning: string | null;
  oneTime: boolean;
  lifecycle: string | null;
  blocker: string | null;
  sweepDestination: string | null;
  thresholdEth: string | null;
  purgeAfterSweep: boolean;
  allocationId: string | null;
  chainId: number | null;
  chainIdAssumed: boolean;
  derivationPath: string | null;
  addressIndex: number | null;
  walletProfile: string | null;
}

function balanceCheckedAt(
  item: ReceivingOverviewResponse["groups"][number]["items"][number],
): number | null {
  const value = item.balance_last_checked_at_unix;
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

function sourceTypeLabel(sourceType: string): string {
  if (sourceType === "hd") return "HD address";
  if (sourceType === "stealth") return "Stealth";
  return sourceType ? sourceType.replace(/_/g, " ") : "Address";
}

/** Merge overview items (balances, linkage) with allocations (one-time 3.3). */
export function buildAddressCards(
  overview: ReceivingOverviewResponse | null,
  allocations: TreasuryReceiveAllocation[],
  parties: Counterparty[],
): AddressCardModel[] {
  const partyName = new Map(parties.map((party) => [party.id, party.name]));
  const cards = new Map<string, AddressCardModel>();

  for (const allocation of allocations) {
    const key = allocation.address.toLowerCase();
    cards.set(key, {
      key,
      address: allocation.address,
      sourceLabel: "Allocated",
      purpose: allocation.purpose || null,
      label: allocation.label ?? null,
      counterpartyName: allocation.counterparty_id
        ? partyName.get(allocation.counterparty_id) ?? null
        : null,
      balanceEth: null,
      balanceKnown: false,
      balanceCheckedAt: null,
      status: allocation.status,
      createdAt: allocation.created_at_unix,
      linkageWarning: null,
      oneTime: allocation.one_time === true,
      lifecycle: allocation.lifecycle_state ?? null,
      blocker: allocation.sweep_blocker ?? null,
      sweepDestination: allocation.sweep_destination_address ?? null,
      thresholdEth: allocation.min_sweep_amount_hex
        ? formatEthAmount(allocation.min_sweep_amount_hex, null)
        : null,
      purgeAfterSweep: allocation.purge_after_sweep === true,
      allocationId: allocation.id,
      chainId: allocation.chain_id ?? null,
      chainIdAssumed: allocation.chain_id_assumed === true,
      derivationPath: allocation.derivation_path || null,
      addressIndex: allocation.address_index ?? null,
      walletProfile: allocation.wallet_profile || null,
    });
  }

  for (const group of overview?.groups ?? []) {
    for (const item of group.items ?? []) {
      const key = item.address.toLowerCase();
      const existing = cards.get(key);
      const balanceEth = item.balance_known
        ? formatEthAmount(item.balance_native_wei_hex ?? "0x0", null)
        : null;
      const checkedAt = balanceCheckedAt(item);
      if (existing) {
        existing.balanceEth = balanceEth;
        existing.balanceKnown = item.balance_known;
        existing.balanceCheckedAt = checkedAt;
        existing.linkageWarning = item.linkage_warning ?? null;
        existing.sourceLabel =
          item.source_type === "hd" || item.source_type === "stealth"
            ? sourceTypeLabel(item.source_type)
            : existing.sourceLabel;
        if (!existing.counterpartyName && item.counterparty_id) {
          existing.counterpartyName = partyName.get(item.counterparty_id) ?? null;
        }
        continue;
      }
      cards.set(key, {
        key,
        address: item.address,
        sourceLabel: sourceTypeLabel(item.source_type),
        purpose: item.purpose ?? null,
        label: item.label ?? null,
        counterpartyName: item.counterparty_id
          ? partyName.get(item.counterparty_id) ?? null
          : group.counterparty?.name ?? null,
        balanceEth,
        balanceKnown: item.balance_known,
        balanceCheckedAt: checkedAt,
        status: item.status,
        createdAt: item.created_at_unix,
        linkageWarning: item.linkage_warning ?? null,
        oneTime: false,
        lifecycle: null,
        blocker: null,
        sweepDestination: null,
        thresholdEth: null,
        purgeAfterSweep: false,
        allocationId: null,
        chainId: item.chain_id ?? null,
        chainIdAssumed: item.chain_id_assumed === true,
        derivationPath: item.derivation_path ?? null,
        addressIndex: null,
        walletProfile: null,
      });
    }
  }

  return Array.from(cards.values()).sort((a, b) => {
    // Active surfaces first, then newest first.
    const aActive = a.status === "active" ? 0 : 1;
    const bActive = b.status === "active" ? 0 : 1;
    if (aActive !== bActive) return aActive - bActive;
    return b.createdAt - a.createdAt;
  });
}

// ── Deposit status filter vocabulary (humanized at render time) ──────

const DEPOSIT_FILTERS: { value: string; label: string }[] = [
  { value: "", label: "All states" },
  { value: "pending", label: "Waiting for payment" },
  { value: "underfunded", label: "Partially funded" },
  { value: "funded_needs_gas", label: "Funded, needs gas" },
  { value: "funded", label: "Ready to sweep" },
  { value: "sweep_queued", label: "Sweep queued" },
  { value: "sweep_sent", label: "Sweep sent" },
  { value: "sweep_confirmed", label: "Swept" },
  { value: "sweep_failed", label: "Sweep failed" },
  { value: "sweep_operator_action_required", label: "Needs a decision" },
];

const DEPOSITS_PAGE_SIZE = 10;

/** The legacy card ids this destination takes over while mounted. */
export const RECEIVING_HOST_ID = "receivingCard";
const REPLACED_SIBLING_IDS = ["receiveBookCard", "depositsCard"];

// ── Small DOM utilities ───────────────────────────────────────────────

function replaceChildren(node: HTMLElement, children: (Node | string)[]): void {
  for (const child of Array.from(node.childNodes)) {
    (child as ChildNode).remove();
  }
  for (const child of children) {
    node.appendChild(typeof child === "string" ? document.createTextNode(child) : child);
  }
}

function fieldWrapper(labelText: string, input: HTMLElement, hint?: string): HTMLElement {
  const children: HTMLElement[] = [
    el("span", { class: "recv-field-label", text: labelText }),
    input,
  ];
  if (hint) children.push(el("span", { class: "recv-field-hint", text: hint }));
  return el("label", { class: "recv-field" }, ...children);
}

function textInput(placeholder: string, ariaLabel: string): HTMLInputElement {
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = placeholder;
  input.setAttribute("aria-label", ariaLabel);
  return input;
}

/** <option> builder — `new Option()` does not exist in the fake-DOM harness. */
function makeOption(labelText: string, value: string): HTMLOptionElement {
  const option = document.createElement("option");
  option.value = value;
  option.textContent = labelText;
  return option;
}

/** Reset tracked form fields — `form.reset()` does not exist in the harness. */
function clearInputs(inputs: (HTMLInputElement | HTMLSelectElement)[]): void {
  for (const input of inputs) {
    input.value = "";
    if ((input as HTMLInputElement).type === "checkbox") {
      (input as HTMLInputElement).checked = false;
    }
  }
}

// ── Controller ────────────────────────────────────────────────────────

export function createReceivingDestination(runtime: CoreRuntime): DestinationController {
  // ── Per-mount state ──
  let overview: ReceivingOverviewResponse | null = null;
  let allocations: TreasuryReceiveAllocation[] = [];
  let parties: Counterparty[] = [];
  let deposits: EthStealthDeposit[] = [];
  let depositsPagination: PaginationInfo | null = null;
  let depositsOffset = 0;
  let depositsFilter = "";
  let stealthProfiles: { name: string; wallet?: string }[] = [];
  let firstLoadPending = true;
  let refreshingBalances = false;
  let overviewLoadSequence = 0;
  let allocationsLoadSequence = 0;
  let partiesLoadSequence = 0;
  let depositsLoadSequence = 0;
  let depositTagRevision = 0;
  let nextDepositTagGeneration = 0;
  const depositTagGenerationById = new Map<string, number>();
  const pendingDepositTags = new Map<
    string,
    { generation: number; previous: string; next: string }
  >();

  let host: HTMLElement | null = null;
  let root: HTMLElement | null = null;
  let stashedChildren: Node[] = [];
  const unsubscribers: Unsubscribe[] = [];

  // Element refs, rebuilt by buildDom() on every mount.
  const refs: Record<string, HTMLElement> = {};

  function ref(name: string): HTMLElement {
    return refs[name];
  }

  function resetState(): void {
    overview = null;
    allocations = [];
    parties = [];
    deposits = [];
    depositsPagination = null;
    depositsOffset = 0;
    depositsFilter = "";
    stealthProfiles = [];
    firstLoadPending = true;
    refreshingBalances = false;
    // Invalidate work started by a previous mount without reusing generations.
    overviewLoadSequence += 1;
    allocationsLoadSequence += 1;
    partiesLoadSequence += 1;
    depositsLoadSequence += 1;
    depositTagRevision += 1;
    depositTagGenerationById.clear();
    pendingDepositTags.clear();
  }

  // ── Feedback surfaces ──

  function showBanner(name: "stale" | "locked", message: string): void {
    const banner = ref(name === "stale" ? "staleBanner" : "lockBanner");
    const text = ref(name === "stale" ? "staleBannerText" : "lockBannerText");
    text.textContent = message;
    banner.classList.remove("hidden");
  }

  function hideBanner(name: "stale" | "locked"): void {
    ref(name === "stale" ? "staleBanner" : "lockBanner").classList.add("hidden");
  }

  /** Error rendering driven by the structured failure code (plan 4.4). */
  function reportFailure(error: unknown, context: string): void {
    const failure = apiFailure(error);
    if (failure?.code === "vault_locked") {
      showBanner(
        "locked",
        "The vault is locked. Unlock it to load receive addresses and deposits.",
      );
      return;
    }
    const reason = failure?.error ?? "the daemon is unreachable";
    showBanner(
      "stale",
      "Couldn't refresh " + context + " (" + reason + "). What you see may be stale.",
    );
  }

  function setSectionNote(name: string, message: string): void {
    const note = ref(name);
    note.textContent = message;
    note.classList.remove("hidden");
  }

  // ── Copy affordance (same contract as the legacy copyText) ──

  async function copyValue(value: string, label: string): Promise<void> {
    try {
      const nav = (
        globalThis as { navigator?: { clipboard?: { writeText(v: string): Promise<void> } } }
      ).navigator;
      const win = (globalThis as { window?: { isSecureContext?: boolean } }).window;
      if (nav?.clipboard && win?.isSecureContext !== false) {
        await nav.clipboard.writeText(value);
        setSectionNote("copyNote", label + " copied.");
        return;
      }
    } catch (_) {
      // fall through to the manual-copy dialog
    }
    await informDialog({
      title: "Copy " + label,
      body: "Clipboard access is unavailable here. Select the value below and copy it manually.",
      valueDisplay: value,
    });
  }

  // ── Section: address cards ──

  function renderAddressSection(): void {
    const cards = buildAddressCards(overview, allocations, parties);

    const coverage = ref("coverageLine");
    if (overview) {
      const known = overview.coverage?.addresses_with_known_balance ?? 0;
      const total = overview.coverage?.addresses_total ?? 0;
      coverage.textContent =
        String(known) +
        " of " +
        String(total) +
        (total === 1 ? " address has" : " addresses have") +
        " a saved balance. Overview generated " +
        formatTimestamp(overview.generated_at_unix) +
        ".";
    } else {
      coverage.textContent = "";
    }

    const grid = ref("addressGrid");
    const empty = ref("addressEmpty");
    if (!overview && !allocations.length && firstLoadPending) return; // skeletons stay
    if (!cards.length) {
      renderList(grid, [], (card: AddressCardModel) => card.key, renderAddressCard);
      replaceChildren(empty, [
        el(
          "div",
          { class: "section-empty" },
          el("p", { class: "section-empty-title", text: "No receive addresses yet" }),
          el("p", {
            class: "section-empty-body",
            text: "Allocate a dedicated address per payer and purpose so incoming payments stay unlinkable.",
          }),
          el("button", {
            class: "btn-primary",
            attrs: { type: "button" },
            text: "Allocate an address",
            on: {
              click: () => {
                ref("allocateWallet").focus();
              },
            },
          }),
        ),
      ]);
      empty.classList.remove("hidden");
      return;
    }

    empty.classList.add("hidden");
    renderList(grid, cards, (card) => card.key, renderAddressCard);
  }

  function addressCardSignature(card: AddressCardModel): string {
    return JSON.stringify([
      card.balanceEth,
      card.balanceKnown,
      card.balanceCheckedAt,
      card.status,
      card.lifecycle,
      card.blocker,
      card.counterpartyName,
      card.linkageWarning,
      card.purpose,
      card.label,
    ]);
  }

  function pill(
    labelText: string,
    tone: "good" | "warn" | "danger" | "info" | "neutral",
  ): HTMLElement {
    return el("span", { class: "pill pill-" + tone, text: labelText });
  }

  function renderAddressCard(card: AddressCardModel, existing: HTMLElement | null): HTMLElement {
    const signature = addressCardSignature(card);
    if (existing && existing.dataset.signature === signature) return existing;
    const fresh = buildAddressCardNode(card, signature);
    if (!existing) return fresh;
    // renderList keeps keyed nodes: patch in place and return `existing` —
    // returning a fresh node for a kept key never removes the old one.
    existing.dataset.signature = signature;
    replaceChildren(existing, Array.from(fresh.childNodes));
    return existing;
  }

  function buildAddressCardNode(card: AddressCardModel, signature: string): HTMLElement {
    const title = el(
      "div",
      { class: "recv-card-title" },
      el("span", { class: "mono recv-address", text: truncateAddress(card.address) }),
      el("button", {
        class: "btn-ghost btn-small",
        attrs: { type: "button", "aria-label": "Copy address " + truncateAddress(card.address) },
        text: "Copy",
        on: { click: () => void copyValue(card.address, "Receive address") },
      }),
    );

    const pills = el("div", { class: "recv-card-pills" });
    pills.appendChild(pill(card.sourceLabel, "info"));
    if (card.status === "active") pills.appendChild(pill("Active", "good"));
    else if (card.status) pills.appendChild(pill(card.status.replace(/_/g, " "), "neutral"));
    if (card.oneTime) {
      pills.appendChild(pill("One-time", "warn"));
      if (card.lifecycle) pills.appendChild(pill(card.lifecycle.replace(/_/g, " "), "neutral"));
    }

    const lines = el("div", { class: "recv-card-lines" });
    const who: string[] = [];
    if (card.purpose) who.push(card.purpose);
    if (card.label) who.push(card.label);
    if (card.counterpartyName) who.push("for " + card.counterpartyName);
    if (who.length) {
      lines.appendChild(el("p", { class: "recv-card-line", text: who.join(" · ") }));
    }

    const balanceLine = el("p", { class: "recv-card-line recv-balance" });
    if (card.balanceKnown && card.balanceEth !== null) {
      balanceLine.appendChild(el("span", { class: "nums", text: card.balanceEth + " ETH" }));
    } else {
      balanceLine.textContent = "Balance unknown — run Refresh balances.";
    }
    lines.appendChild(balanceLine);

    const balanceFreshness =
      card.balanceCheckedAt !== null
        ? (card.balanceKnown ? "Balance checked " : "Balance unavailable · checked ") +
          formatTimestamp(card.balanceCheckedAt)
        : card.balanceKnown
          ? "Balance check time unavailable"
          : "Balance unavailable · check time unavailable";
    lines.appendChild(
      el("p", {
        class: "recv-card-line recv-card-freshness",
        text: balanceFreshness + " · allocated " + formatTimestamp(card.createdAt),
      }),
    );

    if (card.oneTime) {
      const bits: string[] = [];
      if (card.sweepDestination) {
        bits.push("sweeps to " + truncateAddress(card.sweepDestination));
      }
      bits.push(
        card.thresholdEth ? "threshold " + card.thresholdEth + " ETH" : "sweeps any amount",
      );
      if (card.purgeAfterSweep) bits.push("record purged after the sweep");
      lines.appendChild(
        el("p", { class: "recv-card-line", text: "One-time: " + bits.join(" · ") + "." }),
      );
      if (card.blocker) {
        lines.appendChild(
          el("p", {
            class: "recv-card-line recv-card-blocker",
            text: "Not swept yet: " + oneTimeBlockerText(card.blocker) + ".",
          }),
        );
      }
    }

    if (card.linkageWarning) {
      lines.appendChild(
        el("p", {
          class: "recv-card-line recv-card-warning",
          text:
            card.linkageWarning +
            " Sigillum-generated gas top-ups are checked; manual gas funding, timing correlation, and downstream re-merging remain operator discipline.",
        }),
      );
    }

    // Raw values stay one click away (DESIGN.md rule 3).
    const details = el("details", { class: "recv-card-details" });
    details.appendChild(el("summary", { text: "Details" }));
    const rawLines: string[] = ["Address " + card.address];
    if (card.chainId !== null) {
      rawLines.push(
        "Chain " + String(card.chainId) + (card.chainIdAssumed ? " (assumed)" : ""),
      );
    }
    if (card.walletProfile) rawLines.push("Wallet profile " + card.walletProfile);
    if (card.derivationPath) {
      rawLines.push(
        "Path " +
          card.derivationPath +
          (card.addressIndex !== null ? " · index " + String(card.addressIndex) : ""),
      );
    }
    for (const line of rawLines) {
      details.appendChild(el("p", { class: "recv-card-line mono", text: line }));
    }

    const children: HTMLElement[] = [title, pills, lines, details];
    if (card.allocationId && card.status === "active") {
      children.push(
        el(
          "div",
          { class: "recv-card-actions" },
          el("button", {
            class: "btn-ghost btn-small",
            attrs: { type: "button" },
            text: "Rotate",
            on: { click: () => void rotateAllocation(card.allocationId as string) },
          }),
        ),
      );
    }

    return el(
      "article",
      { class: "recv-address-card", dataset: { signature }, attrs: { role: "listitem" } },
      ...children,
    );
  }

  // ── Section: stealth deposits (guided lifecycle) ──

  function renderDeposits(): void {
    const list = ref("depositList");
    const empty = ref("depositEmpty");
    const pageNote = ref("depositPageNote");
    const total = depositsPagination?.total ?? deposits.length;
    const page = Math.floor(depositsOffset / DEPOSITS_PAGE_SIZE) + 1;
    pageNote.textContent = depositsPagination
      ? "Page " + String(page) + " · " + String(total) + " deposits"
      : deposits.length
        ? String(deposits.length) + " deposits"
        : "";
    (ref("depositPrev") as HTMLButtonElement).disabled = depositsOffset <= 0;
    (ref("depositNext") as HTMLButtonElement).disabled = !(depositsPagination?.has_more ?? false);

    if (!deposits.length) {
      renderList(list, [], (deposit: EthStealthDeposit) => deposit.id, renderDepositCard);
      const emptyState =
        depositsFilter === ""
          ? el(
              "div",
              { class: "section-empty" },
              el("p", { class: "section-empty-title", text: "No tracked stealth deposits yet" }),
              el("p", {
                class: "section-empty-body",
                text: "Give a payer your meta-address or create a tracked deposit address; incoming payments show up here as a guided lifecycle.",
              }),
              el("button", {
                class: "btn-primary",
                attrs: { type: "button" },
                text: "Show payer instructions",
                on: {
                  click: () => {
                    ref("getPaidSection").scrollIntoView();
                    ref("metaWalletSelect").focus();
                  },
                },
              }),
            )
          : el(
              "div",
              { class: "section-empty" },
              el("p", { class: "section-empty-title", text: "No deposits in this state" }),
              el("p", {
                class: "section-empty-body",
                text: "Try a different filter — deposits move through announced, funded, gas-ready, and swept.",
              }),
            );
      replaceChildren(empty, [emptyState]);
      empty.classList.remove("hidden");
      return;
    }

    empty.classList.add("hidden");
    renderList(list, deposits, (deposit) => deposit.id, renderDepositCard);
  }

  function depositSignature(deposit: EthStealthDeposit): string {
    const pendingTag = pendingDepositTags.get(deposit.id);
    return JSON.stringify([
      deposit.status,
      deposit.observed_amount_hex,
      deposit.observed_native_balance_wei_hex,
      deposit.expected_amount_hex,
      deposit.queue_job_state,
      deposit.gas_topup_job_state,
      deposit.counterparty_id,
      deposit.note,
      deposit.updated_at_unix,
      parties.map((party) => [party.id, party.name]),
      pendingTag ? [pendingTag.generation, pendingTag.next] : null,
    ]);
  }

  function renderDepositCard(deposit: EthStealthDeposit, existing: HTMLElement | null): HTMLElement {
    const signature = depositSignature(deposit);
    if (existing && existing.dataset.signature === signature) return existing;
    const fresh = buildDepositCardNode(deposit, signature);
    if (!existing) return fresh;
    // renderList keeps keyed nodes: patch in place and return `existing`.
    existing.dataset.signature = signature;
    existing.dataset.tier = fresh.dataset.tier;
    replaceChildren(existing, Array.from(fresh.childNodes));
    return existing;
  }

  function buildDepositCardNode(deposit: EthStealthDeposit, signature: string): HTMLElement {
    const lifecycle = depositLifecycle(deposit);
    const isNative = deposit.asset_kind === "native";

    // Lifecycle stepper: announced → funded → gas-ready → swept.
    const stepper = el("ol", { class: "recv-lifecycle" });
    DEPOSIT_STAGES.forEach((stage, index) => {
      const stepNumber = index + 1;
      const state =
        stepNumber <= lifecycle.completed
          ? "done"
          : stepNumber === lifecycle.completed + 1
            ? lifecycle.attention
              ? "attention"
              : "current"
            : "todo";
      stepper.appendChild(
        el("li", { dataset: { state } }, el("span", { class: "recv-lifecycle-label", text: stage })),
      );
    });

    const titleRow = el(
      "div",
      { class: "recv-card-title" },
      el("span", { class: "mono recv-address", text: truncateAddress(deposit.stealth_address) }),
      el("button", {
        class: "btn-ghost btn-small",
        attrs: {
          type: "button",
          "aria-label": "Copy deposit address " + truncateAddress(deposit.stealth_address),
        },
        text: "Copy",
        on: { click: () => void copyValue(deposit.stealth_address, "Deposit address") },
      }),
      pill(isNative ? "Native" : "ERC-20", "info"),
    );

    const lines = el("div", { class: "recv-card-lines" });
    lines.appendChild(el("p", { class: "recv-card-line", text: depositStatusLine(deposit) }));

    // Amounts: native humanizes to ETH; ERC-20 stays raw behind details
    // (no token-registry decimals loaded here — guessing units would lie).
    const expected = isNative ? formatEthAmount(deposit.expected_amount_hex, "ETH") : null;
    const observed = isNative ? formatEthAmount(deposit.observed_amount_hex, "ETH") : null;
    const amountBits: string[] = [];
    if (observed) amountBits.push("received " + observed);
    if (expected) amountBits.push("expected " + expected);
    const nativeBalance = formatEthAmount(deposit.observed_native_balance_wei_hex, "ETH");
    const hasNativeGas =
      nativeBalance !== null && BigInt(deposit.observed_native_balance_wei_hex as string) > 0n;
    if (!isNative && hasNativeGas) amountBits.push("native gas on address " + nativeBalance);
    if (amountBits.length) {
      lines.appendChild(el("p", { class: "recv-card-line nums", text: amountBits.join(" · ") }));
    }
    if (!isNative && (deposit.expected_amount_hex || deposit.observed_amount_hex)) {
      const raw = el("details", { class: "recv-card-details" });
      raw.appendChild(el("summary", { text: "Raw token amounts (base units)" }));
      if (deposit.expected_amount_hex) {
        raw.appendChild(
          el("p", { class: "recv-card-line mono", text: "expected " + deposit.expected_amount_hex }),
        );
      }
      if (deposit.observed_amount_hex) {
        raw.appendChild(
          el("p", { class: "recv-card-line mono", text: "observed " + deposit.observed_amount_hex }),
        );
      }
      lines.appendChild(raw);
    }

    for (const note of depositGasNotes(deposit)) {
      lines.appendChild(el("p", { class: "recv-card-line", text: note }));
    }
    if (deposit.queue_job_id) {
      lines.appendChild(
        el("p", {
          class: "recv-card-line",
          text:
            "Sweep job " +
            String(deposit.queue_job_state || "queued").replace(/_/g, " ") +
            " — track it in Move.",
        }),
      );
    }
    if (deposit.note) {
      lines.appendChild(el("p", { class: "recv-card-line", text: deposit.note }));
    }
    lines.appendChild(
      el("p", {
        class: "recv-card-line recv-card-freshness",
        text:
          "Checked " +
          formatTimestamp(deposit.last_checked_at_unix) +
          " · updated " +
          formatTimestamp(deposit.updated_at_unix),
      }),
    );

    // Counterparty tagging (plan §4.3.3c).
    const tagSelect = document.createElement("select");
    tagSelect.setAttribute(
      "aria-label",
      "Counterparty for deposit " + truncateAddress(deposit.stealth_address),
    );
    tagSelect.appendChild(makeOption("No counterparty", ""));
    for (const party of parties) {
      tagSelect.appendChild(makeOption(party.name, party.id));
    }
    const selectedPartyId = deposit.counterparty_id ?? "";
    if (selectedPartyId && !parties.some((party) => party.id === selectedPartyId)) {
      // Keep the persisted-but-dangling value explicit and selectable. If we
      // silently display the empty option, choosing "No counterparty" does
      // not fire a change and the operator cannot actually clear the tag.
      tagSelect.appendChild(
        makeOption(
          "Deleted or unavailable counterparty — clear or retag",
          selectedPartyId,
        ),
      );
    }
    tagSelect.value = selectedPartyId;
    const pendingTag = pendingDepositTags.get(deposit.id);
    if (pendingTag) {
      tagSelect.disabled = true;
      tagSelect.setAttribute("aria-busy", "true");
    }
    tagSelect.addEventListener("change", () =>
      void tagDeposit(deposit.id, tagSelect.value),
    );
    const tagRow = el(
      "div",
      { class: "recv-tag-row" },
      el("span", { class: "recv-field-label", text: "Paid by" }),
      tagSelect,
    );

    // Payer-facing announcement details (what the payer attaches).
    let announcementDetails: HTMLElement | null = null;
    if (deposit.announcement) {
      const announcement = deposit.announcement;
      announcementDetails = el("details", { class: "recv-card-details" });
      announcementDetails.appendChild(el("summary", { text: "Payer announcement data" }));
      announcementDetails.appendChild(
        el("p", {
          class: "recv-card-line",
          text: "The payer announces the payment to the ERC-5564 announcer with this calldata.",
        }),
      );
      announcementDetails.appendChild(
        el(
          "div",
          { class: "recv-copy-row" },
          el("span", {
            class: "mono recv-card-line",
            text: truncateAddress(announcement.announcer_address),
          }),
          el("button", {
            class: "btn-ghost btn-small",
            attrs: { type: "button" },
            text: "Copy announcer",
            on: {
              click: () => void copyValue(announcement.announcer_address, "ERC-5564 announcer"),
            },
          }),
        ),
      );
      announcementDetails.appendChild(
        el(
          "div",
          { class: "recv-copy-row" },
          el("button", {
            class: "btn-ghost btn-small",
            attrs: { type: "button" },
            text: "Copy announce calldata",
            on: { click: () => void copyValue(announcement.calldata_hex, "Announce calldata") },
          }),
        ),
      );
    }

    const actions = el("div", { class: "recv-card-actions" });
    actions.appendChild(
      el("button", {
        class: "btn-ghost btn-small",
        attrs: { type: "button" },
        text: "Refresh",
        on: {
          click: (event) =>
            void refreshSingleDeposit(deposit.id, event.target as HTMLButtonElement),
        },
      }),
    );
    if (deposit.status !== "sweep_confirmed" && !deposit.queue_job_id) {
      actions.appendChild(
        el("button", {
          class: "btn-primary btn-small",
          attrs: { type: "button" },
          text: "Queue sweep",
          on: {
            click: (event) => void enqueueSweep(deposit.id, event.target as HTMLButtonElement),
          },
        }),
      );
    }
    actions.appendChild(
      el("button", {
        class: "btn-danger btn-small",
        attrs: { type: "button" },
        text: "Delete",
        on: { click: () => void deleteDeposit(deposit.id) },
      }),
    );

    const children: (HTMLElement | null)[] = [
      titleRow,
      stepper,
      lines,
      tagRow,
      announcementDetails,
      actions,
    ];
    return el(
      "article",
      {
        class: "recv-deposit-card",
        dataset: { signature, tier: lifecycle.attention ? "review" : "quiet" },
        attrs: { role: "listitem" },
      },
      ...(children.filter(Boolean) as HTMLElement[]),
    );
  }

  // ── Section: counterparties ──

  function renderParties(): void {
    const list = ref("partyList");
    const empty = ref("partyEmpty");
    if (!parties.length) {
      renderList(list, [], (party: Counterparty) => party.id, renderPartyRow);
      replaceChildren(empty, [
        el(
          "div",
          { class: "section-empty" },
          el("p", { class: "section-empty-title", text: "No counterparties yet" }),
          el("p", {
            class: "section-empty-body",
            text: "Track payers as parties so each one gets a dedicated receive address and deposits stay attributed.",
          }),
        ),
      ]);
      empty.classList.remove("hidden");
    } else {
      empty.classList.add("hidden");
      renderList(list, parties, (party) => party.id, renderPartyRow);
    }

    // Keep the allocate-form party select in sync.
    const select = ref("allocateParty") as HTMLSelectElement;
    const previous = select.value;
    replaceChildren(select, [makeOption("No counterparty (optional)", "")]);
    for (const party of parties) {
      select.appendChild(makeOption(party.name, party.id));
    }
    select.value = previous && parties.some((party) => party.id === previous) ? previous : "";
  }

  function renderPartyRow(party: Counterparty, existing: HTMLElement | null): HTMLElement {
    const signature = JSON.stringify([party.name, party.note, party.sweep_destination_address]);
    if (existing && existing.dataset.signature === signature) return existing;
    const fresh = buildPartyRowNode(party, signature);
    if (!existing) return fresh;
    // renderList keeps keyed nodes: patch in place and return `existing`.
    existing.dataset.signature = signature;
    replaceChildren(existing, Array.from(fresh.childNodes));
    return existing;
  }

  function buildPartyRowNode(party: Counterparty, signature: string): HTMLElement {
    const lines: HTMLElement[] = [el("span", { class: "recv-party-name", text: party.name })];
    if (party.note) lines.push(el("span", { class: "recv-card-line", text: party.note }));
    if (party.sweep_destination_address) {
      lines.push(
        el("span", {
          class: "recv-card-line",
          text: "sweeps to " + truncateAddress(party.sweep_destination_address),
        }),
      );
    }
    const destinationInput = textInput(
      "0x sweep destination",
      "Sweep destination for " + party.name,
    );
    destinationInput.className = "mono input-wider";
    destinationInput.value = party.sweep_destination_address ?? "";
    const actions = el(
      "div",
      { class: "recv-card-actions" },
      destinationInput,
      el("button", {
        class: "btn-ghost btn-small",
        attrs: { type: "button" },
        text: "Save",
        on: {
          click: (event) =>
            void updatePartySweepDestination(
              party.id,
              destinationInput,
              "save",
              event.target as HTMLButtonElement,
            ),
        },
      }),
      el("button", {
        class: "btn-ghost btn-small",
        attrs: { type: "button" },
        text: "Clear",
        on: {
          click: (event) =>
            void updatePartySweepDestination(
              party.id,
              destinationInput,
              "clear",
              event.target as HTMLButtonElement,
            ),
        },
      }),
      el("button", {
        class: "btn-danger btn-small",
        attrs: { type: "button" },
        text: "Delete",
        on: {
          click: (event) =>
            void deleteParty(party.id, event.target as HTMLButtonElement),
        },
      }),
    );
    return el(
      "div",
      { class: "recv-party-row", dataset: { signature }, attrs: { role: "listitem" } },
      ...lines,
      actions,
    );
  }

  // ── Section: skeletons & loading ──

  function renderSkeletons(): void {
    const skeletonCard = () =>
      el(
        "div",
        { class: "recv-address-card" },
        el("div", { class: "skeleton skeleton-text short" }),
        el("div", { class: "skeleton skeleton-text" }),
        el("div", { class: "skeleton skeleton-block" }),
      );
    replaceChildren(ref("addressEmpty"), [skeletonCard(), skeletonCard(), skeletonCard()]);
    ref("addressEmpty").classList.remove("hidden");
    replaceChildren(ref("depositEmpty"), [skeletonCard(), skeletonCard()]);
    ref("depositEmpty").classList.remove("hidden");
  }

  // ── Data loading ──

  async function loadOverview(): Promise<void> {
    const requestSequence = ++overviewLoadSequence;
    try {
      const next = await runtime.api.getReceivingOverview();
      if (requestSequence !== overviewLoadSequence) return;
      overview = next;
      renderAddressSection();
    } catch (error) {
      if (requestSequence !== overviewLoadSequence) return;
      throw error;
    }
  }

  async function loadAllocations(): Promise<void> {
    const requestSequence = ++allocationsLoadSequence;
    try {
      const r = await request<AllocationListResponse>("GET", "/api/treasury/receive-addresses");
      if (requestSequence !== allocationsLoadSequence) return;
      allocations = r.allocations ?? [];
      renderAddressSection();
    } catch (error) {
      if (requestSequence !== allocationsLoadSequence) return;
      throw error;
    }
  }

  async function loadParties(): Promise<void> {
    const requestSequence = ++partiesLoadSequence;
    try {
      const r = await request<PartyListResponse>("GET", "/api/treasury/parties");
      if (requestSequence !== partiesLoadSequence) return;
      parties = r.parties ?? [];
      renderParties();
      renderAddressSection(); // party names ride on the cards
      renderDeposits(); // party options ride on every deposit card
    } catch (error) {
      if (requestSequence !== partiesLoadSequence) return;
      throw error;
    }
  }

  async function loadDeposits(): Promise<void> {
    const requestSequence = ++depositsLoadSequence;
    const tagRevisionAtStart = depositTagRevision;
    const r = await runtime.api.listDeposits({
      limit: DEPOSITS_PAGE_SIZE,
      offset: depositsOffset,
      sort: "created",
      order: "desc",
      ...(depositsFilter
        ? { status: depositsFilter as EthStealthDepositListQuery["status"] }
        : {}),
    });
    // A list read begun before a tag mutation must never overwrite the
    // confirmed or rolled-back result. A newer list request also wins.
    if (
      requestSequence !== depositsLoadSequence ||
      tagRevisionAtStart !== depositTagRevision
    ) {
      return;
    }
    deposits = (r.deposits ?? []).map((deposit) => {
      const pending = pendingDepositTags.get(deposit.id);
      return pending
        ? { ...deposit, counterparty_id: pending.next || null }
        : deposit;
    });
    depositsPagination = r.pagination ?? null;
    renderDeposits();
  }

  async function reconcileReceivingMutation(context: string): Promise<void> {
    const results = await Promise.allSettled([
      loadParties(),
      loadAllocations(),
      loadOverview(),
      loadDeposits(),
    ]);
    const failure = results
      .map((result) => (result.status === "rejected" ? result.reason : null))
      .find((reason) => reason != null);
    if (failure) {
      reportFailure(failure, context);
      return;
    }
    hideBanner("stale");
    hideBanner("locked");
  }

  async function loadStealthProfiles(): Promise<void> {
    const r = await request<StealthProfileListResponse>("GET", "/api/profiles/eth-stealth");
    stealthProfiles = (r.profiles ?? []).filter(
      (profile): profile is { name: string; wallet?: string } => typeof profile.name === "string",
    );
    fillWalletSelects();
  }

  function fillWalletSelects(): void {
    for (const name of [
      "metaWalletSelect",
      "requestWalletSelect",
      "erc20WalletSelect",
      "scanWalletSelect",
    ]) {
      const select = ref(name) as HTMLSelectElement;
      const previous = select.value;
      replaceChildren(select, []);
      if (!stealthProfiles.length) {
        select.appendChild(makeOption("No stealth wallets yet — create one in Portfolio", ""));
      } else {
        for (const profile of stealthProfiles) {
          const label = profile.wallet
            ? profile.name + " · " + truncateAddress(profile.wallet)
            : profile.name;
          select.appendChild(makeOption(label, profile.name));
        }
      }
      select.value =
        previous && stealthProfiles.some((profile) => profile.name === previous)
          ? previous
          : stealthProfiles[0]?.name ?? "";
    }
  }

  async function loadAll(initial: boolean): Promise<void> {
    if (initial) renderSkeletons();
    const results = await Promise.allSettled([
      loadOverview(),
      loadAllocations(),
      loadParties(),
      loadDeposits(),
      loadStealthProfiles(),
    ]);
    firstLoadPending = false;
    const failure = results
      .map((result) => (result.status === "rejected" ? result.reason : null))
      .find((reason) => reason != null);
    if (failure) {
      reportFailure(failure, "receiving data");
      // Sections that never loaded still settle into their empty states.
      renderAddressSection();
      renderDeposits();
      renderParties();
    } else {
      hideBanner("stale");
      hideBanner("locked");
    }
  }

  // ── Actions ──

  function busyButton(button: HTMLButtonElement, busy: boolean, busyLabel?: string): void {
    if (busy) {
      button.dataset.idleLabel = button.textContent ?? "";
      if (busyLabel) button.textContent = busyLabel;
      button.disabled = true;
      button.setAttribute("aria-busy", "true");
    } else {
      if (button.dataset.idleLabel) button.textContent = button.dataset.idleLabel;
      button.disabled = false;
      button.removeAttribute("aria-busy");
    }
  }

  async function refreshBalances(button: HTMLButtonElement): Promise<void> {
    if (refreshingBalances) return;
    refreshingBalances = true;
    busyButton(button, true, "Refreshing…");
    setSectionNote(
      "refreshNote",
      "Querying your provider for each address — this can take a moment.",
    );
    try {
      const r = await request<ReceivingRefreshResponse>("POST", "/api/receiving/refresh-balances");
      if (r.provider_status === "no_provider") {
        setSectionNote("refreshNote", "Configure an RPC provider before refreshing balances.");
      } else if (r.provider_status === "partial") {
        const first = r.errors && r.errors.length ? " First error: " + r.errors[0] : "";
        setSectionNote(
          "refreshNote",
          "Some addresses failed to refresh." + first + " Showing what succeeded.",
        );
      } else {
        setSectionNote(
          "refreshNote",
          "Refreshed " +
            String(r.addresses_refreshed ?? 0) +
            " addresses" +
            (r.addresses_skipped ? " (" + String(r.addresses_skipped) + " skipped by the cap)" : "") +
            ".",
        );
      }
      await loadAll(false);
    } catch (error) {
      reportFailure(error, "balances");
      setSectionNote("refreshNote", "Balance refresh failed — showing the last saved scan.");
    } finally {
      refreshingBalances = false;
      busyButton(button, false);
    }
  }

  function markFieldInvalid(input: HTMLElement, invalid: boolean): void {
    input.classList.toggle("input-invalid", invalid);
    if (invalid) input.setAttribute("aria-invalid", "true");
    else input.removeAttribute("aria-invalid");
  }

  /** Apply a validation_failed failure to a form's fields by name. */
  function applyFieldErrors(
    error: unknown,
    inputsByField: Record<string, HTMLElement>,
  ): boolean {
    const failure = apiFailure(error);
    if (failure?.code !== "validation_failed" || !failure.fields?.length) return false;
    for (const field of failure.fields) {
      const input = inputsByField[field.field];
      if (input) markFieldInvalid(input, true);
    }
    return true;
  }

  function clearFieldErrors(inputs: HTMLElement[]): void {
    for (const input of inputs) markFieldInvalid(input, false);
  }

  async function submitAllocate(): Promise<void> {
    const wallet = ref("allocateWallet") as HTMLInputElement;
    const purpose = ref("allocatePurpose") as HTMLInputElement;
    const labelInput = ref("allocateLabel") as HTMLInputElement;
    const party = ref("allocateParty") as HTMLSelectElement;
    const oneTime = ref("allocateOneTime") as HTMLInputElement;
    const destination = ref("allocateDestination") as HTMLInputElement;
    const threshold = ref("allocateThreshold") as HTMLInputElement;
    const purge = ref("allocatePurge") as HTMLInputElement;
    clearFieldErrors([wallet, purpose, destination, threshold]);

    const body: Record<string, unknown> = {
      wallet_profile: wallet.value.trim(),
      purpose: purpose.value.trim(),
    };
    if (!body.wallet_profile || !body.purpose) {
      markFieldInvalid(wallet, !body.wallet_profile);
      markFieldInvalid(purpose, !body.purpose);
      setSectionNote("allocateNote", "Wallet profile and purpose are required.");
      return;
    }
    if (labelInput.value.trim()) body.label = labelInput.value.trim();
    if (party.value) body.counterparty_id = party.value;
    if (oneTime.checked) {
      const destinationValue = destination.value.trim();
      if (!destinationValue) {
        markFieldInvalid(destination, true);
        setSectionNote(
          "allocateNote",
          "A one-time address needs a sweep destination — where should the funds go?",
        );
        return;
      }
      body.one_time = true;
      body.sweep_destination_address = destinationValue;
      const thresholdValue = threshold.value.trim();
      if (thresholdValue) {
        const weiHex = parseEthToWeiHex(thresholdValue);
        if (!weiHex) {
          markFieldInvalid(threshold, true);
          setSectionNote("allocateNote", "The sweep threshold must be an ETH amount like 0.05.");
          return;
        }
        body.min_sweep_amount_hex = weiHex;
      }
      body.purge_after_sweep = purge.checked;
    }

    const submit = ref("allocateSubmit") as HTMLButtonElement;
    busyButton(submit, true, "Allocating…");
    try {
      const r = await request<{ allocation?: TreasuryReceiveAllocation }>(
        "POST",
        "/api/treasury/receive-addresses/allocate",
        body,
      );
      clearInputs([wallet, purpose, labelInput, party, destination, threshold, oneTime, purge]);
      toggleOneTimeOptions(false);
      setSectionNote(
        "allocateNote",
        "Address allocated" +
          (r.allocation?.address ? ": " + truncateAddress(r.allocation.address) : "") +
          ".",
      );
      if (r.allocation?.address) void copyValue(r.allocation.address, "Receive address");
      await Promise.all([loadAllocations(), loadOverview()]);
    } catch (error) {
      if (
        applyFieldErrors(error, {
          wallet_profile: wallet,
          purpose,
          label: labelInput,
          sweep_destination_address: destination,
          min_sweep_amount_hex: threshold,
        })
      ) {
        setSectionNote("allocateNote", apiFailure(error)?.error ?? "Check the highlighted fields.");
      } else {
        reportFailure(error, "allocation");
        setSectionNote("allocateNote", apiFailure(error)?.error ?? "Allocation failed.");
      }
    } finally {
      busyButton(submit, false);
    }
  }

  function toggleOneTimeOptions(show: boolean): void {
    ref("oneTimeOptions").classList.toggle("hidden", !show);
  }

  async function rotateAllocation(allocationId: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Rotate receive address",
      body: "Rotate this receive address? The current address is retired and a fresh one is derived for future payments. The old address stays valid on-chain but no longer shows as the active receive address.",
      actionLabel: "Rotate address",
    });
    if (!confirmed) return;
    try {
      await request("POST", "/api/treasury/receive-addresses/rotate", {
        allocation_id: allocationId,
      });
      setSectionNote("refreshNote", "Receive address rotated.");
      await Promise.all([loadAllocations(), loadOverview()]);
    } catch (error) {
      reportFailure(error, "rotation");
    }
  }

  async function enqueueSweep(depositId: string, button: HTMLButtonElement): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Queue deposit sweep",
      body: "Enqueue a sweep job for this deposit? When the queue processes it and the job passes its checks, the sweep is signed and broadcast on-chain.",
      actionLabel: "Queue sweep",
    });
    if (!confirmed) return;
    busyButton(button, true, "Queueing…");
    try {
      const r = await request<EnqueueSweepResult>(
        "POST",
        "/api/deposits/eth-stealth/enqueue-sweep",
        { id: depositId },
      );
      setSectionNote("depositNote", "Sweep queued" + (r.job?.id ? " — track it in Move." : "."));
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the sweep");
    } finally {
      busyButton(button, false);
    }
  }

  async function deleteDeposit(depositId: string): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Delete deposit",
      body: "Delete this deposit record? It is removed from this daemon; funds already on-chain are not moved.",
      actionLabel: "Delete",
    });
    if (!confirmed) return;
    try {
      await request("POST", "/api/deposits/eth-stealth/delete", { id: depositId });
      setSectionNote("depositNote", "Deposit deleted.");
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the deposit");
    }
  }

  async function refreshSingleDeposit(depositId: string, button: HTMLButtonElement): Promise<void> {
    busyButton(button, true, "Refreshing…");
    try {
      const r = await request<DepositRefreshResult>("POST", "/api/deposits/eth-stealth/refresh", {
        id: depositId,
        limit: 1,
        auto_enqueue: false,
      });
      setSectionNote(
        "depositNote",
        "Checked 1 deposit — " + String(r.detected ?? 0) + " payments detected.",
      );
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the deposit");
    } finally {
      busyButton(button, false);
    }
  }

  async function refreshAllDeposits(button: HTMLButtonElement): Promise<void> {
    busyButton(button, true, "Refreshing…");
    try {
      const r = await request<DepositRefreshResult>("POST", "/api/deposits/eth-stealth/refresh", {
        id: null,
        limit: 50,
        auto_enqueue: false,
      });
      setSectionNote(
        "depositNote",
        "Checked " +
          String(r.processed ?? 0) +
          " deposits — " +
          String(r.detected ?? 0) +
          " payments detected.",
      );
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "deposits");
    } finally {
      busyButton(button, false);
    }
  }

  async function tagDeposit(depositId: string, counterpartyId: string): Promise<void> {
    const current = deposits.find((deposit) => deposit.id === depositId);
    if (!current) return;
    const previous = current.counterparty_id ?? "";
    if (previous === counterpartyId) return;

    const generation = ++nextDepositTagGeneration;
    depositTagGenerationById.set(depositId, generation);
    pendingDepositTags.set(depositId, {
      generation,
      previous,
      next: counterpartyId,
    });
    depositTagRevision += 1;
    deposits = deposits.map((deposit) =>
      deposit.id === depositId
        ? { ...deposit, counterparty_id: counterpartyId || null }
        : deposit,
    );
    renderDeposits();

    let response: DepositMutationResponse;
    try {
      response = await request<DepositMutationResponse>(
        "POST",
        "/api/receiving/deposits/tag",
        {
          deposit_id: depositId,
          counterparty_id: counterpartyId || null,
        },
      );
    } catch (error) {
      if (depositTagGenerationById.get(depositId) !== generation) return;
      pendingDepositTags.delete(depositId);
      depositTagRevision += 1;
      deposits = deposits.map((deposit) =>
        deposit.id === depositId
          ? { ...deposit, counterparty_id: previous || null }
          : deposit,
      );
      renderDeposits();
      reportFailure(error, "the counterparty tag");
      setSectionNote(
        "depositNote",
        "Counterparty update failed. The previous selection was restored.",
      );
      return;
    }

    if (depositTagGenerationById.get(depositId) !== generation) return;
    pendingDepositTags.delete(depositId);
    depositTagRevision += 1;
    deposits = deposits.map((deposit) =>
      deposit.id === depositId
        ? response.deposit ?? { ...deposit, counterparty_id: counterpartyId || null }
        : deposit,
    );
    renderDeposits();
    setSectionNote("depositNote", "Counterparty updated.");

    // The mutation is already committed. A reconciliation failure must leave
    // that confirmed state visible rather than looking like a failed write.
    try {
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the updated counterparty tag");
      setSectionNote(
        "depositNote",
        "Counterparty updated, but the latest list could not be refreshed. Showing the confirmed update.",
      );
    }
  }

  async function updatePartySweepDestination(
    partyId: string,
    input: HTMLInputElement,
    action: "save" | "clear",
    button: HTMLButtonElement,
  ): Promise<void> {
    const party = parties.find((candidate) => candidate.id === partyId);
    if (!party) return;
    const destination = action === "clear" ? "" : input.value.trim();
    clearFieldErrors([input]);
    if (action === "save" && !destination) {
      markFieldInvalid(input, true);
      setSectionNote(
        "partyNoteLine",
        "Enter a sweep destination to save, or use Clear to remove it.",
      );
      return;
    }

    busyButton(button, true, action === "clear" ? "Clearing…" : "Saving…");
    try {
      const r = await request<PartyMutationResponse>(
        "POST",
        "/api/treasury/parties/update",
        {
          id: party.id,
          name: party.name,
          note: party.note ?? null,
          sweep_destination_address: destination,
        },
      );
      const updated =
        r.party ?? {
          ...party,
          sweep_destination_address: destination || null,
        };
      parties = parties.map((candidate) =>
        candidate.id === party.id ? updated : candidate,
      );
      renderParties();
      renderAddressSection();
      renderDeposits();
      setSectionNote(
        "partyNoteLine",
        destination ? "Sweep destination saved." : "Sweep destination cleared.",
      );
      await reconcileReceivingMutation("receiving data after the counterparty update");
    } catch (error) {
      if (applyFieldErrors(error, { sweep_destination_address: input })) {
        setSectionNote(
          "partyNoteLine",
          apiFailure(error)?.error ?? "Check the highlighted destination.",
        );
      } else {
        reportFailure(error, "the counterparty update");
      }
    } finally {
      busyButton(button, false);
    }
  }

  async function deleteParty(partyId: string, button: HTMLButtonElement): Promise<void> {
    const party = parties.find((candidate) => candidate.id === partyId);
    if (!party) return;
    const confirmed = await confirmDangerDialog({
      title: "Delete counterparty",
      body:
        'Delete counterparty "' +
        party.name +
        '"? Existing receive allocations remain but are unbound. Existing stealth deposit records may retain this counterparty ID and stay explicitly marked deleted or unavailable until you retag them.',
      actionLabel: "Delete counterparty",
    });
    if (!confirmed) return;

    busyButton(button, true, "Deleting…");
    try {
      await request("POST", "/api/treasury/parties/delete", { id: party.id });
      parties = parties.filter((candidate) => candidate.id !== party.id);
      allocations = allocations.map((allocation) =>
        allocation.counterparty_id === party.id
          ? { ...allocation, counterparty_id: null }
          : allocation,
      );
      if (overview) {
        overview = {
          ...overview,
          groups: overview.groups.map((group) =>
            group.counterparty?.id === party.id
              ? { ...group, counterparty: null }
              : group,
          ),
        };
      }
      renderParties();
      renderAddressSection();
      renderDeposits();
      setSectionNote(
        "partyNoteLine",
        "Counterparty deleted. Receive allocations were unbound; retained deposit tags are marked deleted or unavailable until retagged.",
      );
      await reconcileReceivingMutation("receiving data after the counterparty deletion");
    } catch (error) {
      reportFailure(error, "the counterparty deletion");
    } finally {
      busyButton(button, false);
    }
  }

  async function submitParty(): Promise<void> {
    const name = ref("partyName") as HTMLInputElement;
    const note = ref("partyNote") as HTMLInputElement;
    const destination = ref("partyDestination") as HTMLInputElement;
    clearFieldErrors([name, destination]);
    if (!name.value.trim()) {
      markFieldInvalid(name, true);
      setSectionNote("partyNoteLine", "A counterparty needs a name.");
      return;
    }
    const body: Record<string, unknown> = { name: name.value.trim() };
    if (note.value.trim()) body.note = note.value.trim();
    if (destination.value.trim()) body.sweep_destination_address = destination.value.trim();
    try {
      const r = await request<PartyMutationResponse>("POST", "/api/treasury/parties", body);
      clearInputs([name, note, destination]);
      const created = r.party;
      if (created) {
        parties = [...parties.filter((party) => party.id !== created.id), created];
        renderParties();
        renderAddressSection();
        renderDeposits();
      }
      setSectionNote("partyNoteLine", "Counterparty added.");
      await reconcileReceivingMutation("receiving data after adding the counterparty");
    } catch (error) {
      if (applyFieldErrors(error, { name, sweep_destination_address: destination })) {
        setSectionNote(
          "partyNoteLine",
          apiFailure(error)?.error ?? "Check the highlighted fields.",
        );
      } else {
        reportFailure(error, "the counterparty");
      }
    }
  }

  async function submitMetaAddress(): Promise<void> {
    const select = ref("metaWalletSelect") as HTMLSelectElement;
    if (!select.value) {
      setSectionNote("metaNote", "Create a stealth wallet in Portfolio first.");
      return;
    }
    try {
      const r = await request<MetaAddressExportResult>("POST", "/api/wallets/eth-stealth/export", {
        wallet: select.value,
        short_name: null,
      });
      if (!r.stealth_meta_address) {
        setSectionNote("metaNote", "The daemon returned no meta-address for this wallet.");
        return;
      }
      ref("metaAddressValue").textContent = r.stealth_meta_address;
      ref("metaSchemeLine").textContent =
        "If the payer's wallet asks for a scheme, it is scheme " +
        String(r.scheme_id ?? 1) +
        " (secp256k1 with view tags) — the ERC-5564 default.";
      ref("metaResult").classList.remove("hidden");
      setSectionNote("metaNote", "Meta-address ready — hand it to the payer.");
    } catch (error) {
      reportFailure(error, "the meta-address");
    }
  }

  async function submitRequestPayment(): Promise<void> {
    const select = ref("requestWalletSelect") as HTMLSelectElement;
    const expected = ref("requestExpected") as HTMLInputElement;
    const note = ref("requestNote") as HTMLInputElement;
    const requestGas = ref("requestGas") as HTMLInputElement;
    const gasAmount = ref("requestGasAmount") as HTMLInputElement;
    clearFieldErrors([expected, gasAmount]);
    if (!select.value) {
      setSectionNote("requestNoteLine", "Create a stealth wallet in Portfolio first.");
      return;
    }
    const body: Record<string, unknown> = {
      wallet_profile: select.value,
      auto_queue_sweep: true,
      request_gas: requestGas.checked,
    };
    if (expected.value.trim()) {
      const weiHex = parseEthToWeiHex(expected.value.trim());
      if (!weiHex) {
        markFieldInvalid(expected, true);
        setSectionNote("requestNoteLine", "The expected amount must be an ETH amount like 0.42.");
        return;
      }
      body.expected_value_wei_hex = weiHex;
    }
    if (note.value.trim()) body.note = note.value.trim();
    if (requestGas.checked && gasAmount.value.trim()) {
      const gasWeiHex = parseEthToWeiHex(gasAmount.value.trim());
      if (!gasWeiHex) {
        markFieldInvalid(gasAmount, true);
        setSectionNote("requestNoteLine", "The gas amount must be an ETH amount like 0.005.");
        return;
      }
      body.gas_amount_wei_hex = gasWeiHex;
    }

    try {
      const r = await request<CreateDepositResult>(
        "POST",
        "/api/deposits/eth-stealth/create-native",
        body,
      );
      clearInputs([expected, note, requestGas, gasAmount]);
      renderCreateResult(r);
      await loadDeposits();
    } catch (error) {
      if (
        applyFieldErrors(error, {
          expected_value_wei_hex: expected,
          gas_amount_wei_hex: gasAmount,
        })
      ) {
        setSectionNote(
          "requestNoteLine",
          apiFailure(error)?.error ?? "Check the highlighted fields.",
        );
      } else {
        reportFailure(error, "the deposit address");
      }
    }
  }

  function renderCreateResult(result: CreateDepositResult): void {
    const panel = ref("requestResult");
    const deposit = result.deposit;
    if (!deposit) {
      panel.classList.add("hidden");
      return;
    }
    panel.classList.remove("hidden");
    const body = ref("requestResultBody");
    replaceChildren(body, []);

    const warnings = (result.warnings ?? []).filter(
      (warning) => typeof warning === "string" && warning.length > 0,
    );
    if (warnings.length) {
      const warningBox = el("div", { class: "recv-warning-box", dataset: { tier: "review" } });
      warningBox.appendChild(
        el("p", { class: "recv-card-line", text: "Review before sharing this address:" }),
      );
      for (const warning of warnings) {
        warningBox.appendChild(el("p", { class: "recv-card-line", text: warning }));
      }
      body.appendChild(warningBox);
    }

    body.appendChild(
      el(
        "div",
        { class: "recv-copy-row" },
        el("span", { class: "mono", text: deposit.stealth_address }),
        el("button", {
          class: "btn-ghost btn-small",
          attrs: { type: "button" },
          text: "Copy deposit address",
          on: { click: () => void copyValue(deposit.stealth_address, "Deposit address") },
        }),
      ),
    );
    if (deposit.requested_gas_wei_hex) {
      body.appendChild(
        el("p", {
          class: "recv-card-line",
          text:
            "The payer is asked to attach " +
            (formatEthAmount(deposit.requested_gas_wei_hex, "ETH") ?? "gas") +
            " so the sweep can pay its own gas.",
        }),
      );
    }
    if (deposit.announcement) {
      body.appendChild(
        el("p", {
          class: "recv-card-line",
          text: "If the payer's wallet does not announce automatically, give them the announcer address and calldata from the deposit card below.",
        }),
      );
    }
  }

  async function submitErc20(): Promise<void> {
    const select = ref("erc20WalletSelect") as HTMLSelectElement;
    const token = ref("erc20Token") as HTMLInputElement;
    const note = ref("erc20Note") as HTMLInputElement;
    const requestGas = ref("erc20RequestGas") as HTMLInputElement;
    const gasAmount = ref("erc20GasAmount") as HTMLInputElement;
    clearFieldErrors([token, gasAmount]);
    if (!select.value || !token.value.trim()) {
      if (!token.value.trim()) markFieldInvalid(token, true);
      setSectionNote("erc20NoteLine", "A token address is required for an ERC-20 deposit.");
      return;
    }
    const body: Record<string, unknown> = {
      wallet_profile: select.value,
      token_address: token.value.trim(),
      auto_queue_sweep: true,
      request_gas: requestGas.checked,
    };
    if (note.value.trim()) body.note = note.value.trim();
    if (requestGas.checked && gasAmount.value.trim()) {
      const gasWeiHex = parseEthToWeiHex(gasAmount.value.trim());
      if (!gasWeiHex) {
        markFieldInvalid(gasAmount, true);
        setSectionNote("erc20NoteLine", "The gas amount must be an ETH amount like 0.005.");
        return;
      }
      body.gas_amount_wei_hex = gasWeiHex;
    }
    try {
      const r = await request<CreateDepositResult>(
        "POST",
        "/api/deposits/eth-stealth/create-erc20",
        body,
      );
      clearInputs([token, note, requestGas, gasAmount]);
      renderCreateResult(r);
      setSectionNote("erc20NoteLine", "ERC-20 deposit address created — it is listed above.");
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the ERC-20 deposit");
    }
  }

  async function submitScan(): Promise<void> {
    const select = ref("scanWalletSelect") as HTMLSelectElement;
    const fromBlock = ref("scanFromBlock") as HTMLInputElement;
    const toBlock = ref("scanToBlock") as HTMLInputElement;
    clearFieldErrors([fromBlock]);
    if (!select.value || !fromBlock.value.trim()) {
      if (!fromBlock.value.trim()) markFieldInvalid(fromBlock, true);
      setSectionNote("scanNoteLine", "A wallet and a starting block are required to scan.");
      return;
    }
    const body: Record<string, unknown> = {
      wallet_profile: select.value,
      from_block: fromBlock.value.trim(),
      limit: 1000,
      auto_queue_sweep: true,
    };
    if (toBlock.value.trim()) body.to_block = toBlock.value.trim();
    try {
      const r = await request<ScanAnnouncementsResult>(
        "POST",
        "/api/deposits/eth-stealth/scan-announcements",
        body,
      );
      clearInputs([fromBlock, toBlock]);
      setSectionNote(
        "scanNoteLine",
        "Scanned " +
          String(r.scanned ?? 0) +
          " announcements — " +
          String(r.matched ?? 0) +
          " matched this wallet, " +
          String(r.created ?? 0) +
          " new deposits tracked.",
      );
      await loadDeposits();
    } catch (error) {
      reportFailure(error, "the announcement scan");
    }
  }

  // ── DOM construction ──

  function section(
    titleText: string,
    summaryText: string,
  ): { wrap: HTMLElement; body: HTMLElement } {
    const body = el("div", { class: "recv-section-body" });
    const wrap = el(
      "section",
      { class: "recv-section" },
      el("h3", { class: "recv-section-title", text: titleText }),
      el("p", { class: "recv-section-summary", text: summaryText }),
      body,
    );
    return { wrap, body };
  }

  function buildDom(): HTMLElement {
    // Page header: the question this screen answers + the refresh action.
    refs.refreshNote = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    const header = el(
      "div",
      { class: "page-header" },
      el(
        "div",
        {},
        el("h2", { class: "page-header-title", text: "Receive" }),
        el("p", {
          class: "page-header-summary",
          text: "Get paid without linking your payments: dedicated addresses per payer, one-time addresses that sweep themselves, and stealth deposits tracked end to end.",
        }),
      ),
      el(
        "div",
        { class: "page-header-actions" },
        el("button", {
          class: "btn-ghost",
          attrs: { type: "button" },
          text: "Refresh balances",
          on: {
            click: (event) => void refreshBalances(event.target as HTMLButtonElement),
          },
        }),
      ),
    );

    // Persistent banners (NOT toasts): stale data + vault lock guidance.
    refs.staleBannerText = el("p", { class: "attention-item-body" });
    refs.staleBanner = el(
      "div",
      { class: "attention-item", dataset: { tier: "review" }, attrs: { role: "alert" } },
      el(
        "div",
        { class: "attention-item-main" },
        el("p", { class: "attention-item-title", text: "Showing stale data" }),
        refs.staleBannerText,
      ),
      el("button", {
        class: "btn-ghost btn-small attention-item-action",
        attrs: { type: "button" },
        text: "Retry",
        on: { click: () => void loadAll(false) },
      }),
    );
    refs.lockBannerText = el("p", { class: "attention-item-body" });
    refs.lockBanner = el(
      "div",
      { class: "attention-item", dataset: { tier: "review" }, attrs: { role: "alert" } },
      el(
        "div",
        { class: "attention-item-main" },
        el("p", { class: "attention-item-title", text: "Vault locked" }),
        refs.lockBannerText,
      ),
      el("button", {
        class: "btn-primary btn-small attention-item-action",
        attrs: { type: "button" },
        text: "Go to Vault",
        on: { click: () => runtime.router.navigate("#/vault") },
      }),
    );

    refs.copyNote = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });

    // ── (a) Address cards ──
    refs.coverageLine = el("p", { class: "recv-section-summary" });
    refs.addressEmpty = el("div", {});
    refs.addressGrid = el("div", { class: "recv-grid", attrs: { role: "list" } });
    const addresses = section(
      "Receive addresses",
      "Every active receive address and stealth surface. Balances come from the last saved scan; refreshing queries your provider, which then sees those addresses.",
    );
    addresses.body.appendChild(refs.coverageLine);
    addresses.body.appendChild(refs.refreshNote);
    addresses.body.appendChild(refs.addressEmpty);
    addresses.body.appendChild(refs.addressGrid);

    // ── (b) Allocate + one-time flow ──
    refs.allocateWallet = textInput("e.g. donations", "Wallet profile");
    refs.allocatePurpose = textInput("e.g. March invoices", "Purpose");
    refs.allocateLabel = textInput("optional", "Label");
    refs.allocateParty = document.createElement("select");
    refs.allocateParty.setAttribute("aria-label", "Counterparty (optional)");
    refs.allocateOneTime = document.createElement("input");
    (refs.allocateOneTime as HTMLInputElement).type = "checkbox";
    refs.allocateDestination = textInput("0x… — where the funds land", "Sweep destination");
    refs.allocateThreshold = textInput("optional, e.g. 0.05", "Sweep threshold in ETH");
    refs.allocatePurge = document.createElement("input");
    (refs.allocatePurge as HTMLInputElement).type = "checkbox";
    refs.allocateNote = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });

    refs.oneTimeOptions = el(
      "div",
      { class: "recv-onetime-options" },
      el("p", {
        class: "recv-section-summary",
        text: "How a one-time address works: hand it to one payer; when funds arrive, Sigillum sweeps them to the destination below and retires the address — the payer never sees a reused address linked to your treasury.",
      }),
      fieldWrapper(
        "Sweep destination",
        refs.allocateDestination,
        "Where funds go when they arrive — for example your cold treasury address.",
      ),
      fieldWrapper(
        "Sweep threshold (ETH, optional)",
        refs.allocateThreshold,
        "Wait until at least this much has arrived before sweeping. Empty sweeps any amount.",
      ),
      el(
        "label",
        { class: "checkbox-row" },
        refs.allocatePurge,
        "Purge the record after the sweep — maximum unlinkability, but you lose the local audit trail.",
      ),
    );

    (refs.allocateOneTime as HTMLInputElement).addEventListener("change", () => {
      toggleOneTimeOptions((refs.allocateOneTime as HTMLInputElement).checked);
    });

    refs.allocateSubmit = el("button", {
      class: "btn-primary",
      attrs: { type: "submit" },
      text: "Allocate address",
    });
    const allocateForm = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitAllocate();
          },
        },
      },
      el(
        "div",
        { class: "recv-form-grid" },
        fieldWrapper(
          "Wallet profile",
          refs.allocateWallet,
          "The wallet this address is derived from.",
        ),
        fieldWrapper("Purpose", refs.allocatePurpose, "Groups allocations — invoices, donations, …"),
        fieldWrapper("Label (optional)", refs.allocateLabel),
        fieldWrapper("Counterparty (optional)", refs.allocateParty),
      ),
      el(
        "label",
        { class: "checkbox-row" },
        refs.allocateOneTime,
        "Use this address once — sweep funds automatically, then retire it",
      ),
      refs.oneTimeOptions,
      el("div", { class: "recv-form-actions" }, refs.allocateSubmit),
      refs.allocateNote,
    );
    const allocate = section(
      "New receive address",
      "Allocate a dedicated address per wallet profile and purpose. Rotated addresses stay tracked as retired.",
    );
    allocate.body.appendChild(allocateForm);

    // ── (d) Payer instructions ──
    refs.metaWalletSelect = document.createElement("select");
    refs.metaWalletSelect.setAttribute("aria-label", "Stealth wallet for the meta-address");
    refs.metaNote = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    refs.metaAddressValue = el("code", { class: "mono recv-meta-address" });
    refs.metaSchemeLine = el("p", { class: "recv-card-line" });
    refs.metaResult = el(
      "div",
      { class: "recv-meta-result" },
      el("p", {
        class: "recv-field-label",
        text: "Your stealth meta-address — this is what the payer needs",
      }),
      el(
        "div",
        { class: "recv-copy-row" },
        refs.metaAddressValue,
        el("button", {
          class: "btn-ghost btn-small",
          attrs: { type: "button" },
          text: "Copy meta-address",
          on: {
            click: () =>
              void copyValue(refs.metaAddressValue.textContent ?? "", "Stealth meta-address"),
          },
        }),
      ),
      el("p", {
        class: "recv-card-line",
        text: "What the payer attaches: nothing by hand — their stealth wallet reads the meta-address and derives a fresh one-time address per payment, then announces it on-chain (ERC-5564). Only you can recognize and spend those payments.",
      }),
      refs.metaSchemeLine,
    );
    const metaForm = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitMetaAddress();
          },
        },
      },
      el("div", { class: "recv-form-grid" }, fieldWrapper("Stealth wallet", refs.metaWalletSelect)),
      el(
        "div",
        { class: "recv-form-actions" },
        el("button", {
          class: "btn-primary",
          attrs: { type: "submit" },
          text: "Show payer instructions",
        }),
      ),
      refs.metaNote,
      refs.metaResult,
    );

    refs.requestWalletSelect = document.createElement("select");
    refs.requestWalletSelect.setAttribute("aria-label", "Stealth wallet for the deposit address");
    refs.requestExpected = textInput("optional, e.g. 0.42", "Expected amount in ETH");
    refs.requestNote = textInput("optional — only you see this", "Note");
    refs.requestGas = document.createElement("input");
    (refs.requestGas as HTMLInputElement).type = "checkbox";
    refs.requestGasAmount = textInput("optional, e.g. 0.005", "Gas amount in ETH");
    refs.requestNoteLine = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    refs.requestResultBody = el("div", {});
    refs.requestResult = el(
      "div",
      { class: "recv-meta-result", attrs: { role: "status", "aria-live": "polite" } },
      el("p", {
        class: "recv-field-label",
        text: "Deposit address created — give it to the payer",
      }),
      refs.requestResultBody,
    );
    const requestForm = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitRequestPayment();
          },
        },
      },
      el(
        "div",
        { class: "recv-form-grid" },
        fieldWrapper("Stealth wallet", refs.requestWalletSelect),
        fieldWrapper("Expected amount (ETH, optional)", refs.requestExpected),
        fieldWrapper("Note (optional)", refs.requestNote),
      ),
      el(
        "label",
        { class: "checkbox-row" },
        refs.requestGas,
        "Ask the payer to attach gas for the sweep",
      ),
      el("p", {
        class: "recv-field-hint",
        text: "The announcement metadata follows the ERC-5564 native layout, so a payer wallet that implements it learns the total ETH to attach — payment plus the gas your sweep will spend. Without it, the sweep spends from the payment itself.",
      }),
      fieldWrapper("Gas amount (ETH, optional)", refs.requestGasAmount),
      el(
        "div",
        { class: "recv-form-actions" },
        el("button", {
          class: "btn-primary",
          attrs: { type: "submit" },
          text: "Create deposit address",
        }),
      ),
      refs.requestNoteLine,
      refs.requestResult,
    );

    const getPaid = section(
      "Get paid privately",
      "How do I get paid privately? Give the payer your stealth meta-address, or create a tracked deposit address for a specific payment.",
    );
    getPaid.wrap.dataset.section = "pay";
    getPaid.body.appendChild(metaForm);
    getPaid.body.appendChild(requestForm);
    refs.getPaidSection = getPaid.wrap;

    // ── (c) Stealth deposits lifecycle ──
    refs.depositFilter = document.createElement("select");
    refs.depositFilter.setAttribute("aria-label", "Filter deposits by state");
    for (const filter of DEPOSIT_FILTERS) {
      refs.depositFilter.appendChild(makeOption(filter.label, filter.value));
    }
    (refs.depositFilter as HTMLSelectElement).addEventListener("change", () => {
      depositsFilter = (refs.depositFilter as HTMLSelectElement).value;
      depositsOffset = 0;
      void loadDeposits().catch((error) => reportFailure(error, "deposits"));
    });
    refs.depositPrev = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "Previous",
      on: {
        click: () => {
          depositsOffset = Math.max(0, depositsOffset - DEPOSITS_PAGE_SIZE);
          void loadDeposits().catch((error) => reportFailure(error, "deposits"));
        },
      },
    });
    refs.depositNext = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "Next",
      on: {
        click: () => {
          depositsOffset += DEPOSITS_PAGE_SIZE;
          void loadDeposits().catch((error) => reportFailure(error, "deposits"));
        },
      },
    });
    refs.depositPageNote = el("span", { class: "recv-page-note" });
    refs.depositNote = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    refs.depositEmpty = el("div", {});
    refs.depositList = el("div", { class: "recv-deposit-list", attrs: { role: "list" } });
    const depositsSection = section(
      "Stealth deposits",
      "Each payment moves through a guided lifecycle: announced, funded, gas-ready, swept. Gas problems are explained on the card.",
    );
    depositsSection.wrap.dataset.section = "deposits";
    depositsSection.body.appendChild(
      el(
        "div",
        { class: "recv-deposit-toolbar" },
        refs.depositFilter,
        el("button", {
          class: "btn-ghost btn-small",
          attrs: { type: "button" },
          text: "Refresh deposits",
          on: {
            click: (event) => void refreshAllDeposits(event.target as HTMLButtonElement),
          },
        }),
        refs.depositPageNote,
        refs.depositPrev,
        refs.depositNext,
      ),
    );
    depositsSection.body.appendChild(refs.depositNote);
    depositsSection.body.appendChild(refs.depositEmpty);
    depositsSection.body.appendChild(refs.depositList);
    depositsSection.body.appendChild(buildAdvancedDeposits());
    refs.depositsSection = depositsSection.wrap;

    // ── Counterparties ──
    refs.partyEmpty = el("div", {});
    refs.partyList = el("div", { class: "recv-party-list", attrs: { role: "list" } });
    refs.partyName = textInput("e.g. Acme Ltd", "Counterparty name");
    refs.partyNote = textInput("optional", "Note");
    refs.partyDestination = textInput("optional 0x… — default sweep destination", "Sweep destination");
    refs.partyNoteLine = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    const partyForm = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitParty();
          },
        },
      },
      el(
        "div",
        { class: "recv-form-grid" },
        fieldWrapper("Name", refs.partyName),
        fieldWrapper("Note (optional)", refs.partyNote),
        fieldWrapper("Sweep destination (optional)", refs.partyDestination),
      ),
      el(
        "div",
        { class: "recv-form-actions" },
        el("button", { class: "btn-primary", attrs: { type: "submit" }, text: "Add counterparty" }),
      ),
      refs.partyNoteLine,
    );
    const partiesSection = section(
      "Counterparties",
      "Track payers as parties so each one gets dedicated addresses and deposits stay attributed.",
    );
    partiesSection.body.appendChild(refs.partyEmpty);
    partiesSection.body.appendChild(refs.partyList);
    partiesSection.body.appendChild(partyForm);

    // Elements that start hidden get the class via classList (not className)
    // so runtime toggling behaves identically in the browser and the fake
    // DOM harness, where className and classList are not synced.
    for (const node of [
      refs.staleBanner,
      refs.lockBanner,
      refs.oneTimeOptions,
      refs.metaResult,
      refs.requestResult,
      refs.addressEmpty,
      refs.depositEmpty,
      refs.partyEmpty,
      refs.refreshNote,
      refs.copyNote,
      refs.allocateNote,
      refs.metaNote,
      refs.requestNoteLine,
      refs.erc20NoteLine,
      refs.scanNoteLine,
      refs.partyNoteLine,
      refs.depositNote,
    ]) {
      node.classList.add("hidden");
    }

    return el(
      "div",
      { class: "dest-recv" },
      header,
      refs.staleBanner,
      refs.lockBanner,
      refs.copyNote,
      addresses.wrap,
      allocate.wrap,
      getPaid.wrap,
      depositsSection.wrap,
      partiesSection.wrap,
    );
  }

  function buildAdvancedDeposits(): HTMLElement {
    refs.erc20WalletSelect = document.createElement("select");
    refs.erc20WalletSelect.setAttribute("aria-label", "Stealth wallet for the ERC-20 deposit");
    refs.erc20Token = textInput("0x… token contract", "Token address");
    refs.erc20Note = textInput("optional", "Note");
    refs.erc20RequestGas = document.createElement("input");
    (refs.erc20RequestGas as HTMLInputElement).type = "checkbox";
    refs.erc20GasAmount = textInput("optional, e.g. 0.005", "Gas amount in ETH");
    refs.erc20NoteLine = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    const erc20Form = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitErc20();
          },
        },
      },
      el(
        "div",
        { class: "recv-form-grid" },
        fieldWrapper("Stealth wallet", refs.erc20WalletSelect),
        fieldWrapper("Token address", refs.erc20Token),
        fieldWrapper("Note (optional)", refs.erc20Note),
      ),
      el(
        "label",
        { class: "checkbox-row" },
        refs.erc20RequestGas,
        "Ask the payer to attach gas for the sweep",
      ),
      el("p", {
        class: "recv-field-hint",
        text: "Token transfers cannot pay their own sweep gas. With the gas option the announcement metadata (ERC-5564 token layout) tells the payer's wallet which token and amount to send, plus the native gas to attach to the deposit address.",
      }),
      fieldWrapper("Gas amount (ETH, optional)", refs.erc20GasAmount),
      el(
        "div",
        { class: "recv-form-actions" },
        el("button", {
          class: "btn-primary",
          attrs: { type: "submit" },
          text: "Create ERC-20 deposit",
        }),
      ),
      refs.erc20NoteLine,
    );

    refs.scanWalletSelect = document.createElement("select");
    refs.scanWalletSelect.setAttribute("aria-label", "Stealth wallet to scan for");
    refs.scanFromBlock = textInput("block tag or 0x block", "From block");
    refs.scanToBlock = textInput("optional — latest", "To block");
    refs.scanNoteLine = el("p", {
      class: "recv-inline-note",
      attrs: { role: "status", "aria-live": "polite" },
    });
    const scanForm = el(
      "form",
      {
        class: "recv-form",
        on: {
          submit: (event) => {
            event.preventDefault();
            void submitScan();
          },
        },
      },
      el(
        "div",
        { class: "recv-form-grid" },
        fieldWrapper("Stealth wallet", refs.scanWalletSelect),
        fieldWrapper("From block", refs.scanFromBlock),
        fieldWrapper("To block (optional)", refs.scanToBlock),
      ),
      el(
        "div",
        { class: "recv-form-actions" },
        el("button", { class: "btn-primary", attrs: { type: "submit" }, text: "Scan announcements" }),
      ),
      refs.scanNoteLine,
    );

    return el(
      "details",
      { class: "recv-advanced" },
      el("summary", { text: "Advanced: ERC-20 deposit & announcement scan" }),
      el("h4", { class: "recv-sub-title", text: "Create ERC-20 deposit" }),
      erc20Form,
      el("h4", { class: "recv-sub-title", text: "Scan stealth payments (ERC-5564)" }),
      el("p", {
        class: "recv-section-summary",
        text: "Scan on-chain announcements for payments made to this wallet's stealth meta-address and track them as deposits.",
      }),
      scanForm,
    );
  }

  // ── Lifecycle ──

  function takeOverHost(): void {
    host = document.getElementById(RECEIVING_HOST_ID);
    if (!host) return;
    stashedChildren = Array.from(host.childNodes);
    for (const child of stashedChildren) {
      (child as ChildNode).remove();
    }
    for (const id of REPLACED_SIBLING_IDS) {
      document.getElementById(id)?.classList.add("hidden");
    }
    root = buildDom();
    host.appendChild(root);
  }

  function restoreHost(): void {
    if (root) root.remove();
    root = null;
    if (host) {
      for (const child of stashedChildren) {
        host.appendChild(child);
      }
    }
    stashedChildren = [];
    for (const id of REPLACED_SIBLING_IDS) {
      document.getElementById(id)?.classList.remove("hidden");
    }
    host = null;
  }

  function scrollToSubSection(route: Route): void {
    const sub = route.path[0];
    if (!sub) return;
    const target =
      sub === "deposits" ? refs.depositsSection : sub === "pay" ? refs.getPaidSection : null;
    target?.scrollIntoView();
  }

  return {
    id: "receive",
    migrated: true,
    mount(route: Route): void {
      // Destination-owned sub-routes (adapter contract rule 1).
      runtime.router.register("receive", "deposits");
      runtime.router.register("receive", "pay");

      resetState();
      takeOverHost();
      if (!host) return;

      scrollToSubSection(route);
      void loadAll(true);

      // Live updates come from the store slices (no new pollers):
      // resync → refetch everything; queueEvents → deposit sweep states move;
      // status → lock/unlock transitions clear the lock banner.
      unsubscribers.push(
        runtime.store.subscribe("resync", () => {
          void loadAll(false);
        }),
        runtime.store.subscribe("queueEvents", () => {
          void loadDeposits().catch((error) => reportFailure(error, "deposits"));
        }),
        runtime.store.subscribe("status", (status) => {
          if (status && status.locked === false) {
            hideBanner("locked");
          }
        }),
      );
    },
    unmount(): void {
      for (const unsubscribe of unsubscribers.splice(0)) unsubscribe();
      restoreHost();
    },
  };
}
