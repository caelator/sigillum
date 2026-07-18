/**
 * destinations/Move.ts — the Move destination controller (plan task 4.3.1).
 *
 * Move is the signature destination: plan review is treated like a
 * hardware-wallet confirmation, the queue reads as an ops timeline, and the
 * treasury policy is a guided editor with presets and a plain-English
 * summary. It renders INTO the four legacy Move cards (`plansCard`,
 * `queueCard`, `policyCard`, `maintenanceCard` — the only
 * `data-workspace-section="move"` hosts) and restores their original markup
 * on unmount, so the legacy console keeps working for the unmigrated
 * destinations.
 *
 * Sub-routes owned here (adapter contract rule 1):
 *   `#/move`            plan list + generation, queue, policy, maintenance
 *   `#/move/plan/:id`   the plan review screen (aux cards concealed)
 *   `#/move/queue`      list mode, queue card focused
 *   `#/move/policy`     list mode, policy card focused
 *
 * Live data: plans/queue/policy/parties/chains are fetched through the typed
 * core client (or thin wrappers below around `requestWithSession` for the
 * endpoints it lacks — core/api.ts is not extended). Refresh triggers are
 * the store slices (`resync` refetches everything, `queueEvents` refetches
 * the queue, `operations` drives the background-operation strip, `status`
 * refetches after unlock). No new pollers, no timers.
 *
 * The bulk-enqueue typed-confirmation flow preserves the legacy server
 * contract EXACTLY (see enqueuePlanWithTypedConfirm): probe with an empty
 * confirmation, read the daemon-computed phrase from the machine-readable
 * `action` field, gate the shared typed dialog on that phrase, then re-POST
 * with it. The daemon re-validates everything; this UI never widens what it
 * allows.
 */

import type {
  ChainProfile,
  ConsolidationPlan,
  ConsolidationPlanExportResponse,
  ConsolidationPlanGenerateRequest,
  ConsolidationPlanStep,
  Counterparty,
  FieldError,
  Operation,
  PaginationInfo,
  PartyDestination,
  QueueJob,
  StatusResponse,
  TreasuryAllowedDestination,
  TreasuryPolicy,
  TreasuryPolicyUpdateRequest,
} from "../contracts";
import { requestWithSession, type DaemonPayload } from "../api/session";
import { apiFailure, type ApiFailure } from "../core/api";
import { el, renderList } from "../core/dom";
import type { CoreRuntime } from "../core/live";
import {
  formatHash,
  type DestinationController,
  type Route,
} from "../core/router";
import type { Unsubscribe } from "../core/store";
import { confirmDangerDialog, confirmTypedDialog } from "../render/confirm";
import {
  chainLabel,
  formatHexQuantity,
  formatTimestamp,
  formatTokenAmount,
} from "../render/format";
import { pillClass } from "../render/html";

// ── Constants ────────────────────────────────────────────────────────────

const PLANS_PAGE_SIZE = 20;
const QUEUE_PAGE_SIZE = 25;
const DEFAULT_SIM_FRESHNESS_SECS = 900;
const WEI_PER_ETH = 10n ** 18n;
const WEI_PER_GWEI = 10n ** 9n;

/** Operation kinds shown in the queue card's background-operation strip. */
const MOVE_OPERATION_KINDS = new Set(["queue_process", "maintenance_run"]);
const ACTIVE_OPERATION_STATES = new Set(["running", "cancel_requested"]);

type NoticeTone = "info" | "success" | "warning" | "error";
type Tier = "quiet" | "review" | "danger";

// ── Small shared DOM helpers ─────────────────────────────────────────────

/** Remove every child (fake-DOM safe: `textContent = ""` does not detach). */
function clearChildren(node: Element): void {
  const children = Array.from(
    (node as unknown as { childNodes: ArrayLike<Node> }).childNodes,
  ) as Node[];
  for (const child of children) (child as ChildNode).remove();
}

/** Fingerprints of the last keyed-row render, so unchanged rows keep their
 * nodes (and any focus inside them) across live refreshes. */
const rowFingerprints = new WeakMap<Element, string>();

function patchKeyedRow(
  existing: HTMLElement | null,
  className: string,
  fingerprint: string,
  build: (row: HTMLElement) => void,
): HTMLElement {
  const row = existing ?? el("div", { class: className });
  if (existing && rowFingerprints.get(existing) === fingerprint) {
    return existing;
  }
  clearChildren(row);
  row.className = className;
  build(row);
  rowFingerprints.set(row, fingerprint);
  return row;
}

function setHidden(element: HTMLElement | null, hidden: boolean): void {
  element?.classList.toggle("hidden", hidden);
}

function setBusy(button: HTMLElement | null, busy: boolean): void {
  if (!button) return;
  button.classList.toggle("btn-busy", busy);
  (button as HTMLButtonElement).disabled = busy;
}

function pill(status: unknown, label?: string): HTMLElement {
  return el("span", {
    class: "pill " + pillClass(status),
    text: label ?? String(status || "unknown").replace(/_/g, " "),
  });
}

function tierChip(text: string, tier: Tier): HTMLElement {
  return el("span", {
    class: "move-chip",
    dataset: { tier },
    text,
  });
}

// ── Time formatting ──────────────────────────────────────────────────────

export function relativeTime(
  unixSecs: number | null | undefined,
  nowSecs: number,
): string {
  if (!unixSecs) return "never";
  const delta = nowSecs - unixSecs;
  if (delta < 45) return "just now";
  const minutes = Math.round(delta / 60);
  if (minutes < 60) return minutes + "m ago";
  const hours = Math.round(minutes / 60);
  if (hours < 48) return hours + "h ago";
  return Math.round(hours / 24) + "d ago";
}

export function futureTime(
  unixSecs: number | null | undefined,
  nowSecs: number,
): string {
  if (!unixSecs) return "-";
  const delta = unixSecs - nowSecs;
  if (delta <= 0) return "due now";
  const minutes = Math.round(delta / 60);
  if (minutes < 1) return "in under a minute";
  if (minutes < 60) return "in " + minutes + "m";
  const hours = Math.round(minutes / 60);
  if (hours < 48) return "in " + hours + "h";
  return "in " + Math.round(hours / 24) + "d";
}

function timeEl(
  unixSecs: number | null | undefined,
  nowSecs: number,
): HTMLElement {
  return el("span", {
    class: "move-time",
    text: relativeTime(unixSecs, nowSecs),
    attrs: { title: unixSecs ? formatTimestamp(unixSecs) : "never" },
  });
}

// ── Amount / address formatting (ported from views/treasury.ts so this
// module stays self-contained; behavior identical) ───────────────────────

export function formatWeiHexAsEth(
  weiHex: string | null | undefined,
): string | null {
  return formatTokenAmount(weiHex, 18);
}

export function formatWeiHexAsGwei(
  weiHex: string | null | undefined,
): string | null {
  return formatTokenAmount(weiHex, 9);
}

export function parseEthToWeiHex(value: string): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  const match = /^(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!match) return null;
  const fractionDigits = match[2] || "";
  if (fractionDigits.length > 18) return null;
  const wei =
    BigInt(match[1]) * WEI_PER_ETH + BigInt(fractionDigits.padEnd(18, "0"));
  return "0x" + wei.toString(16);
}

export function parseGweiToWeiHex(value: string): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  const match = /^(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!match) return null;
  const fractionDigits = match[2] || "";
  if (fractionDigits.length > 9) return null;
  const wei =
    BigInt(match[1]) * WEI_PER_GWEI + BigInt(fractionDigits.padEnd(9, "0"));
  return "0x" + wei.toString(16);
}

/** Middle-truncated address, e.g. `0x71C7…976F`. Non-hex input shortens plainly. */
export function shortAddress(value: string | null | undefined): string {
  const trimmed = (value || "").trim();
  if (!trimmed) return "-";
  if (trimmed.startsWith("0x") && trimmed.length > 14) {
    return trimmed.slice(0, 6) + "…" + trimmed.slice(-4);
  }
  return trimmed.length > 24
    ? trimmed.slice(0, 12) + "…" + trimmed.slice(-6)
    : trimmed;
}

export function shortId(value: string | null | undefined): string {
  const trimmed = (value || "").trim();
  if (!trimmed) return "-";
  return trimmed.length > 16
    ? trimmed.slice(0, 8) + "…" + trimmed.slice(-4)
    : trimmed;
}

function joinEnglishList(items: string[]): string {
  if (items.length <= 1) return items.join("");
  if (items.length === 2) return items[0] + " and " + items[1];
  return items.slice(0, -1).join(", ") + ", and " + items[items.length - 1];
}

// ── Plan-step logic (ported from views/inventory.ts — identical gates) ──

/// Per-family W7.1 execution gate field for each plan-step action.
/// review_asset is deliberately absent: it is never executable.
const EXECUTION_FAMILY_GATE: Record<string, string> = {
  sweep_native: "allow_sweep_execution",
  sweep_erc20: "allow_sweep_execution",
  sweep_nft: "allow_sweep_execution",
  revoke_erc20_approval: "allow_revoke_execution",
  revoke_permit2_allowance: "allow_revoke_execution",
  revoke_nft_operator_approval: "allow_revoke_execution",
  revoke_approval: "allow_revoke_execution",
  approve_erc20: "allow_revoke_execution",
  exit_defi_position: "allow_exit_execution",
  claim_reward: "allow_claim_execution",
  fund_gas: "allow_gas_topups",
};

export function stepSimulatedAtUnix(step: ConsolidationPlanStep): number | null {
  let simulatedAt: number | null = null;
  for (const item of step.simulation_evidence || []) {
    if (item.startsWith("simulated_at_unix=")) {
      const value = Number(item.slice("simulated_at_unix=".length));
      if (Number.isFinite(value)) simulatedAt = value;
    }
  }
  return simulatedAt;
}

/// Mirror of the daemon's enqueue eligibility gates for the Execute
/// affordance ONLY (ported verbatim from the legacy view): policy on + gates
/// on + not paused + approved + fresh passed simulation + unblocked + not
/// already enqueued. The daemon re-validates everything server-side.
export function stepExecutionEligible(
  step: ConsolidationPlanStep,
  policy: TreasuryPolicy | Record<string, unknown> | null | undefined,
  nowSecs: number,
): boolean {
  const gates = policy as Record<string, unknown> | null | undefined;
  if (!gates || !gates.enabled) return false;
  if (gates.execution_paused) return false;
  if (!gates.allow_plan_execution) return false;
  const gateField = EXECUTION_FAMILY_GATE[step.action];
  if (!gateField || !gates[gateField]) return false;
  if (!step.approved || step.status !== "approved") return false;
  if ((step.blockers || []).length) return false;
  if (step.simulation_status !== "passed") return false;
  const simulatedAt = stepSimulatedAtUnix(step);
  if (simulatedAt === null) return false;
  const freshnessSecs = Number(
    gates.simulation_freshness_secs ?? DEFAULT_SIM_FRESHNESS_SECS,
  );
  if (nowSecs - simulatedAt > freshnessSecs) return false;
  if (step.queued_job_id) return false;
  return true;
}

export function blockerLabel(code: string): string {
  switch (code) {
    case "missing_party_destination":
      return "No destination set for this payer";
    case "missing_destination":
      return "No destination set";
    case "cross_party_linkage":
      return "Destination shared with another payer";
    case "claim_execution_disabled":
      return "Claim execution disabled (needs policy opt-in, passed simulation, trusted/reviewed claim contract, and approval)";
    default:
      return code;
  }
}

/** One `key=value` item from a step's simulation evidence list. */
export function evidenceValue(
  step: ConsolidationPlanStep,
  key: string,
): string | null {
  for (const item of step.simulation_evidence || []) {
    if (item.startsWith(key + "=")) return item.slice(key.length + 1);
  }
  return null;
}

/** Gas story parsed from simulation evidence: limit + per-gas cap. */
export function stepGasInfo(step: ConsolidationPlanStep): {
  gasLimit: number | null;
  maxFeePerGasWeiHex: string | null;
} {
  const maxFee = evidenceValue(step, "max_fee_per_gas_hex");
  const limitText =
    evidenceValue(step, "transaction_gas_limit") ??
    evidenceValue(step, "native_gas_limit");
  const gasLimit =
    limitText && /^\d+$/.test(limitText) ? parseInt(limitText, 10) : null;
  return { gasLimit, maxFeePerGasWeiHex: maxFee };
}

/** Worst-case fee (max fee per gas × gas limit) as a wei hex quantity. */
export function stepFeeCapWeiHex(step: ConsolidationPlanStep): string | null {
  const { gasLimit, maxFeePerGasWeiHex } = stepGasInfo(step);
  if (gasLimit === null || !maxFeePerGasWeiHex) return null;
  try {
    return "0x" + (BigInt(maxFeePerGasWeiHex) * BigInt(gasLimit)).toString(16);
  } catch (_) {
    return null;
  }
}

export interface SimulationBadge {
  kind: "fresh" | "stale" | "missing" | "failed" | "unsupported";
  text: string;
  tier: Tier;
}

/** Simulation badge with evidence age: fresh / stale / missing / failed. */
export function simulationBadge(
  step: ConsolidationPlanStep,
  freshnessSecs: number,
  nowSecs: number,
): SimulationBadge {
  const status = step.simulation_status || "not_run";
  if (status === "passed") {
    const at = stepSimulatedAtUnix(step);
    if (at === null) {
      return {
        kind: "missing",
        text: "Simulation passed, evidence age unknown",
        tier: "review",
      };
    }
    if (nowSecs - at > freshnessSecs) {
      return {
        kind: "stale",
        text: "Simulation stale · ran " + relativeTime(at, nowSecs),
        tier: "review",
      };
    }
    return {
      kind: "fresh",
      text: "Simulated " + relativeTime(at, nowSecs),
      tier: "quiet",
    };
  }
  if (status === "failed") {
    return { kind: "failed", text: "Simulation failed", tier: "danger" };
  }
  if (status === "blocked") {
    return { kind: "failed", text: "Simulation blocked", tier: "danger" };
  }
  if (status === "unsupported") {
    return {
      kind: "unsupported",
      text: "Simulation unsupported",
      tier: "review",
    };
  }
  return { kind: "missing", text: "Not simulated", tier: "review" };
}

// ── Destination trust chips ──────────────────────────────────────────────

export interface DestinationTrust {
  kind: "unset" | "allowlisted" | "party" | "foreign";
  /** Chip label, e.g. "Treasury vault (cold)". */
  label: string;
  tier: Tier;
}

export function destinationTrust(
  address: string | null | undefined,
  policy: TreasuryPolicy | null | undefined,
  parties: Counterparty[],
): DestinationTrust {
  const normalized = (address || "").trim().toLowerCase();
  if (!normalized) {
    return { kind: "unset", label: "No destination", tier: "danger" };
  }
  const allowed = (policy?.allowed_destinations || []).find(
    (destination) => destination.address.trim().toLowerCase() === normalized,
  );
  if (allowed) {
    const label = (allowed.label || "").trim();
    return {
      kind: "allowlisted",
      label: label || "Allowlisted",
      tier: "quiet",
    };
  }
  const party = parties.find(
    (candidate) =>
      (candidate.sweep_destination_address || "").trim().toLowerCase() ===
      normalized,
  );
  if (party) {
    return {
      kind: "party",
      label: (party.name || party.id) + " (party)",
      tier: "quiet",
    };
  }
  return { kind: "foreign", label: "Foreign destination", tier: "review" };
}

/** How the destination reads inside a plain-language step sentence. */
function destinationInWords(
  address: string | null | undefined,
  policy: TreasuryPolicy | null | undefined,
  parties: Counterparty[],
): string {
  const trust = destinationTrust(address, policy, parties);
  if (trust.kind === "allowlisted" && trust.label !== "Allowlisted") {
    return trust.label;
  }
  if (trust.kind === "party") {
    return trust.label.replace(/ \(party\)$/, "");
  }
  if (trust.kind === "foreign") {
    return shortAddress(address) + " (foreign)";
  }
  return shortAddress(address);
}

// ── Plain-language step sentences ────────────────────────────────────────

function assetKindLabel(kind: string): string {
  switch (kind) {
    case "native":
      return "native";
    case "erc20":
      return "token";
    case "erc721":
    case "erc1155":
    case "nft":
      return "NFT";
    default:
      return kind.replace(/_/g, " ");
  }
}

export interface StepSentenceContext {
  symbol?: string;
  policy?: TreasuryPolicy | null;
  parties?: Counterparty[];
}

/** "Sweep 0.42 ETH from 0x71C7…976F → Treasury vault (cold)". */
export function stepPlainLanguage(
  step: ConsolidationPlanStep,
  context: StepSentenceContext = {},
): string {
  const symbol = context.symbol || "ETH";
  const policy = context.policy ?? null;
  const parties = context.parties ?? [];
  const from = shortAddress(step.address);
  const destination = destinationInWords(
    step.destination_address,
    policy,
    parties,
  );
  const amount = formatWeiHexAsEth(step.amount_hex) ?? "an unknown amount of";
  const asset = shortAddress(step.asset_address);
  const counterparty = shortAddress(step.counterparty_address);
  switch (step.action) {
    case "sweep_native":
      return (
        "Sweep " + amount + " " + symbol + " from " + from + " → " + destination
      );
    case "sweep_erc20":
      return (
        "Sweep " +
        amount +
        " of token " +
        asset +
        " from " +
        from +
        " → " +
        destination
      );
    case "sweep_nft": {
      const tokenId = step.token_id_hex
        ? " #" + (formatHexQuantity(step.token_id_hex) ?? step.token_id_hex)
        : "";
      return (
        "Send NFT" + tokenId + " of " + asset + " from " + from + " → " + destination
      );
    }
    case "fund_gas":
      return (
        "Top up " +
        shortAddress(step.destination_address) +
        " with " +
        amount +
        " " +
        symbol +
        " of gas from sponsor " +
        from
      );
    case "revoke_erc20_approval":
      return (
        "Revoke " +
        counterparty +
        "'s spending approval on token " +
        asset +
        " at " +
        from
      );
    case "revoke_permit2_allowance":
      return (
        "Revoke the Permit2 allowance for " +
        counterparty +
        " on token " +
        asset +
        " at " +
        from
      );
    case "revoke_nft_operator_approval":
      return "Revoke NFT operator " + counterparty + " on " + asset + " at " + from;
    case "revoke_approval":
      return "Revoke the approval for " + counterparty + " on " + asset + " at " + from;
    case "approve_erc20":
      return "Approve token spending for " + counterparty + " on " + asset + " at " + from;
    case "exit_defi_position":
      return (
        "Exit the DeFi position on protocol " +
        shortAddress(step.protocol_address) +
        " at " +
        from
      );
    case "claim_reward":
      return (
        "Claim rewards via " + (step.claim_adapter || "the claim adapter") + " at " + from
      );
    case "review_asset":
      return (
        "Review the " +
        assetKindLabel(step.asset_kind) +
        " holding at " +
        from +
        " (no automatic action)"
      );
    default:
      return (
        String(step.action).replace(/_/g, " ") +
        " · " +
        assetKindLabel(step.asset_kind) +
        " at " +
        from
      );
  }
}

/** Total native value moved by a plan's sweep/top-up steps (running total). */
export function planNativeTotalWeiHex(plan: ConsolidationPlan): string | null {
  let total = 0n;
  let any = false;
  for (const step of plan.steps || []) {
    if (step.action !== "sweep_native" && step.action !== "fund_gas") continue;
    try {
      total += BigInt(step.amount_hex);
      any = true;
    } catch (_) {
      /* unparseable step amount — excluded from the total */
    }
  }
  return any ? "0x" + total.toString(16) : null;
}

/** "12 steps · 3 need review · 1 blocked · up to 0.42 ETH". */
export function planCountsLine(
  plan: ConsolidationPlan,
  symbol: string,
): string {
  const summary = plan.summary || ({} as ConsolidationPlan["summary"]);
  const parts: string[] = [
    String(summary.total_steps ?? (plan.steps || []).length) + " steps",
  ];
  if (summary.review_required_steps) {
    parts.push(String(summary.review_required_steps) + " need review");
  }
  if (summary.blocked_steps) {
    parts.push(String(summary.blocked_steps) + " blocked");
  }
  if (summary.approved_steps) {
    parts.push(String(summary.approved_steps) + " approved");
  }
  const total = planNativeTotalWeiHex(plan);
  if (total) {
    parts.push("up to " + (formatWeiHexAsEth(total) ?? "?") + " " + symbol);
  }
  return parts.join(" · ");
}

// ── Treasury policy summary (ported from views/treasury.ts, Phase 0) ────

/// Plain-English description of what the current policy permits, in 2-4
/// short sentences.
export function treasuryPolicySummary(policy: TreasuryPolicy): string[] {
  const sentences: string[] = [];
  if (!policy.enabled) {
    sentences.push(
      "The policy is disabled, so nothing may execute — no plan steps and no stealth deposit sweeps.",
    );
  } else if (!policy.allow_plan_execution) {
    sentences.push(
      "Plan execution is switched off, so no plan step or stealth deposit sweep may execute yet.",
    );
  } else {
    const allowed: string[] = [];
    if (policy.allow_sweep_execution) allowed.push("sweeps");
    if (policy.allow_revoke_execution) allowed.push("revokes");
    if (policy.allow_exit_execution) allowed.push("DeFi exits");
    if (policy.allow_claim_execution) allowed.push("claims");
    if (allowed.length) {
      sentences.push("Plans may execute " + joinEnglishList(allowed) + ".");
    }
    const blocked: string[] = [];
    if (!policy.allow_claim_execution) blocked.push("Claims");
    if (!policy.allow_exit_execution) blocked.push("DeFi exits");
    if (!policy.allow_sweep_execution) blocked.push("Sweeps");
    if (!policy.allow_revoke_execution) blocked.push("Revokes");
    if (blocked.length) {
      sentences.push(joinEnglishList(blocked) + " are blocked.");
    }
    sentences.push(
      policy.allow_sweep_execution
        ? "The sweep gate also covers stealth deposit sweeps and transfers."
        : "Stealth deposit sweeps and transfers stay blocked until the sweep gate is on.",
    );
  }
  sentences.push(
    policy.block_cross_party_linkage
      ? "Cross-party linkage blocking is on; destinations are limited to the allow-list below."
      : "Cross-party linkage blocking is off — plans may route different payers to a shared destination.",
  );
  const gasTopupCap = (policy.max_gas_topup_wei_hex || "").trim();
  const gasTopupCapIsValid = /^0[xX][0-9a-fA-F]{1,64}$/.test(gasTopupCap);
  const operational: string[] = [];
  if (!policy.allow_gas_topups) {
    operational.push("Sponsor gas top-ups (plan and stealth) are off");
  } else if (!gasTopupCapIsValid) {
    operational.push(
      "Sponsor gas top-ups (plan and stealth) remain disabled until a valid explicit maximum is saved",
    );
  } else {
    operational.push(
      "Sponsor gas top-ups (plan and stealth) are allowed up to the explicit maximum",
    );
  }
  if (policy.execution_paused) {
    operational.push("queue execution is currently paused");
  }
  sentences.push(operational.join("; ") + ".");
  return sentences;
}

// ── Policy presets ───────────────────────────────────────────────────────

export interface PolicyPresetGateValues {
  enabled: boolean;
  require_simulation: boolean;
  block_cross_party_linkage: boolean;
  allow_plan_execution: boolean;
  allow_sweep_execution: boolean;
  allow_revoke_execution: boolean;
  allow_exit_execution: boolean;
  allow_claim_execution: boolean;
  allow_gas_topups: boolean;
  allow_treasury_automation: boolean;
}

export interface PolicyPreset {
  id: "consolidation" | "recovery" | "custom";
  label: string;
  description: string;
  /** Gate values the preset pre-fills; null = leave the form as-is (custom). */
  values: PolicyPresetGateValues | null;
}

export const POLICY_PRESETS: PolicyPreset[] = [
  {
    id: "consolidation",
    label: "Consolidation",
    description:
      "Everyday consolidation: sweeps to allow-listed destinations only. Simulation and linkage blocking stay on; revokes, DeFi exits, claims, and gas top-ups stay off.",
    values: {
      enabled: true,
      require_simulation: true,
      block_cross_party_linkage: true,
      allow_plan_execution: true,
      allow_sweep_execution: true,
      allow_revoke_execution: false,
      allow_exit_execution: false,
      allow_claim_execution: false,
      allow_gas_topups: false,
      allow_treasury_automation: false,
    },
  },
  {
    id: "recovery",
    label: "Recovery operator",
    description:
      "Everything needed to evacuate a wallet: sweeps, revokes, DeFi exits, and sponsor gas top-ups on. Claims stay off; simulation and linkage blocking stay on. Enter a finite maximum gas top-up below before saving; Sigillum does not assume a cap.",
    values: {
      enabled: true,
      require_simulation: true,
      block_cross_party_linkage: true,
      allow_plan_execution: true,
      allow_sweep_execution: true,
      allow_revoke_execution: true,
      allow_exit_execution: true,
      allow_claim_execution: false,
      allow_gas_topups: true,
      allow_treasury_automation: false,
    },
  },
  {
    id: "custom",
    label: "Custom",
    description:
      "Adjust individual gates and caps below. The summary sentence previews exactly what the policy will allow.",
    values: null,
  },
];

// ── Policy destination lines (ported from views/treasury.ts) ────────────

export function parseTreasuryDestinationLines(
  value: string | null | undefined,
): TreasuryAllowedDestination[] {
  const destinations: TreasuryAllowedDestination[] = [];
  (value || "").split(/\r?\n/).forEach((line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const colonIndex = trimmed.indexOf(":");
    if (colonIndex < 0) {
      destinations.push({ address: trimmed });
      return;
    }
    const address = trimmed.slice(0, colonIndex).trim();
    if (!address) return;
    const label = trimmed.slice(colonIndex + 1).trim();
    destinations.push(label ? { address, label } : { address });
  });
  return destinations;
}

export function formatDestinationLines(
  destinations: TreasuryAllowedDestination[] | undefined,
): string {
  return (destinations || [])
    .map((destination) =>
      destination.label
        ? destination.address + ":" + destination.label
        : destination.address,
    )
    .join("\n");
}

// ── Queue logic ──────────────────────────────────────────────────────────

/** Grouping order; `operator_action_required` always surfaces first. */
const QUEUE_STATE_ORDER = [
  "operator_action_required",
  "blocked",
  "retrying",
  "queued",
  "prepared",
  "submitted_unknown",
  "sent",
  "deferred",
  "confirmed",
  "failed_terminal",
  "failed",
];

export function queueStateLabel(state: string): string {
  switch (state) {
    case "queued":
      return "Queued";
    case "blocked":
      return "Blocked";
    case "retrying":
      return "Retrying";
    case "prepared":
      return "Prepared";
    case "submitted_unknown":
      return "Broadcast (result unknown)";
    case "sent":
      return "Awaiting confirmation";
    case "confirmed":
      return "Confirmed";
    case "failed_terminal":
      return "Failed";
    case "failed":
      return "Failed";
    case "operator_action_required":
      return "Needs your action";
    case "deferred":
      return "Deferred";
    default:
      return state.replace(/_/g, " ");
  }
}

export function queueStateTier(state: string): Tier {
  if (state === "failed" || state === "failed_terminal") return "danger";
  if (
    state === "operator_action_required" ||
    state === "blocked" ||
    state === "submitted_unknown"
  ) {
    return "review";
  }
  return "quiet";
}

export function queueKindLabel(kind: string): string {
  switch (kind) {
    case "eth_stealth_transfer":
      return "Stealth transfer";
    case "eth_stealth_erc20_transfer":
      return "Stealth token transfer";
    case "eth_stealth_native_sweep":
      return "Stealth sweep";
    case "eth_stealth_erc20_sweep":
      return "Stealth token sweep";
    case "eth_stealth_gas_topup":
      return "Stealth gas top-up";
    case "eth_seed_transfer":
      return "Seed wallet transfer";
    case "eth_seed_native_sweep":
      return "Seed wallet sweep";
    case "eth_seed_erc20_sweep":
      return "Seed wallet token sweep";
    case "plan_step_execution":
      return "Plan step";
    default:
      return kind.replace(/_/g, " ");
  }
}

function planActionLabel(action: string | undefined): string {
  switch (action) {
    case "sweep_native":
      return "Sweep";
    case "sweep_erc20":
      return "Token sweep";
    case "sweep_nft":
      return "NFT transfer";
    case "fund_gas":
      return "Gas top-up";
    case "exit_defi_position":
      return "DeFi exit";
    case "claim_reward":
      return "Claim";
    case "approve_erc20":
      return "Token approval";
    default:
      return action && action.startsWith("revoke")
        ? "Revoke"
        : (action || "step").replace(/_/g, " ");
  }
}

/** Human one-liner for a queue job (ported semantics, no key=value dumps). */
export function describeQueueJob(
  job: QueueJob,
  context: { symbol?: string } = {},
): string {
  const symbol = context.symbol || "ETH";
  const amountText = (hex: string | null | undefined): string | null => {
    const amount = formatWeiHexAsEth(hex);
    return amount ? amount + " " + symbol : null;
  };
  switch (job.kind) {
    case "plan_step_execution": {
      const parts: string[] = [planActionLabel(job.action)];
      const amount = amountText(job.amount_hex ?? job.value_wei_hex);
      if (amount) parts.push(amount);
      if (job.source_address) parts.push("from " + shortAddress(job.source_address));
      if (job.destination_address) {
        parts.push("→ " + shortAddress(job.destination_address));
      }
      return parts.join(" ");
    }
    case "eth_stealth_transfer":
    case "eth_seed_transfer": {
      const parts = [queueKindLabel(job.kind)];
      const amount = amountText(job.value_wei_hex);
      if (amount) parts.push("of " + amount);
      parts.push(
        "to " + shortAddress(job.destination_address || job.recipient_address),
      );
      return parts.join(" ");
    }
    case "eth_stealth_native_sweep":
    case "eth_seed_native_sweep": {
      const parts = [queueKindLabel(job.kind)];
      const amount = amountText(job.value_wei_hex);
      if (amount) parts.push("of " + amount);
      parts.push("from " + shortAddress(job.stealth_address || job.address));
      if (job.destination_address) {
        parts.push("→ " + shortAddress(job.destination_address));
      }
      return parts.join(" ");
    }
    case "eth_stealth_erc20_transfer":
    case "eth_stealth_erc20_sweep":
    case "eth_seed_erc20_sweep": {
      const parts = [queueKindLabel(job.kind)];
      const amount = job.amount_hex ? amountText(job.amount_hex) : null;
      if (amount) parts.push("of " + amount);
      if (job.token_address) parts.push("token " + shortAddress(job.token_address));
      if (job.destination_address) {
        parts.push("→ " + shortAddress(job.destination_address));
      }
      return parts.join(" ");
    }
    case "eth_stealth_gas_topup":
      return (
        queueKindLabel(job.kind) +
        " for " +
        shortAddress(job.address || job.stealth_address) +
        " from sponsor " +
        shortAddress(job.sponsor_address)
      );
    default:
      return queueKindLabel(job.kind || "unknown");
  }
}

/** Ported from the legacy queue view (W7.4 semantics): which jobs a manual
 * Process may re-drive. */
export function queueJobCanProcess(job: QueueJob): boolean {
  const state = String(job.state || "");
  if (state === "sent" && job.kind === "plan_step_execution") {
    return true;
  }
  return ![
    "operator_action_required",
    "sent",
    "confirmed",
    "failed",
    "failed_terminal",
  ].includes(state);
}

export interface QueueJobGroup {
  state: string;
  jobs: QueueJob[];
}

/** Jobs grouped by state, worst first; empty groups dropped. */
export function groupQueueJobs(jobs: QueueJob[]): QueueJobGroup[] {
  const byState = new Map<string, QueueJob[]>();
  for (const job of jobs) {
    const state = job.state || "unknown";
    const group = byState.get(state) ?? [];
    group.push(job);
    byState.set(state, group);
  }
  const ordered: QueueJobGroup[] = [];
  for (const state of QUEUE_STATE_ORDER) {
    const group = byState.get(state);
    if (group?.length) ordered.push({ state, jobs: group });
    byState.delete(state);
  }
  for (const [state, group] of Array.from(byState)) {
    ordered.push({ state, jobs: group });
  }
  return ordered;
}

/** Best-effort human lead for a daemon queue error string. */
export function humanizeQueueError(error: string): string {
  const text = error.trim();
  if (/^insufficient_gas/i.test(text)) {
    return "Not enough gas — " + text;
  }
  if (/^(policy_block|policy_violation)/i.test(text)) {
    return "Blocked by treasury policy — " + text;
  }
  if (/^provider_error/i.test(text)) {
    return "Provider problem — " + text;
  }
  if (/^on_chain_revert/i.test(text)) {
    return "Reverted on-chain — " + text;
  }
  if (/^broadcast_rejected/i.test(text)) {
    return "Broadcast rejected — " + text;
  }
  if (/^receipt_timeout/i.test(text)) {
    return "Confirmation timed out — " + text;
  }
  return text;
}

/** Humanized `POST /api/queue/process` tally (also used per-job). */
export function queueProcessSummary(payload: {
  processed?: number;
  succeeded?: number;
  blocked?: number;
  retrying?: number;
  operator_action_required?: number;
  failed?: number;
  confirmed?: number;
  failures_by_cause?: unknown;
  paused_reason?: string | null;
}): string {
  const parts: string[] = [];
  if (payload.succeeded) parts.push(String(payload.succeeded) + " succeeded");
  if (payload.confirmed) parts.push(String(payload.confirmed) + " confirmed");
  if (payload.blocked) parts.push(String(payload.blocked) + " blocked");
  if (payload.retrying) parts.push(String(payload.retrying) + " will retry");
  if (payload.operator_action_required) {
    parts.push(String(payload.operator_action_required) + " need your action");
  }
  if (payload.failed) parts.push(String(payload.failed) + " failed");
  let text =
    "Processed " +
    String(payload.processed || 0) +
    " job(s)" +
    (parts.length ? ": " + parts.join(", ") + "." : ".");
  const breakdown = payload.failures_by_cause as
    | Record<string, unknown>
    | null
    | undefined;
  if (breakdown) {
    const labels: [string, string][] = [
      ["provider_error", "provider errors"],
      ["policy_block", "policy blocks"],
      ["insufficient_gas", "gas shortfalls"],
      ["validation", "validation failures"],
      ["on_chain_revert", "on-chain reverts"],
      ["broadcast_rejected", "broadcast rejections"],
      ["receipt_timeout", "receipt timeouts"],
      ["unknown", "unknown causes"],
    ];
    const causes = labels
      .filter(([key]) => Number(breakdown[key] || 0) > 0)
      .map(([key, label]) => Number(breakdown[key]) + " " + label);
    if (causes.length) text += " Failure causes: " + causes.join(", ") + ".";
  }
  if (payload.paused_reason) {
    text += " Paused: " + String(payload.paused_reason) + ".";
  }
  return text;
}

/** Humanized `POST /api/maintenance/run` result. */
export function maintenanceSummary(payload: DaemonPayload): string {
  if (payload.status === "canceled") {
    return "Maintenance cycle canceled between stages — completed stages keep their effects.";
  }
  const parts: string[] = [];
  parts.push(
    "refreshed " + String(payload.refreshed || 0) + " deposit(s)",
    "detected " + String(payload.detected || 0),
    "enqueued " + String(payload.queued || 0) + " sweep(s)",
  );
  let text = "Cycle complete: " + parts.join(", ") + ".";
  if (payload.processed != null) {
    text += " " + queueProcessSummary(payload as Parameters<typeof queueProcessSummary>[0]);
  }
  const automation = payload.treasury_automation as
    | { generated_steps?: number; enqueued_steps?: number; skipped_steps?: number }
    | undefined;
  if (automation) {
    text +=
      " Treasury automation drafted " +
      String(automation.generated_steps || 0) +
      " step(s) and enqueued " +
      String(automation.enqueued_steps || 0) +
      ".";
  }
  return text;
}

// ── Thin API wrappers (endpoints the typed core client lacks) ───────────
// Same envelope convention as core/api.ts: `{ error, code, action, fields }`.
// These return the raw payload so the enqueue probe/phrase flow keeps the
// legacy server contract exactly; callers branch with payloadFailure().

const moveApi = {
  generatePlan: (body: ConsolidationPlanGenerateRequest): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/consolidation/generate", body),
  approvePlan: (planId: string): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/consolidation/approve", {
      plan_id: planId,
      step_ids: [],
    }),
  simulatePlan: (planId: string): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/consolidation/simulate", {
      plan_id: planId,
      step_ids: [],
    }),
  exportPlan: (
    planId: string,
    format: string,
    safeAddress: string | null,
  ): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/consolidation/export", {
      plan_id: planId,
      step_ids: [],
      format,
      safe_address: safeAddress,
    }),
  enqueueStep: (planId: string, stepId: string): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/enqueue-step", {
      plan_id: planId,
      step_id: stepId,
      confirm: true,
    }),
  enqueuePlan: (planId: string, confirmation: string): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/plans/enqueue-plan", {
      plan_id: planId,
      confirmation,
    }),
  listParties: (): Promise<DaemonPayload> =>
    requestWithSession("GET", "/api/treasury/parties"),
  listChains: (): Promise<DaemonPayload> =>
    requestWithSession("GET", "/api/chains"),
  runMaintenance: (body: Record<string, unknown>): Promise<DaemonPayload> =>
    requestWithSession("POST", "/api/maintenance/run", body),
};

/** Error envelope → ApiFailure (null when the payload is not an error). */
function payloadFailure(payload: DaemonPayload): ApiFailure | null {
  if (payload == null || payload.error == null) return null;
  return {
    code: payload.code ?? "unknown",
    error: String(payload.error),
    action: typeof payload.action === "string" ? payload.action : undefined,
    fields: payload.fields,
  };
}

/** Thrown error → ApiFailure (synthesizes `unavailable` for network errors). */
function thrownFailure(error: unknown): ApiFailure {
  return (
    apiFailure(error) ?? {
      code: "unavailable",
      error: error instanceof Error ? error.message : String(error),
    }
  );
}

// ── The controller ───────────────────────────────────────────────────────

type MoveCardId = "plansCard" | "queueCard" | "policyCard" | "maintenanceCard";
const MOVE_CARD_IDS: MoveCardId[] = [
  "plansCard",
  "queueCard",
  "policyCard",
  "maintenanceCard",
];
/** Cards concealed while the plan review screen has focus. */
const AUX_CARD_IDS: MoveCardId[] = ["queueCard", "policyCard", "maintenanceCard"];

interface MoveNotice {
  message: string;
  tone: NoticeTone;
}

interface MoveState {
  mounted: boolean;
  mode: "list" | "detail";
  detailPlanId: string | null;
  plans: ConsolidationPlan[] | null;
  plansPagination: PaginationInfo | null;
  plansOffset: number;
  plansFailure: ApiFailure | null;
  plansLoading: boolean;
  queue: QueueJob[] | null;
  queuePagination: PaginationInfo | null;
  queueOffset: number;
  queueFailure: ApiFailure | null;
  queueLoading: boolean;
  policy: TreasuryPolicy | null;
  policyLoaded: boolean;
  policyFailure: ApiFailure | null;
  parties: Counterparty[];
  chains: ChainProfile[];
  planNotice: MoveNotice | null;
  queueNotice: MoveNotice | null;
  policyNotice: MoveNotice | null;
  maintenanceNotice: MoveNotice | null;
}

interface MoveDom {
  cards: Partial<Record<MoveCardId, HTMLElement>>;
  savedHtml: Partial<Record<MoveCardId, string>>;
  plansBody: HTMLElement | null;
}

export function createMoveDestination(
  runtime: CoreRuntime,
): DestinationController {
  const state: MoveState = {
    mounted: false,
    mode: "list",
    detailPlanId: null,
    plans: null,
    plansPagination: null,
    plansOffset: 0,
    plansFailure: null,
    plansLoading: false,
    queue: null,
    queuePagination: null,
    queueOffset: 0,
    queueFailure: null,
    queueLoading: false,
    policy: null,
    policyLoaded: false,
    policyFailure: null,
    parties: [],
    chains: [],
    planNotice: null,
    queueNotice: null,
    policyNotice: null,
    maintenanceNotice: null,
  };
  const dom: MoveDom = { cards: {}, savedHtml: {}, plansBody: null };
  const unsubscribes: Unsubscribe[] = [];
  // Policy-editor session state (survives re-renders within one mount).
  let policyFormFingerprint: string | null = null;
  let policyFormDirty = false;
  let activePreset: PolicyPreset["id"] = "custom";
  // Coalescing guards for store-driven refetches (no timers anywhere).
  let plansFetching = false;
  let plansFetchAgain = false;
  let queueFetching = false;
  let queueFetchAgain = false;
  // Detail-view content fingerprint: the review screen is rebuilt only when
  // the plan/policy content actually changes, preserving in-progress input.
  let detailFingerprint: string | null = null;

  const nowSecs = (): number => Math.floor(Date.now() / 1000);

  function nativeSymbol(chainId: number | null | undefined): string {
    const profile = state.chains.find(
      (chain) => chain.enabled && chain.chain_id === chainId,
    );
    return profile?.native_symbol || "ETH";
  }

  function chainName(chainId: number | null | undefined): string {
    return chainLabel(chainId, state.chains);
  }

  // ── Shared regions ─────────────────────────────────────────────────

  function renderNotice(
    region: HTMLElement | null,
    notice: MoveNotice | null,
  ): void {
    if (!region) return;
    clearChildren(region);
    if (!notice) {
      region.classList.add("hidden");
      return;
    }
    region.classList.remove("hidden");
    region.dataset.tone = notice.tone;
    region.setAttribute(
      "role",
      notice.tone === "error" || notice.tone === "warning" ? "alert" : "status",
    );
    region.appendChild(
      el("p", { class: "move-notice-text", text: notice.message }),
    );
  }

  /** Persistent stale-data banner (NOT a vanishing toast): shown when a
   * refresh fails while earlier data is still on screen. */
  function staleBanner(resource: string, retry: () => void): HTMLElement {
    return el(
      "div",
      { class: "move-banner", dataset: { tier: "review" } },
      el("p", {
        class: "move-banner-text",
        text:
          "Couldn't refresh " +
          resource +
          " — what you see may be out of date.",
      }),
      el("button", {
        class: "btn-ghost btn-small",
        attrs: { type: "button" },
        text: "Retry",
        on: { click: () => retry() },
      }),
    );
  }

  /** First-load failure panel, driven by the daemon's error code. */
  function failurePanel(failure: ApiFailure, retry: () => void): HTMLElement {
    let title = "Couldn't load";
    let body = failure.error || "Something went wrong.";
    let tier: Tier = "danger";
    let vaultLink = false;
    switch (failure.code) {
      case "vault_locked":
        title = "The vault is locked";
        body = "Unlock the vault to see plans, the queue, and the treasury policy.";
        tier = "review";
        vaultLink = true;
        break;
      case "unauthorized":
      case "forbidden":
        title = "The session has expired";
        body = "Unlock again to continue.";
        tier = "review";
        vaultLink = true;
        break;
      case "not_initialized":
        title = "No vault yet";
        body = "Create a vault before moving funds.";
        tier = "review";
        break;
      case "unavailable":
        title = "Can't reach the daemon";
        body = failure.error || "The daemon did not answer.";
        break;
      default:
        break;
    }
    return el(
      "div",
      { class: "move-failure", dataset: { tier } },
      el("p", { class: "move-failure-title", text: title }),
      el("p", { class: "move-failure-body", text: body }),
      el(
        "div",
        { class: "move-failure-actions" },
        vaultLink
          ? el("a", {
              class: "btn-primary move-btn-link",
              attrs: { href: formatHash("vault") },
              text: "Go to Vault",
            })
          : null,
        el("button", {
          class: "btn-ghost",
          attrs: { type: "button" },
          text: "Try again",
          on: { click: () => retry() },
        }),
      ),
    );
  }

  function skeletonBlock(rows: number): HTMLElement {
    const wrap = el("div", {
      class: "move-skeletons",
      attrs: { "aria-hidden": "true" },
    });
    for (let index = 0; index < rows; index++) {
      wrap.appendChild(
        el("div", { class: "skeleton skeleton-block move-skeleton-row" }),
      );
    }
    return wrap;
  }

  function sectionEmpty(
    title: string,
    body: string,
    actionLabel: string | null,
    onAction: (() => void) | null,
  ): HTMLElement {
    return el(
      "div",
      { class: "section-empty" },
      el("p", { class: "section-empty-title", text: title }),
      el("p", { class: "section-empty-body", text: body }),
      actionLabel && onAction
        ? el("button", {
            class: "btn-primary",
            attrs: { type: "button" },
            text: actionLabel,
            on: { click: () => onAction() },
          })
        : null,
    );
  }

  // ── Data loading ───────────────────────────────────────────────────

  async function refreshPlans(): Promise<void> {
    if (plansFetching) {
      plansFetchAgain = true;
      return;
    }
    plansFetching = true;
    if (state.plans === null) {
      state.plansLoading = true;
      renderPlansCard();
    }
    try {
      // The review screen needs the full list (deep links may point at any
      // plan); the list view pages with the 1.5 query params.
      const response =
        state.mode === "detail"
          ? await runtime.api.listPlans()
          : await runtime.api.listPlans({
              limit: PLANS_PAGE_SIZE,
              offset: state.plansOffset,
              sort: "created",
              order: "desc",
            });
      state.plans = response.plans || [];
      state.plansPagination = response.pagination ?? null;
      state.plansFailure = null;
    } catch (error) {
      state.plansFailure = thrownFailure(error);
    }
    state.plansLoading = false;
    plansFetching = false;
    renderPlansCard();
    if (plansFetchAgain) {
      plansFetchAgain = false;
      void refreshPlans();
    }
  }

  async function refreshQueue(): Promise<void> {
    if (queueFetching) {
      queueFetchAgain = true;
      return;
    }
    queueFetching = true;
    if (state.queue === null) {
      state.queueLoading = true;
      renderQueueCard();
    }
    try {
      const response = await runtime.api.listQueueJobs({
        limit: QUEUE_PAGE_SIZE,
        offset: state.queueOffset,
        sort: "updated",
        order: "desc",
      });
      state.queue = response.jobs || [];
      state.queuePagination = response.pagination ?? null;
      state.queueFailure = null;
    } catch (error) {
      state.queueFailure = thrownFailure(error);
    }
    state.queueLoading = false;
    queueFetching = false;
    renderQueueCard();
    if (queueFetchAgain) {
      queueFetchAgain = false;
      void refreshQueue();
    }
  }

  async function refreshPolicy(): Promise<void> {
    try {
      const response = await runtime.api.getTreasuryPolicy();
      state.policy = response.policy;
      state.policyLoaded = true;
      state.policyFailure = null;
    } catch (error) {
      state.policyFailure = thrownFailure(error);
    }
    renderPolicyCard();
    renderQueueCard();
    // Eligibility mirrors policy gates — plan affordances follow policy.
    if (state.plans !== null) renderPlansCard();
  }

  async function refreshPartiesAndChains(): Promise<void> {
    try {
      const payload = await moveApi.listParties();
      if (!payloadFailure(payload)) {
        state.parties = (payload.parties as Counterparty[]) || [];
      }
    } catch (_) {
      /* trust chips fall back to "foreign" */
    }
    try {
      const payload = await moveApi.listChains();
      if (!payloadFailure(payload)) {
        state.chains = (payload.chains as ChainProfile[]) || [];
      }
    } catch (_) {
      /* chain labels fall back to "Chain N" */
    }
  }

  function refreshAll(): void {
    void refreshPlans();
    void refreshQueue();
    void refreshPolicy();
    void refreshPartiesAndChains().then(() => {
      syncPartyRegionFn?.();
      renderPlansCard();
      renderPolicyCard();
    });
  }

  // ── Plans card: list mode ──────────────────────────────────────────

  function renderPlansCard(): void {
    const body = dom.plansBody;
    if (!body || !state.mounted) return;
    if (state.mode === "detail") {
      renderPlanDetail(body);
    } else {
      renderPlanList(body);
    }
  }

  interface PlanListShell {
    banner: HTMLElement;
    generate: HTMLElement;
    list: HTMLElement;
    pagination: HTMLElement;
  }

  // Shell refs are closure state (the fake-DOM harness does not index
  // generated elements, and real-DOM queries would be churn): rebuilt only
  // when the mode flips, so form inputs keep their content across refreshes.
  let planListShell: PlanListShell | null = null;

  function ensurePlanListShell(body: HTMLElement): PlanListShell {
    if (planListShell) return planListShell;
    clearChildren(body);
    const banner = el("div", {
      attrs: { role: "status", "data-move-region": "plans-banner" },
    });
    const generate = el("div", {
      attrs: { "data-move-region": "plans-generate" },
    });
    const list = el("div", { attrs: { "data-move-region": "plans-list" } });
    const pagination = el("div", {
      attrs: { "data-move-region": "plans-pagination" },
    });
    const shell = el(
      "div",
      { class: "move-root" },
      buildPageHeader(),
      banner,
      generate,
      list,
      pagination,
    );
    body.appendChild(shell);
    buildGenerateSection(generate);
    planListShell = { banner, generate, list, pagination };
    return planListShell;
  }

  function buildPageHeader(): HTMLElement {
    return el(
      "div",
      { class: "page-header" },
      el(
        "div",
        null,
        el("h2", { class: "page-header-title", text: "Move" }),
        el("p", {
          class: "page-header-summary",
          text:
            "Review consolidation plans step by step — like a hardware-wallet confirmation — then approve once. The queue below runs what you approve; the treasury policy decides what may run at all.",
        }),
      ),
      el(
        "div",
        { class: "page-header-actions" },
        el("button", {
          class: "btn-ghost btn-small",
          attrs: { type: "button" },
          text: "Refresh",
          on: { click: () => refreshAll() },
        }),
      ),
    );
  }

  function renderPlanList(body: HTMLElement): void {
    detailFingerprint = null; // leaving the review screen: next entry rebuilds
    const shell = ensurePlanListShell(body);

    // Stale banner: refresh failed with earlier data still on screen.
    clearChildren(shell.banner);
    if (state.plansFailure && state.plans !== null) {
      shell.banner.appendChild(staleBanner("plans", () => void refreshPlans()));
    }

    clearChildren(shell.list);
    if (state.plansLoading && state.plans === null) {
      shell.list.appendChild(skeletonBlock(3));
    } else if (state.plansFailure && state.plans === null) {
      shell.list.appendChild(
        failurePanel(state.plansFailure, () => void refreshPlans()),
      );
    } else if ((state.plans || []).length === 0) {
      shell.list.appendChild(
        sectionEmpty(
          "No consolidation plans yet",
          "Plans are dry-run until you review and approve them. Generate one to see exactly what would move, where to, and at what fee — nothing executes from a plan without your typed confirmation.",
          "Generate a plan",
          () => openGenerateForm(),
        ),
      );
    } else {
      const listEl = el("div", { class: "move-plan-list" });
      shell.list.appendChild(listEl);
      renderList<ConsolidationPlan>(
        listEl,
        state.plans || [],
        (plan) => plan.id,
        renderPlanRow,
      );
    }

    renderPlanPagination(shell.pagination);
  }

  function renderPlanPagination(container: HTMLElement): void {
    clearChildren(container);
    const info = state.plansPagination;
    if (!info || info.total <= PLANS_PAGE_SIZE) return;
    const page = Math.floor(info.offset / PLANS_PAGE_SIZE) + 1;
    const pages = Math.max(1, Math.ceil(info.total / PLANS_PAGE_SIZE));
    const prev = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "← Newer",
      on: {
        click: () => {
          state.plansOffset = Math.max(0, state.plansOffset - PLANS_PAGE_SIZE);
          void refreshPlans();
        },
      },
    }) as HTMLButtonElement;
    prev.disabled = info.offset <= 0;
    const next = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "Older →",
      on: {
        click: () => {
          state.plansOffset = state.plansOffset + PLANS_PAGE_SIZE;
          void refreshPlans();
        },
      },
    }) as HTMLButtonElement;
    next.disabled = !info.has_more;
    container.appendChild(
      el(
        "div",
        { class: "move-pagination" },
        prev,
        el("span", {
          class: "move-pagination-label nums",
          text: "Page " + page + " of " + pages + " · " + info.total + " plans",
        }),
        next,
      ),
    );
  }

  function renderPlanRow(
    plan: ConsolidationPlan,
    existing: HTMLElement | null,
  ): HTMLElement {
    const now = nowSecs();
    const eligible = (plan.steps || []).filter((step) =>
      stepExecutionEligible(step, state.policy, now),
    ).length;
    const fingerprint = [
      plan.id,
      plan.status,
      plan.updated_at_unix,
      JSON.stringify(plan.summary || {}),
      eligible,
      (plan.linkage_findings || []).length,
      state.chains.length,
      state.parties.length,
    ].join("|");
    return patchKeyedRow(existing, "move-plan-row", fingerprint, (row) => {
      row.dataset.tier =
        plan.status === "blocked"
          ? "danger"
          : plan.status === "review_required"
            ? "review"
            : "quiet";
      const main = el(
        "div",
        { class: "move-plan-row-main" },
        el(
          "p",
          { class: "move-plan-row-title" },
          el("span", { text: "Plan " + shortId(plan.id) + " " }),
          pill(plan.status),
        ),
        el(
          "p",
          { class: "move-plan-row-summary" },
          el("span", {
            class: "nums",
            text: planCountsLine(plan, nativeSymbol(plan.chain_id)),
          }),
          " · " + chainName(plan.chain_id) + " · created ",
          timeEl(plan.created_at_unix, now),
          eligible ? " · " + eligible + " ready to enqueue" : "",
        ),
        (plan.linkage_findings || []).length
          ? el("p", {
              class: "move-plan-row-warning",
              text: "Privacy: this plan would link payers — review before approving.",
            })
          : null,
      );
      const review = el("a", {
        class: "btn-primary move-btn-link",
        attrs: {
          href: formatHash("move", "plan", plan.id),
          "aria-label": "Review plan " + plan.id,
        },
        text: "Review",
      });
      row.appendChild(main);
      row.appendChild(el("div", { class: "move-plan-row-actions" }, review));
    });
  }

  // ── Plan generation ────────────────────────────────────────────────

  // Closure refs into the generate form (valid until the shell is rebuilt).
  let generateDetailsEl: (HTMLElement & { open?: boolean }) | null = null;
  let generateDestinationInput: HTMLInputElement | null = null;
  let partyDestinationInputs: { partyId: string; input: HTMLInputElement }[] =
    [];
  let syncPartyRegionFn: (() => void) | null = null;

  function openGenerateForm(): void {
    if (generateDetailsEl) generateDetailsEl.open = true;
    generateDestinationInput?.focus?.();
  }

  function buildGenerateSection(container: HTMLElement): void {
    clearChildren(container);
    partyDestinationInputs = [];
    const errorRegion = el("div", {
      class: "move-field-errors hidden",
      attrs: { role: "alert", "data-move-region": "generate-errors" },
    });

    const routing = el(
      "select",
      { class: "input-wide" },
      el("option", { text: "Single destination", attrs: { value: "single" } }),
      el("option", {
        text: "Per-party (isolate payers)",
        attrs: { value: "per_party" },
      }),
    ) as HTMLSelectElement;

    const partyRegion = el("div", {
      class: "move-party-destinations hidden",
      attrs: { "data-move-region": "party-destinations" },
    });
    const partyHint = el("p", {
      class: "field-hint hidden",
      text:
        "Per-party routing sends each payer's holdings to its own destination so consolidation does not link payers. The single destination below is used only for unattributed holdings (your own hot/change addresses).",
      attrs: { "data-move-region": "party-hint" },
    });

    const syncPartyRegion = (): void => {
      const perParty = routing.value === "per_party";
      setHidden(partyRegion, !perParty);
      setHidden(partyHint, !perParty);
      if (!perParty) return;
      clearChildren(partyRegion);
      partyDestinationInputs = [];
      if (!state.parties.length) {
        partyRegion.appendChild(
          el("p", {
            class: "field-hint",
            text: "No counterparties yet — per-party routing needs parties with sweep destinations.",
          }),
        );
        return;
      }
      for (const party of state.parties) {
        const input = el("input", {
          class: "input-wide move-party-destination",
          attrs: {
            type: "text",
            placeholder: "0x… destination for " + (party.name || party.id),
            autocomplete: "off",
          },
        }) as HTMLInputElement;
        input.value = party.sweep_destination_address || "";
        partyDestinationInputs.push({ partyId: party.id, input });
        partyRegion.appendChild(
          el(
            "label",
            { class: "field-label move-party-field" },
            el("span", { text: party.name || party.id }),
            input,
          ),
        );
      }
    };
    routing.addEventListener("change", syncPartyRegion);
    syncPartyRegionFn = syncPartyRegion;

    const destination = el("input", {
      class: "input-wide",
      attrs: {
        type: "text",
        placeholder: "0x… consolidation destination",
        autocomplete: "off",
        "data-move-region": "generate-destination",
      },
    }) as HTMLInputElement;
    generateDestinationInput = destination;
    const chainId = el("input", {
      class: "input-mid",
      attrs: {
        type: "number",
        placeholder: "All chains",
        min: "1",
        "data-move-region": "generate-chain",
      },
    }) as HTMLInputElement;

    const submit = el("button", {
      class: "btn-primary",
      attrs: { type: "submit" },
      text: "Generate plan",
    }) as HTMLButtonElement;

    const form = el(
      "form",
      { class: "move-generate-form" },
      el(
        "div",
        { class: "form-row" },
        el(
          "label",
          { class: "field-label" },
          el("span", { text: "Routing" }),
          routing,
        ),
      ),
      partyHint,
      partyRegion,
      el(
        "div",
        { class: "form-row" },
        el(
          "label",
          { class: "field-label move-grow" },
          el("span", { text: "Destination address" }),
          destination,
        ),
        el(
          "label",
          { class: "field-label" },
          el("span", { text: "Chain (optional)" }),
          chainId,
        ),
      ),
      errorRegion,
      el("div", { class: "form-row" }, submit),
    );
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void submitGenerateForm({
        routing,
        destination,
        chainId,
        submit,
        errorRegion,
      });
    });

    const details = el(
      "details",
      { class: "move-generate" },
      el("summary", { text: "Generate a new plan" }),
      el("p", {
        class: "helper-text",
        text:
          "Generation is a dry-run: it scans tracked holdings and drafts steps. Nothing moves until you review the plan and confirm with a typed phrase.",
      }),
      form,
    ) as HTMLElement & { open?: boolean };
    generateDetailsEl = details;
    container.appendChild(details);
    syncPartyRegion();
  }

  async function submitGenerateForm(form: {
    routing: HTMLSelectElement;
    destination: HTMLInputElement;
    chainId: HTMLInputElement;
    submit: HTMLButtonElement;
    errorRegion: HTMLElement;
  }): Promise<void> {
    clearChildren(form.errorRegion);
    form.errorRegion.classList.add("hidden");
    form.destination.classList.remove("input-invalid");
    form.chainId.classList.remove("input-invalid");

    const routingStrategy =
      form.routing.value === "per_party" ? "per_party" : "single";
    const body: ConsolidationPlanGenerateRequest = {
      destination_address: form.destination.value.trim() || null,
      include_watch_only: true,
      auto_queue_low_risk: false,
      routing_strategy: routingStrategy,
    };
    const chainText = form.chainId.value.trim();
    if (chainText) {
      const parsed = parseInt(chainText, 10);
      if (!Number.isFinite(parsed) || parsed <= 0) {
        form.chainId.classList.add("input-invalid");
        showFormErrors(form.errorRegion, [
          "Chain must be a positive chain id, or leave it empty for all chains.",
        ]);
        return;
      }
      body.chain_id = parsed;
    }
    if (routingStrategy === "per_party") {
      const destinations: PartyDestination[] = [];
      for (const entry of partyDestinationInputs) {
        const value = entry.input.value.trim();
        if (!value) continue;
        destinations.push({
          counterparty_id: entry.partyId,
          destination_address: value,
        });
      }
      body.party_destinations = destinations;
    }

    setBusy(form.submit, true);
    try {
      const payload = await moveApi.generatePlan(body);
      const failure = payloadFailure(payload);
      if (failure) {
        if (failure.code === "validation_failed" && failure.fields?.length) {
          markGenerateFieldErrors(failure.fields, form);
        }
        showFailureAsNotice("planNotice", failure);
        return;
      }
      state.planNotice = {
        message: "Dry-run plan generated — review it below.",
        tone: "success",
      };
      renderPlansCard();
      state.plansOffset = 0;
      await refreshPlans();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(form.submit, false);
    }
  }

  function showFormErrors(region: HTMLElement, messages: string[]): void {
    clearChildren(region);
    region.classList.remove("hidden");
    for (const message of messages) {
      region.appendChild(el("p", { class: "move-field-error", text: message }));
    }
  }

  function markGenerateFieldErrors(
    fields: FieldError[],
    form: {
      destination: HTMLInputElement;
      chainId: HTMLInputElement;
      errorRegion: HTMLElement;
    },
  ): void {
    const messages: string[] = [];
    for (const field of fields) {
      if (field.field.startsWith("destination_address")) {
        form.destination.classList.add("input-invalid");
      } else if (field.field.startsWith("chain_id")) {
        form.chainId.classList.add("input-invalid");
      }
      messages.push(field.message);
    }
    showFormErrors(form.errorRegion, messages);
  }

  function showFailureAsNotice(
    slot: "planNotice" | "queueNotice" | "policyNotice" | "maintenanceNotice",
    failure: ApiFailure,
  ): void {
    let message = failure.error || "Something went wrong.";
    if (failure.code === "vault_locked") {
      message = "The vault is locked — unlock it in the Vault section, then try again.";
    } else if (failure.code === "execution_gate_denied") {
      message =
        "Denied by the execution gates: " +
        message +
        " Check the treasury policy gates below.";
    } else if (failure.code === "unavailable") {
      message = "The daemon did not answer — " + message;
    }
    state[slot] = { message, tone: "error" };
    renderPlansCard();
    renderQueueCard();
    renderPolicyCard();
    renderMaintenanceCard();
  }

  // ── Plans card: the plan review screen (detail mode) ───────────────

  function renderPlanDetail(body: HTMLElement): void {
    // Rebuild only when the rendered content actually changed: the review
    // screen holds in-progress input (the Safe address field), and live
    // refreshes must not clobber it.
    const plan = (state.plans || []).find(
      (candidate) => candidate.id === state.detailPlanId,
    );
    const fingerprint = state.plansLoading
      ? "loading"
      : state.plansFailure
        ? "failure:" + state.plansFailure.code + ":" + state.plansFailure.error
        : plan
          ? JSON.stringify(plan) +
            "|" +
            JSON.stringify(state.policy || null) +
            "|" +
            state.parties.length +
            "|" +
            state.chains.length +
            "|" +
            (state.planNotice?.message ?? "")
          : "not-found:" + String(state.detailPlanId);
    if (fingerprint === detailFingerprint) return;
    detailFingerprint = fingerprint;
    planListShell = null; // the list shell (if any) is destroyed below

    clearChildren(body);
    const root = el(
      "div",
      {
        class: "move-root move-detail",
        attrs: { "data-move-region": "plan-detail" },
      },
      el(
        "p",
        { class: "move-back-row" },
        el("a", {
          class: "move-back",
          attrs: { href: formatHash("move") },
          text: "← All plans",
        }),
      ),
    );
    body.appendChild(root);

    if (state.plansLoading && state.plans === null) {
      root.appendChild(skeletonBlock(4));
      return;
    }
    if (state.plansFailure && state.plans === null) {
      root.appendChild(
        failurePanel(state.plansFailure, () => void refreshPlans()),
      );
      return;
    }
    if (!plan) {
      root.appendChild(
        sectionEmpty(
          "Plan not found",
          "This plan is not on the daemon anymore (or the link is wrong). It may have been replaced by a newer dry-run.",
          "Back to plans",
          () => runtime.router.navigate(formatHash("move")),
        ),
      );
      return;
    }

    const now = nowSecs();
    const symbol = nativeSymbol(plan.chain_id);
    const steps = plan.steps || [];
    const eligibleSteps = steps.filter((step) =>
      stepExecutionEligible(step, state.policy, now),
    );
    const totalWeiHex = planNativeTotalWeiHex(plan);

    // ── Header: what this plan would do, at a glance ──
    root.appendChild(
      el(
        "div",
        { class: "move-detail-head" },
        el(
          "p",
          { class: "move-detail-title" },
          el("span", { text: "Plan " + shortId(plan.id) + " " }),
          pill(plan.status),
        ),
        el(
          "p",
          { class: "move-detail-totals" },
          el("span", {
            class: "nums",
            text:
              steps.length +
              " steps" +
              (totalWeiHex
                ? " · moving up to " +
                  (formatWeiHexAsEth(totalWeiHex) ?? "?") +
                  " " +
                  symbol
                : ""),
          }),
          " · " + chainName(plan.chain_id) + " · created ",
          timeEl(plan.created_at_unix, now),
        ),
        el("p", {
          class: "move-detail-guide",
          text:
            "Read every step like a hardware-wallet confirmation: what moves, from where, to whom, at what fee. When each step reads right, one typed approval enqueues the eligible steps.",
        }),
      ),
    );

    // ── Plan-level warnings ──
    if ((plan.policy_violations || []).length) {
      root.appendChild(
        el(
          "div",
          { class: "move-banner", dataset: { tier: "danger" } },
          el("p", {
            class: "move-banner-text",
            text:
              "Policy violations: " + (plan.policy_violations || []).join(" · "),
          }),
        ),
      );
    }
    if ((plan.linkage_findings || []).length) {
      root.appendChild(
        el(
          "div",
          { class: "move-banner", dataset: { tier: "review" } },
          el("p", {
            class: "move-banner-text",
            text:
              "Privacy: this plan would link payers — " +
              (plan.linkage_findings || []).join(" · "),
          }),
          el(
            "details",
            { class: "move-banner-details" },
            el("summary", { text: "What this covers" }),
            el("p", {
              class: "field-hint",
              text:
                "Flags payers that would sweep to the same destination. Sigillum-generated gas top-ups are checked the same way: one sponsor funding different payers warns, and is blocked when linkage protection is on. Manual gas funding, amount/timing correlation, downstream re-merging, and multi-hop flows remain operator discipline.",
            }),
          ),
        ),
      );
    }

    const notice = el("div", {
      class: "move-notice",
      attrs: { role: "status", "data-move-region": "plan-notice" },
    });
    root.appendChild(notice);
    renderNotice(notice, state.planNotice);

    // ── Per-step cards ──
    const stepsEl = el("ol", {
      class: "move-steps",
      attrs: { "data-move-region": "plan-steps" },
    });
    root.appendChild(stepsEl);
    steps.forEach((step, index) => {
      stepsEl.appendChild(renderStepCard(plan, step, index, symbol, now));
    });

    // ── The one deliberate approve action + secondary tooling ──
    root.appendChild(buildPlanActionBar(plan, eligibleSteps.length, symbol));
  }

  function renderStepCard(
    plan: ConsolidationPlan,
    step: ConsolidationPlanStep,
    index: number,
    symbol: string,
    now: number,
  ): HTMLElement {
    const policy = state.policy;
    const freshness =
      Number(policy?.simulation_freshness_secs ?? DEFAULT_SIM_FRESHNESS_SECS) ||
      DEFAULT_SIM_FRESHNESS_SECS;
    const badge = simulationBadge(step, freshness, now);
    const eligible = stepExecutionEligible(step, policy, now);
    const isValueMoving =
      step.action === "sweep_native" ||
      step.action === "sweep_erc20" ||
      step.action === "sweep_nft" ||
      step.action === "fund_gas";
    const trust = isValueMoving
      ? destinationTrust(step.destination_address, policy, state.parties)
      : null;
    const feeCapWeiHex = stepFeeCapWeiHex(step);
    const { maxFeePerGasWeiHex } = stepGasInfo(step);
    const tier: Tier = (step.blockers || []).length
      ? "danger"
      : step.status === "review_required"
        ? "review"
        : "quiet";

    const chips = el(
      "div",
      { class: "move-step-chips" },
      pill(step.status),
      tierChip(badge.text, badge.tier),
      trust ? tierChip(trust.label, trust.tier) : null,
      feeCapWeiHex
        ? el("span", {
            class: "move-chip nums",
            dataset: { tier: "quiet" },
            text:
              "fee ≤ " +
              (formatWeiHexAsEth(feeCapWeiHex) ?? "?") +
              " " +
              symbol +
              (maxFeePerGasWeiHex
                ? " · " + (formatWeiHexAsGwei(maxFeePerGasWeiHex) ?? "?") + " gwei"
                : ""),
          })
        : null,
      step.signer_status === "watch_only"
        ? tierChip("watch-only", "review")
        : null,
    );

    const card = el(
      "li",
      { class: "move-step", dataset: { tier } },
      el(
        "div",
        { class: "move-step-head" },
        el("span", { class: "move-step-num nums", text: String(index + 1) }),
        el("p", {
          class: "move-step-title",
          text: stepPlainLanguage(step, {
            symbol,
            policy,
            parties: state.parties,
          }),
        }),
      ),
      chips,
    );

    if ((step.blockers || []).length) {
      card.appendChild(
        el(
          "ul",
          { class: "move-step-blockers" },
          ...(step.blockers || []).map((blocker) =>
            el("li", { text: "Blocked: " + blockerLabel(blocker) }),
          ),
        ),
      );
    }
    if ((step.linkage_warnings || []).length) {
      card.appendChild(
        el(
          "ul",
          { class: "move-step-warnings" },
          ...(step.linkage_warnings || []).map((warning) =>
            el("li", { text: "Privacy: " + warning }),
          ),
        ),
      );
    }
    if (step.queued_job_id) {
      card.appendChild(
        el(
          "p",
          { class: "move-step-queued" },
          "Queued as job " + shortId(step.queued_job_id) + " — ",
          el("a", {
            class: "move-inline-link",
            attrs: { href: formatHash("move", "queue") },
            text: "see the queue",
          }),
        ),
      );
    }

    // Raw values stay one click away, never in the default view.
    card.appendChild(buildStepDetails(step));

    if (eligible) {
      card.appendChild(
        el(
          "div",
          { class: "move-step-actions" },
          el("button", {
            class: "btn-ghost btn-small",
            attrs: { type: "button" },
            text: "Enqueue this step",
            on: {
              click: (event) => {
                void enqueueSingleStep(
                  plan.id,
                  step.id,
                  event.currentTarget as HTMLElement,
                );
              },
            },
          }),
        ),
      );
    }

    return card;
  }

  function detailRow(term: string, value: string): HTMLElement {
    return el(
      "div",
      { class: "move-kv" },
      el("dt", { text: term }),
      el("dd", { text: value }),
    );
  }

  function buildStepDetails(step: ConsolidationPlanStep): HTMLElement {
    const rows: HTMLElement[] = [
      detailRow("Step id", step.id),
      detailRow("Action", String(step.action)),
      detailRow("Status", String(step.status)),
      detailRow("Asset kind", String(step.asset_kind)),
      detailRow("Amount (hex)", step.amount_hex || "-"),
      detailRow("From address", step.address || "-"),
      detailRow("Destination", step.destination_address || "-"),
      detailRow("Chain id", String(step.chain_id)),
      detailRow("Derivation path", step.derivation_path || "-"),
      detailRow("Sequence", String(step.sequence ?? 0)),
      detailRow("Depends on", (step.depends_on || []).join(", ") || "-"),
      detailRow("Signer status", String(step.signer_status || "-")),
      detailRow("Risk level", String(step.risk_level || "-")),
      detailRow("Simulation", String(step.simulation_status || "not_run")),
      detailRow(
        "Simulation evidence",
        (step.simulation_evidence || []).join(" · ") || "-",
      ),
    ];
    if (step.asset_address) {
      rows.push(detailRow("Asset address", step.asset_address));
    }
    if (step.token_id_hex) {
      rows.push(detailRow("Token id (hex)", step.token_id_hex));
    }
    if (step.counterparty_address) {
      rows.push(detailRow("Spender / operator", step.counterparty_address));
    }
    if (step.protocol_address) {
      rows.push(detailRow("Protocol", step.protocol_address));
    }
    if (step.queued_job_id) {
      rows.push(detailRow("Queue job", step.queued_job_id));
    }
    return el(
      "details",
      { class: "move-step-raw" },
      el("summary", { text: "Technical details" }),
      el("dl", { class: "move-kv-list" }, ...rows),
    );
  }

  // ── Plan actions (approve / simulate / export / enqueue) ───────────

  function buildPlanActionBar(
    plan: ConsolidationPlan,
    eligibleCount: number,
    symbol: string,
  ): HTMLElement {
    const reviewRequired = plan.summary?.review_required_steps ?? 0;

    const approveButton = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button", "data-move-region": "approve-plan" },
      text: "Approve reviewable steps",
      on: {
        click: (event) =>
          void approvePlan(plan.id, event.currentTarget as HTMLElement),
      },
    }) as HTMLButtonElement;
    approveButton.disabled = reviewRequired === 0;

    const simulateButton = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button", "data-move-region": "simulate-plan" },
      text: "Simulate plan",
      on: {
        click: (event) =>
          void simulatePlan(plan.id, event.currentTarget as HTMLElement),
      },
    }) as HTMLButtonElement;

    const exportCallButton = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button", "data-move-region": "export-call" },
      text: "Export call JSON",
      on: {
        click: (event) =>
          void exportPlan(
            plan.id,
            "call_manifest",
            null,
            event.currentTarget as HTMLElement,
          ),
      },
    }) as HTMLButtonElement;

    const safeInput = el("input", {
      class: "input-mid",
      attrs: {
        type: "text",
        placeholder: "Safe address",
        autocomplete: "off",
        "aria-label": "Safe address for Safe transaction builder export",
        "data-move-region": "safe-address",
      },
    }) as HTMLInputElement;
    const exportSafeButton = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button", "data-move-region": "export-safe" },
      text: "Export Safe JSON",
      on: {
        click: (event) =>
          void exportPlan(
            plan.id,
            "safe_tx_builder",
            safeInput.value.trim() || null,
            event.currentTarget as HTMLElement,
          ),
      },
    }) as HTMLButtonElement;

    const enqueueButton = el("button", {
      class: "btn-primary",
      attrs: { type: "button", "data-move-region": "enqueue-plan" },
      text: "Enqueue eligible steps…",
      on: {
        click: (event) =>
          void enqueuePlanWithTypedConfirm(
            plan.id,
            event.currentTarget as HTMLElement,
          ),
      },
    }) as HTMLButtonElement;
    enqueueButton.disabled = eligibleCount === 0;

    return el(
      "div",
      { class: "move-actionbar" },
      el(
        "div",
        { class: "move-actionbar-secondary" },
        approveButton,
        simulateButton,
        exportCallButton,
        el("span", { class: "move-safe-export" }, safeInput, exportSafeButton),
      ),
      el(
        "div",
        { class: "move-actionbar-primary" },
        el("p", {
          class: "move-actionbar-hint nums",
          text:
            eligibleCount === 0
              ? state.policyLoaded && !state.policy
                ? "No treasury policy — set one below before anything can execute."
                : "No steps are currently eligible to enqueue."
              : eligibleCount +
                " of " +
                (plan.steps || []).length +
                " steps eligible" +
                (planNativeTotalWeiHex(plan)
                  ? " · up to " +
                    (formatWeiHexAsEth(planNativeTotalWeiHex(plan)) ?? "?") +
                    " " +
                    symbol
                  : ""),
        }),
        enqueueButton,
      ),
    );
  }

  async function approvePlan(planId: string, button: HTMLElement): Promise<void> {
    setBusy(button, true);
    try {
      const payload = await moveApi.approvePlan(planId);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("planNotice", failure);
        return;
      }
      state.planNotice = {
        message: "Reviewable plan steps approved.",
        tone: "success",
      };
      await refreshPlans();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  async function simulatePlan(planId: string, button: HTMLElement): Promise<void> {
    setBusy(button, true);
    try {
      const payload = await moveApi.simulatePlan(planId);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("planNotice", failure);
        return;
      }
      state.planNotice = {
        message: "Preflight simulation updated — check each step's badge for freshness.",
        tone: "success",
      };
      await refreshPlans();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  function downloadJson(filename: string, payload: unknown): void {
    try {
      const blob = new Blob([JSON.stringify(payload, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a") as HTMLAnchorElement;
      anchor.href = url;
      anchor.download = filename;
      document.body.appendChild(anchor);
      anchor.click?.();
      anchor.remove();
      URL.revokeObjectURL(url);
    } catch (_) {
      /* downloads unavailable (tests): the notice below still reports counts */
    }
  }

  function exportFilename(response: ConsolidationPlanExportResponse): string {
    const plan = String(response.plan_id || "plan").replace(
      /[^a-zA-Z0-9_.-]/g,
      "_",
    );
    return "sigillum-" + plan + "-" + response.format + ".json";
  }

  async function exportPlan(
    planId: string,
    format: string,
    safeAddress: string | null,
    button: HTMLElement,
  ): Promise<void> {
    if (format === "safe_tx_builder" && !safeAddress) {
      state.planNotice = {
        message:
          "Enter the Safe address first — it is required for a Safe transaction builder export.",
        tone: "error",
      };
      renderPlansCard();
      return;
    }
    setBusy(button, true);
    try {
      const payload = await moveApi.exportPlan(planId, format, safeAddress);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("planNotice", failure);
        return;
      }
      const response = payload as unknown as ConsolidationPlanExportResponse;
      downloadJson(exportFilename(response), response);
      state.planNotice = {
        message:
          "Exported " +
          String(response.exported_steps || 0) +
          " step(s); skipped " +
          String((response.skipped_steps || []).length) +
          ".",
        tone: "success",
      };
      renderPlansCard();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  async function enqueueSingleStep(
    planId: string,
    stepId: string,
    button: HTMLElement,
  ): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Enqueue plan step",
      body:
        "Enqueue plan step " +
        stepId +
        " as an execution queue job? The daemon re-validates every gate; " +
        "queued plan-step jobs stay blocked until execution is enabled (W7.3).",
      actionLabel: "Enqueue step",
    });
    if (!confirmed) return;
    setBusy(button, true);
    try {
      const payload = await moveApi.enqueueStep(planId, stepId);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("planNotice", failure);
        return;
      }
      const job = payload.job as { id?: string } | undefined;
      state.planNotice = {
        message: "Plan step queued as job " + (job?.id || "?") + ".",
        tone: "success",
      };
      await refreshPlans();
      await refreshQueue();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  /**
   * THE deliberate approve action. Preserves the legacy enqueuePlanBulk
   * server contract EXACTLY: probe with an empty confirmation (nothing is
   * enqueued; the daemon computes the expected phrase from the CURRENTLY
   * eligible steps and returns it in the machine-readable `action` field),
   * gate the shared typed-confirm dialog on that phrase, then re-POST with
   * it. The daemon re-validates every gate before executing.
   */
  async function enqueuePlanWithTypedConfirm(
    planId: string,
    button: HTMLElement,
  ): Promise<void> {
    setBusy(button, true);
    let probe: DaemonPayload;
    try {
      probe = await moveApi.enqueuePlan(planId, "");
    } catch (error) {
      setBusy(button, false);
      showFailureAsNotice("planNotice", thrownFailure(error));
      return;
    }
    const expected = typeof probe.action === "string" ? probe.action : null;
    if (!expected) {
      setBusy(button, false);
      state.planNotice = {
        message:
          (typeof probe.error === "string" && probe.error) ||
          "No plan steps are eligible for enqueue.",
        tone: "error",
      };
      renderPlansCard();
      return;
    }
    setBusy(button, false);
    const confirmed = await confirmTypedDialog({
      title: "Enqueue all eligible plan steps",
      body:
        "This enqueues every eligible step of the plan as execution queue jobs. " +
        "The daemon re-validates every gate before executing; steps that pass " +
        "their checks are signed and broadcast on-chain.",
      phrase: expected,
      actionLabel: "Enqueue all",
    });
    if (!confirmed) return;
    setBusy(button, true);
    try {
      const payload = await moveApi.enqueuePlan(planId, expected);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("planNotice", failure);
        return;
      }
      state.planNotice = {
        message:
          "Enqueued " +
          String((payload.enqueued as unknown[])?.length ?? 0) +
          " step(s); skipped " +
          String((payload.skipped as unknown[])?.length ?? 0) +
          ". Track them in the queue.",
        tone: "success",
      };
      await refreshPlans();
      await refreshQueue();
    } catch (error) {
      showFailureAsNotice("planNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  // ── Queue card: the ops timeline ───────────────────────────────────

  interface QueueShell {
    ops: HTMLElement;
    notice: HTMLElement;
    banner: HTMLElement;
    groups: HTMLElement;
    pagination: HTMLElement;
    pauseButton: HTMLButtonElement;
    pausedBanner: HTMLElement;
    processSubmit: HTMLButtonElement;
  }
  let queueShell: QueueShell | null = null;

  function buildQueueShell(card: HTMLElement): QueueShell {
    clearChildren(card);
    const ops = el("div", {
      class: "move-ops hidden",
      attrs: { "data-move-region": "queue-ops" },
    });
    const notice = el("div", {
      class: "move-notice hidden",
      attrs: { role: "status", "data-move-region": "queue-notice" },
    });
    const banner = el("div", {
      attrs: { role: "status", "data-move-region": "queue-banner" },
    });
    const groups = el("div", {
      attrs: { "data-move-region": "queue-groups" },
    });
    const pagination = el("div", {
      attrs: { "data-move-region": "queue-pagination" },
    });
    const pausedBanner = el(
      "div",
      { class: "move-banner hidden", dataset: { tier: "review" } },
      el("p", {
        class: "move-banner-text",
        text:
          "Queue execution is paused. No queued job will run until you resume — this holds even while the treasury policy is disabled.",
      }),
    );
    const pauseButton = el("button", {
      class: "btn-danger btn-small",
      attrs: { type: "button", "data-move-region": "queue-pause" },
      text: "Pause execution",
      on: { click: () => void toggleQueuePause() },
    }) as HTMLButtonElement;

    const limit = el("input", {
      class: "input-mid nums",
      attrs: {
        type: "number",
        min: "1",
        "aria-label": "Batch size",
        "data-move-region": "queue-process-limit",
      },
    }) as HTMLInputElement;
    limit.value = "20";
    const runAsync = el("input", {
      attrs: { type: "checkbox", "data-move-region": "queue-process-async" },
    }) as HTMLInputElement;
    const processSubmit = el("button", {
      class: "btn-primary",
      attrs: { type: "submit", "data-move-region": "queue-process" },
      text: "Process queue",
    }) as HTMLButtonElement;
    const processForm = el(
      "form",
      { class: "move-queue-controls" },
      el(
        "label",
        { class: "field-label" },
        el("span", { text: "Batch size" }),
        limit,
      ),
      el(
        "label",
        {
          class: "checkbox-row",
          attrs: {
            title:
              "Start the drain as a background operation you can cancel; progress shows above the list",
          },
        },
        runAsync,
        el("span", { text: " Run in background" }),
      ),
      processSubmit,
    );
    processForm.addEventListener("submit", (event) => {
      event.preventDefault();
      void processQueueBatch(limit, runAsync, processSubmit);
    });

    card.appendChild(
      el(
        "div",
        { class: "move-root" },
        el(
          "div",
          { class: "move-card-head" },
          el(
            "div",
            null,
            el("h3", { class: "move-card-title", text: "Execution queue" }),
            el("p", {
              class: "move-card-summary",
              text:
                "What the queue is doing, grouped by state, newest activity first. Jobs that need you float to the top.",
            }),
          ),
          el(
            "div",
            { class: "move-card-head-actions" },
            pauseButton,
            el("button", {
              class: "btn-ghost btn-small",
              attrs: { type: "button" },
              text: "Refresh",
              on: { click: () => void refreshQueue() },
            }),
          ),
        ),
        pausedBanner,
        ops,
        processForm,
        notice,
        banner,
        groups,
        pagination,
      ),
    );
    return {
      ops,
      notice,
      banner,
      groups,
      pagination,
      pauseButton,
      pausedBanner,
      processSubmit,
    };
  }

  function renderQueueCard(): void {
    const card = dom.cards.queueCard;
    if (!card || !state.mounted) return;
    if (!queueShell) queueShell = buildQueueShell(card);
    const shell = queueShell;

    // Pause switch mirrors the policy's execution_paused flag.
    const paused = Boolean(state.policy?.execution_paused);
    setHidden(shell.pausedBanner, !paused);
    shell.pauseButton.textContent = paused
      ? "Resume execution"
      : "Pause execution";
    shell.pauseButton.className = paused
      ? "btn-success btn-small"
      : "btn-danger btn-small";

    renderQueueOps();
    renderNotice(shell.notice, state.queueNotice);

    clearChildren(shell.banner);
    if (state.queueFailure && state.queue !== null) {
      shell.banner.appendChild(
        staleBanner("the queue", () => void refreshQueue()),
      );
    }

    clearChildren(shell.groups);
    if (state.queueLoading && state.queue === null) {
      shell.groups.appendChild(skeletonBlock(3));
    } else if (state.queueFailure && state.queue === null) {
      shell.groups.appendChild(
        failurePanel(state.queueFailure, () => void refreshQueue()),
      );
    } else if ((state.queue || []).length === 0) {
      shell.groups.appendChild(
        sectionEmpty(
          "The queue is empty",
          "Nothing is waiting to run. Approve a plan above or queue a deposit sweep, and jobs will show up here for review and processing.",
          "Back to plans",
          () => runtime.router.navigate(formatHash("move")),
        ),
      );
    } else {
      const groupsEl = el("div", { class: "move-queue-groups" });
      shell.groups.appendChild(groupsEl);
      renderList<QueueJobGroup>(
        groupsEl,
        groupQueueJobs(state.queue || []),
        (group) => group.state,
        renderQueueGroup,
      );
    }

    renderQueuePagination(shell.pagination);
  }

  function renderQueuePagination(container: HTMLElement): void {
    clearChildren(container);
    const info = state.queuePagination;
    if (!info || info.total <= QUEUE_PAGE_SIZE) return;
    const page = Math.floor(info.offset / QUEUE_PAGE_SIZE) + 1;
    const pages = Math.max(1, Math.ceil(info.total / QUEUE_PAGE_SIZE));
    const prev = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "← Newer",
      on: {
        click: () => {
          state.queueOffset = Math.max(0, state.queueOffset - QUEUE_PAGE_SIZE);
          void refreshQueue();
        },
      },
    }) as HTMLButtonElement;
    prev.disabled = info.offset <= 0;
    const next = el("button", {
      class: "btn-ghost btn-small",
      attrs: { type: "button" },
      text: "Older →",
      on: {
        click: () => {
          state.queueOffset = state.queueOffset + QUEUE_PAGE_SIZE;
          void refreshQueue();
        },
      },
    }) as HTMLButtonElement;
    next.disabled = !info.has_more;
    container.appendChild(
      el(
        "div",
        { class: "move-pagination" },
        prev,
        el("span", {
          class: "move-pagination-label nums",
          text: "Page " + page + " of " + pages + " · " + info.total + " jobs",
        }),
        next,
      ),
    );
  }

  /** Recent transitions for one job, from the queueEvents slice (oldest →
   * newest; the slice is newest-first and carries no timestamps). */
  function jobTimeline(jobId: string): string[] {
    const events = runtime.store.get("queueEvents") || [];
    const states = events
      .filter((event) => event.job_id === jobId)
      .map((event) => event.state);
    return states.reverse().slice(-4);
  }

  function renderQueueGroup(
    group: QueueJobGroup,
    existing: HTMLElement | null,
  ): HTMLElement {
    const fingerprint = JSON.stringify([
      group.state,
      group.jobs.map((job) => [
        job.id,
        job.state,
        job.updated_at_unix,
        job.attempts,
        job.last_error,
        job.receipt_status,
        job.confirmations,
        job.next_attempt_after_unix,
        jobTimeline(job.id).join(">"),
      ]),
    ]);
    return patchKeyedRow(existing, "move-queue-group", fingerprint, (row) => {
      row.appendChild(
        el(
          "h4",
          {
            class: "move-queue-group-title",
            dataset: { tier: queueStateTier(group.state) },
          },
          queueStateLabel(group.state) + " ",
          el("span", { class: "nums", text: "(" + group.jobs.length + ")" }),
        ),
      );
      const jobsEl = el("ul", { class: "move-job-list" });
      row.appendChild(jobsEl);
      renderList<QueueJob>(
        jobsEl,
        group.jobs,
        (job) => job.id,
        renderQueueJobRow,
      );
    });
  }

  function renderQueueJobRow(
    job: QueueJob,
    existing: HTMLElement | null,
  ): HTMLElement {
    const now = nowSecs();
    const timeline = jobTimeline(job.id);
    const fingerprint = JSON.stringify([
      job.id,
      job.state,
      job.updated_at_unix,
      job.attempts,
      job.last_error,
      job.receipt_status,
      job.confirmations,
      job.next_attempt_after_unix,
      timeline.join(">"),
      state.chains.length,
    ]);
    return patchKeyedRow(existing, "move-job", fingerprint, (row) => {
      row.dataset.tier = queueStateTier(job.state || "");
      const symbol = nativeSymbol(job.chain_id);
      row.appendChild(
        el(
          "div",
          { class: "move-job-head" },
          el("p", {
            class: "move-job-title",
            text: describeQueueJob(job, { symbol }),
          }),
          pill(job.state, queueStateLabel(job.state || "")),
        ),
      );
      row.appendChild(
        el(
          "p",
          { class: "move-job-meta" },
          queueKindLabel(job.kind || "unknown") + " · created ",
          timeEl(job.created_at_unix, now),
          " · updated ",
          timeEl(job.updated_at_unix, now),
          job.attempts ? " · " + job.attempts + " attempts" : "",
          job.next_attempt_after_unix
            ? " · next try " + futureTime(job.next_attempt_after_unix, now)
            : "",
        ),
      );

      if (job.last_error) {
        row.appendChild(
          el("p", {
            class: "move-job-error",
            text: humanizeQueueError(job.last_error),
          }),
        );
      }

      // Truthful post-broadcast receipt info (W7.4): only once a receipt
      // exists; `sent` means "broadcast, awaiting confirmation".
      if (job.transaction_hash_hex || job.receipt_status) {
        row.appendChild(
          el(
            "p",
            { class: "move-job-receipt nums" },
            "tx " + shortAddress(job.transaction_hash_hex) + " · ",
            job.receipt_status
              ? job.receipt_status === "success"
                ? "confirmed on-chain"
                : "reverted on-chain"
              : "receipt pending",
            job.confirmations != null
              ? " · " + job.confirmations + " confirmations"
              : "",
            job.receipt_block_number != null
              ? " · block " + job.receipt_block_number
              : "",
            job.receipt_gas_used_hex
              ? " · gas used " +
                (formatHexQuantity(job.receipt_gas_used_hex) ?? "?")
              : "",
          ),
        );
      }

      if (timeline.length > 1) {
        row.appendChild(
          el(
            "p",
            { class: "move-job-timeline" },
            el("span", { class: "move-job-timeline-label", text: "Recent: " }),
            timeline
              .map((state) => queueStateLabel(state).toLowerCase())
              .join(" → "),
          ),
        );
      }

      const actions = el("div", { class: "move-job-actions" });
      if (queueJobCanProcess(job)) {
        actions.appendChild(
          el("button", {
            class: "btn-ghost btn-small",
            attrs: { type: "button" },
            text: "Process now",
            on: {
              click: (event) =>
                void processQueueJob(job.id, event.currentTarget as HTMLElement),
            },
          }),
        );
      }
      if (job.kind === "plan_step_execution" && job.plan_id) {
        actions.appendChild(
          el("a", {
            class: "btn-ghost btn-small move-btn-link",
            attrs: { href: formatHash("move", "plan", job.plan_id) },
            text: "View plan",
          }),
        );
      }
      if (actions.childNodes.length) row.appendChild(actions);
    });
  }

  // ── Background operations strip (from the operations slice) ────────

  function renderQueueOps(): void {
    const shell = queueShell;
    if (!shell || !state.mounted) return;
    const operations = (runtime.store.get("operations") || []).filter(
      (operation) =>
        MOVE_OPERATION_KINDS.has(operation.kind) &&
        ACTIVE_OPERATION_STATES.has(operation.state),
    );
    setHidden(shell.ops, operations.length === 0);
    renderList<Operation>(shell.ops, operations, (op) => op.id, renderOpRow);
  }

  function renderOpRow(
    operation: Operation,
    existing: HTMLElement | null,
  ): HTMLElement {
    const fingerprint = JSON.stringify([
      operation.id,
      operation.state,
      operation.progress?.processed,
      operation.progress?.total,
    ]);
    return patchKeyedRow(existing, "move-op", fingerprint, (row) => {
      const label =
        operation.kind === "maintenance_run"
          ? "Maintenance cycle"
          : "Queue drain";
      const total = operation.progress?.total;
      const progress = total
        ? " · step " + operation.progress.processed + " of " + total
        : operation.progress?.processed
          ? " · " + operation.progress.processed + " done"
          : "";
      row.appendChild(
        el("span", { class: "status-dot", dataset: { state: "busy" } }),
      );
      row.appendChild(
        el(
          "p",
          { class: "move-op-label" },
          label + " running",
          el("span", { class: "nums", text: progress }),
          operation.state === "cancel_requested" ? " · canceling…" : "",
        ),
      );
      row.appendChild(
        el("button", {
          class: "btn-ghost btn-small",
          attrs: { type: "button" },
          text: "Cancel",
          on: {
            click: (event) =>
              void cancelOperation(
                operation.id,
                event.currentTarget as HTMLElement,
              ),
          },
        }),
      );
    });
  }

  async function cancelOperation(id: string, button: HTMLElement): Promise<void> {
    setBusy(button, true);
    try {
      await runtime.api.cancelOperation(id);
      state.queueNotice = {
        message: "Cancel requested — the operation stops at the next safe point.",
        tone: "info",
      };
      renderQueueCard();
    } catch (error) {
      showFailureAsNotice("queueNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  // ── Queue actions ──────────────────────────────────────────────────

  async function toggleQueuePause(): Promise<void> {
    const paused = Boolean(state.policy?.execution_paused);
    try {
      if (paused) {
        await runtime.api.resumeQueue();
        state.queueNotice = {
          message: "Queue execution resumed.",
          tone: "success",
        };
      } else {
        await runtime.api.pauseQueue();
        state.queueNotice = {
          message: "Queue execution paused — no job will run until you resume.",
          tone: "warning",
        };
      }
    } catch (error) {
      showFailureAsNotice("queueNotice", thrownFailure(error));
    }
    await refreshPolicy();
  }

  async function processQueueBatch(
    limit: HTMLInputElement,
    runAsync: HTMLInputElement,
    button: HTMLElement,
  ): Promise<void> {
    const limitText = limit.value.trim();
    const limitValue = limitText ? parseInt(limitText, 10) : null;
    const confirmed = await confirmDangerDialog({
      title: "Process queue",
      body:
        (limitValue
          ? "Process up to " + limitValue + " queued jobs now?"
          : "Process queued jobs now?") +
        " Jobs that pass their checks will be signed and broadcast on-chain.",
      actionLabel: "Process now",
    });
    if (!confirmed) return;
    setBusy(button, true);
    try {
      const response = await runtime.api.processQueue({
        // Legacy sends `id: null` for a batch drain — same request DTO.
        id: null as unknown as string,
        limit: limitValue ?? undefined,
        run_async: runAsync.checked || undefined,
      });
      if (runAsync.checked && response.operation?.id) {
        state.queueNotice = {
          message:
            "Queue drain running in the background — progress is above the list, and you can cancel it there.",
          tone: "info",
        };
      } else {
        state.queueNotice = {
          message: queueProcessSummary(response),
          tone:
            response.failed || response.operator_action_required
              ? "warning"
              : "success",
        };
      }
      renderQueueCard();
      await refreshQueue();
    } catch (error) {
      showFailureAsNotice("queueNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  async function processQueueJob(id: string, button: HTMLElement): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Process queued job",
      body:
        'Process queued job "' +
        id +
        '" now? If it passes its checks it will be signed and broadcast on-chain.',
      actionLabel: "Process now",
    });
    if (!confirmed) return;
    setBusy(button, true);
    try {
      const response = await runtime.api.processQueue({ id, limit: 1 });
      state.queueNotice = {
        message: queueProcessSummary(response),
        tone:
          response.failed || response.operator_action_required
            ? "warning"
            : "success",
      };
      renderQueueCard();
      await refreshQueue();
    } catch (error) {
      showFailureAsNotice("queueNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  // ── Policy card: presets + guided editor ───────────────────────────

  const DEFAULT_HOT_REFILL_WEI_HEX = "0xde0b6b3a7640000"; // 1 ETH (legacy default)

  interface PolicyShell {
    banner: HTMLElement;
    current: HTMLElement;
    notice: HTMLElement;
    presetNote: HTMLElement;
    presetButtons: { id: PolicyPreset["id"]; button: HTMLButtonElement }[];
    fieldErrors: HTMLElement;
    preview: HTMLElement;
    saveButton: HTMLButtonElement;
    enabled: HTMLInputElement;
    requireSim: HTMLInputElement;
    blockLinkage: HTMLInputElement;
    allowPlanExec: HTMLInputElement;
    allowSweepExec: HTMLInputElement;
    allowRevokeExec: HTMLInputElement;
    allowExitExec: HTMLInputElement;
    allowClaimExec: HTMLInputElement;
    allowGasTopups: HTMLInputElement;
    allowAutomation: HTMLInputElement;
    destinations: HTMLTextAreaElement;
    maxStep: HTMLInputElement;
    maxPlan: HTMLInputElement;
    freshness: HTMLInputElement;
    maxGasTopup: HTMLInputElement;
    hotFloor: HTMLInputElement;
    hotTarget: HTMLInputElement;
    hotOverflow: HTMLInputElement;
    maxFee: HTMLInputElement;
  }
  let policyShell: PolicyShell | null = null;

  function checkboxField(
    labelText: string,
    hint: string | undefined,
    region: string,
  ): { row: HTMLElement; input: HTMLInputElement } {
    const input = el("input", {
      attrs: { type: "checkbox", "data-move-region": region },
    }) as HTMLInputElement;
    const row = el(
      "label",
      hint
        ? { class: "checkbox-row", attrs: { title: hint } }
        : { class: "checkbox-row" },
      input,
      el("span", { text: " " + labelText }),
    );
    return { row, input };
  }

  function textField(
    labelText: string,
    options: { placeholder: string; region: string; hint?: string },
  ): { field: HTMLElement; input: HTMLInputElement } {
    const input = el("input", {
      class: "input-mid",
      attrs: {
        type: "text",
        placeholder: options.placeholder,
        autocomplete: "off",
        "data-move-region": options.region,
      },
    }) as HTMLInputElement;
    const field = el(
      "label",
      { class: "field-label" },
      el("span", { text: labelText }),
      input,
      options.hint ? el("span", { class: "field-hint", text: options.hint }) : null,
    );
    return { field, input };
  }

  function buildPolicyShell(card: HTMLElement): PolicyShell {
    clearChildren(card);
    const banner = el("div", {
      attrs: { role: "status", "data-move-region": "policy-banner" },
    });
    const current = el("div", {
      attrs: { "data-move-region": "policy-current" },
    });
    const notice = el("div", {
      class: "move-notice hidden",
      attrs: { role: "status", "data-move-region": "policy-notice" },
    });
    const presetNote = el("p", {
      class: "field-hint",
      attrs: { "data-move-region": "preset-note" },
    });
    const fieldErrors = el("div", {
      class: "move-field-errors hidden",
      attrs: { role: "alert", "data-move-region": "policy-field-errors" },
    });
    const preview = el("div", {
      class: "move-policy-preview",
      attrs: { "data-move-region": "policy-preview" },
    });

    const enabled = checkboxField(
      "Policy enabled",
      "A disabled policy blocks ALL execution — nothing moves.",
      "policy-enabled",
    );
    const allowPlanExec = checkboxField(
      "Allow plan execution (master gate)",
      "Nothing executes from a plan step unless this AND the per-family gate are on.",
      "policy-allow-plan-exec",
    );
    const allowSweepExec = checkboxField(
      "Allow sweep execution",
      "Covers plan-step sweeps AND stealth deposit sweeps/transfers — no stealth carve-out.",
      "policy-allow-sweep-exec",
    );
    const allowRevokeExec = checkboxField(
      "Allow revoke execution",
      "Let approved revoke steps execute on-chain.",
      "policy-allow-revoke-exec",
    );
    const allowExitExec = checkboxField(
      "Allow DeFi exit execution",
      "Let approved DeFi exit steps execute on-chain.",
      "policy-allow-exit-exec",
    );
    const allowClaimExec = checkboxField(
      "Allow Merkle claim execution",
      "Claims stay blocked unless simulation passed and the contract is trusted/reviewed.",
      "policy-allow-claim-exec",
    );
    const requireSim = checkboxField(
      "Require simulation",
      "Steps need a fresh passed simulation before they can enqueue.",
      "policy-require-sim",
    );
    const blockLinkage = checkboxField(
      "Block cross-party linkage (fail-closed, default on)",
      "Blocks plans that would sweep different payers to the same destination.",
      "policy-block-linkage",
    );
    const allowGasTopups = checkboxField(
      "Allow sponsor gas top-ups",
      "Plans may fund a source address's gas shortfall from the wallet's sponsor address.",
      "policy-allow-gas-topups",
    );
    const allowAutomation = checkboxField(
      "Allow treasury automation",
      "Maintenance may plan hot-overflow sweeps and treasury refills and auto-enqueue them; steps still pass every gate.",
      "policy-allow-automation",
    );

    const destinations = el("textarea", {
      class: "input-wide",
      attrs: {
        placeholder: "One per line: 0xADDRESS or 0xADDRESS:label",
        "data-move-region": "policy-destinations",
        "aria-label": "Allowed destinations, one per line",
      },
    }) as HTMLTextAreaElement;

    const maxStep = textField("Per-step cap (ETH)", {
      placeholder: "optional, e.g. 0.5",
      region: "policy-max-step",
      hint: "Largest native value one step may move.",
    });
    const maxPlan = textField("Per-plan cap (ETH)", {
      placeholder: "optional, e.g. 2",
      region: "policy-max-plan",
      hint: "Largest native value a whole plan may move.",
    });
    const maxGasTopup = textField("Max gas top-up (ETH)", {
      placeholder: "required when top-ups are on, e.g. 0.05",
      region: "policy-max-gas-topup",
      hint: "Required when sponsor gas top-ups are on; no default cap is assumed.",
    });
    const maxFee = textField("Max fee per gas (gwei)", {
      placeholder: "optional, e.g. 50",
      region: "policy-max-fee",
      hint: "Broadcasts above this fee are refused.",
    });
    const freshness = textField("Simulation freshness (seconds)", {
      placeholder: "default 900",
      region: "policy-freshness",
      hint: "How long a passed simulation counts as fresh.",
    });
    const hotFloor = textField("Hot floor (ETH)", {
      placeholder: "default 1",
      region: "policy-hot-floor",
    });
    const hotTarget = textField("Hot target (ETH)", {
      placeholder: "default 1",
      region: "policy-hot-target",
    });
    const hotOverflow = textField("Hot overflow threshold (ETH)", {
      placeholder: "optional, e.g. 1.5",
      region: "policy-hot-overflow",
    });

    const saveButton = el("button", {
      class: "btn-primary",
      attrs: { type: "submit", "data-move-region": "policy-save" },
      text: "Save policy",
    }) as HTMLButtonElement;

    const presetButtons: PolicyShell["presetButtons"] = [];
    const presetsWrap = el(
      "div",
      {
        class: "move-presets",
        attrs: { role: "group", "aria-label": "Policy presets" },
      },
    );
    for (const preset of POLICY_PRESETS) {
      const button = el("button", {
        class: "move-preset",
        attrs: {
          type: "button",
          "aria-pressed": "false",
          "data-move-region": "preset-" + preset.id,
        },
        text: preset.label,
        on: { click: () => applyPreset(preset) },
      }) as HTMLButtonElement;
      presetButtons.push({ id: preset.id, button });
      presetsWrap.appendChild(button);
    }

    const form = el(
      "form",
      { class: "move-policy-form" },
      el(
        "div",
        { class: "move-policy-section" },
        el("h4", { class: "move-policy-heading", text: "Gates" }),
        enabled.row,
        allowPlanExec.row,
        allowSweepExec.row,
        allowRevokeExec.row,
        allowExitExec.row,
        allowClaimExec.row,
        requireSim.row,
        blockLinkage.row,
      ),
      el(
        "div",
        { class: "move-policy-section" },
        el("h4", { class: "move-policy-heading", text: "Caps and gas" }),
        el(
          "div",
          { class: "move-policy-grid" },
          maxStep.field,
          maxPlan.field,
          maxGasTopup.field,
          maxFee.field,
          freshness.field,
        ),
        allowGasTopups.row,
      ),
      el(
        "div",
        { class: "move-policy-section" },
        el(
          "h4",
          { class: "move-policy-heading", text: "Hot-wallet refills and automation" },
        ),
        allowAutomation.row,
        el(
          "div",
          { class: "move-policy-grid" },
          hotFloor.field,
          hotTarget.field,
          hotOverflow.field,
        ),
        el("p", {
          class: "field-hint",
          text:
            "Sweeps route to the hot address while its balance is below the floor; the target is the refill ceiling (floor must be ≤ target).",
        }),
      ),
      el(
        "div",
        { class: "move-policy-section" },
        el("h4", { class: "move-policy-heading", text: "Allowed destinations" }),
        destinations,
        el("p", {
          class: "field-hint",
          text: "Sweeps may only route to these addresses when the policy is enabled.",
        }),
      ),
      fieldErrors,
      preview,
      el("div", { class: "form-row" }, saveButton),
    );
    form.addEventListener("input", () => {
      policyFormDirty = true;
      updatePolicyPreview();
    });
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void savePolicy();
    });

    card.appendChild(
      el(
        "div",
        { class: "move-root" },
        el(
          "div",
          { class: "move-card-head" },
          el(
            "div",
            null,
            el("h3", { class: "move-card-title", text: "Treasury policy" }),
            el("p", {
              class: "move-card-summary",
              text:
                "The guardrails every execution path answers to: where value may go, how much, and under which gates. Changes are written to the audit log.",
            }),
          ),
          el(
            "div",
            { class: "move-card-head-actions" },
            el("button", {
              class: "btn-ghost btn-small",
              attrs: { type: "button" },
              text: "Refresh",
              on: { click: () => void refreshPolicy() },
            }),
          ),
        ),
        banner,
        current,
        el(
          "div",
          { class: "move-policy-section" },
          el("h4", { class: "move-policy-heading", text: "Start from a preset" }),
          presetsWrap,
          presetNote,
        ),
        notice,
        form,
      ),
    );

    const shell: PolicyShell = {
      banner,
      current,
      notice,
      presetNote,
      presetButtons,
      fieldErrors,
      preview,
      saveButton,
      enabled: enabled.input,
      requireSim: requireSim.input,
      blockLinkage: blockLinkage.input,
      allowPlanExec: allowPlanExec.input,
      allowSweepExec: allowSweepExec.input,
      allowRevokeExec: allowRevokeExec.input,
      allowExitExec: allowExitExec.input,
      allowClaimExec: allowClaimExec.input,
      allowGasTopups: allowGasTopups.input,
      allowAutomation: allowAutomation.input,
      destinations,
      maxStep: maxStep.input,
      maxPlan: maxPlan.input,
      freshness: freshness.input,
      maxGasTopup: maxGasTopup.input,
      hotFloor: hotFloor.input,
      hotTarget: hotTarget.input,
      hotOverflow: hotOverflow.input,
      maxFee: maxFee.input,
    };
    return shell;
  }

  function applyPreset(preset: PolicyPreset): void {
    const shell = policyShell;
    if (!shell) return;
    activePreset = preset.id;
    if (preset.values) {
      const values = preset.values;
      shell.enabled.checked = values.enabled;
      shell.requireSim.checked = values.require_simulation;
      shell.blockLinkage.checked = values.block_cross_party_linkage;
      shell.allowPlanExec.checked = values.allow_plan_execution;
      shell.allowSweepExec.checked = values.allow_sweep_execution;
      shell.allowRevokeExec.checked = values.allow_revoke_execution;
      shell.allowExitExec.checked = values.allow_exit_execution;
      shell.allowClaimExec.checked = values.allow_claim_execution;
      shell.allowGasTopups.checked = values.allow_gas_topups;
      shell.allowAutomation.checked = values.allow_treasury_automation;
      policyFormDirty = true;
    }
    for (const entry of shell.presetButtons) {
      entry.button.setAttribute(
        "aria-pressed",
        String(entry.id === preset.id),
      );
    }
    shell.presetNote.textContent = preset.description;
    updatePolicyPreview();
  }

  /** The form's gate state as a policy-shaped object for the live summary. */
  function policyFromForm(): TreasuryPolicy {
    const shell = policyShell as PolicyShell;
    return {
      enabled: shell.enabled.checked,
      require_simulation: shell.requireSim.checked,
      block_cross_party_linkage: shell.blockLinkage.checked,
      allow_plan_execution: shell.allowPlanExec.checked,
      allow_sweep_execution: shell.allowSweepExec.checked,
      allow_revoke_execution: shell.allowRevokeExec.checked,
      allow_exit_execution: shell.allowExitExec.checked,
      allow_claim_execution: shell.allowClaimExec.checked,
      allow_gas_topups: shell.allowGasTopups.checked,
      max_gas_topup_wei_hex: parseEthToWeiHex(shell.maxGasTopup.value),
      allow_treasury_automation: shell.allowAutomation.checked,
      execution_paused: Boolean(state.policy?.execution_paused),
      allowed_destinations: parseTreasuryDestinationLines(
        shell.destinations.value,
      ),
      created_at_unix: state.policy?.created_at_unix ?? 0,
      updated_at_unix: state.policy?.updated_at_unix ?? 0,
    } as TreasuryPolicy;
  }

  function updatePolicyPreview(): void {
    const shell = policyShell;
    if (!shell) return;
    clearChildren(shell.preview);
    const sentences = treasuryPolicySummary(policyFromForm());
    shell.preview.appendChild(
      el("p", { class: "move-policy-preview-title", text: "What this policy will do" }),
    );
    shell.preview.appendChild(
      el(
        "ul",
        { class: "move-policy-sentences" },
        ...sentences.map((sentence) => el("li", { text: sentence })),
      ),
    );
  }

  function renderPolicyCard(): void {
    const card = dom.cards.policyCard;
    if (!card || !state.mounted) return;
    if (!policyShell) policyShell = buildPolicyShell(card);
    const shell = policyShell;

    clearChildren(shell.banner);
    if (state.policyFailure && state.policyLoaded) {
      shell.banner.appendChild(
        staleBanner("the treasury policy", () => void refreshPolicy()),
      );
    }
    renderNotice(shell.notice, state.policyNotice);
    renderPolicyCurrent(shell);
    prefillPolicyForm(shell);
  }

  function renderPolicyCurrent(shell: PolicyShell): void {
    clearChildren(shell.current);
    if (!state.policyLoaded && !state.policyFailure) {
      shell.current.appendChild(skeletonBlock(2));
      return;
    }
    if (state.policyFailure && !state.policyLoaded) {
      shell.current.appendChild(
        failurePanel(state.policyFailure, () => void refreshPolicy()),
      );
      return;
    }
    if (!state.policy) {
      shell.current.appendChild(
        sectionEmpty(
          "No treasury policy yet",
          "Nothing may execute — no plan steps, no stealth deposit sweeps — until a policy is saved. Pick a preset as a starting point, adjust it, and save.",
          null,
          null,
        ),
      );
      return;
    }
    const policy = state.policy;
    const wrap = el("div", { class: "move-policy-current" });
    wrap.appendChild(
      el(
        "p",
        { class: "move-policy-current-title" },
        el("span", { text: "Current policy " }),
        pill(policy.enabled ? "enabled" : "disabled"),
      ),
    );
    wrap.appendChild(
      el(
        "ul",
        { class: "move-policy-sentences" },
        ...treasuryPolicySummary(policy).map((sentence) =>
          el("li", { text: sentence }),
        ),
      ),
    );
    const destinations = policy.allowed_destinations || [];
    if (!destinations.length) {
      wrap.appendChild(
        el("p", {
          class: "move-plan-row-warning",
          text: "No allow-listed destinations — sweeps have nowhere to route until you add one.",
        }),
      );
    } else {
      wrap.appendChild(
        el(
          "ul",
          { class: "move-destinations" },
          ...destinations.map((destination) =>
            el(
              "li",
              { attrs: { title: destination.address } },
              el("span", { class: "nums", text: shortAddress(destination.address) }),
              destination.label ? " — " + destination.label : "",
            ),
          ),
        ),
      );
    }
    wrap.appendChild(
      el(
        "p",
        { class: "move-policy-updated" },
        "Updated ",
        timeEl(policy.updated_at_unix, nowSecs()),
      ),
    );

    // Raw state stays one click away.
    const cap = (weiHex: string | null | undefined, unit: "ETH" | "gwei"): string => {
      const amount =
        unit === "ETH" ? formatWeiHexAsEth(weiHex) : formatWeiHexAsGwei(weiHex);
      return amount ? amount + " " + unit : "-";
    };
    const gate = (value: unknown): string => (value ? "on" : "off");
    wrap.appendChild(
      el(
        "details",
        { class: "move-policy-raw" },
        el("summary", { text: "Technical state" }),
        el(
          "dl",
          { class: "move-kv-list" },
          detailRow("Per-step cap", cap(policy.max_step_native_wei_hex, "ETH")),
          detailRow("Per-plan cap", cap(policy.max_plan_native_wei_hex, "ETH")),
          detailRow("Hot floor", cap(policy.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX, "ETH")),
          detailRow("Hot target", cap(policy.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX, "ETH")),
          detailRow("Hot overflow", cap(policy.hot_overflow_wei_hex, "ETH")),
          detailRow("Max gas top-up", cap(policy.max_gas_topup_wei_hex, "ETH")),
          detailRow("Max fee per gas", cap(policy.max_fee_per_gas_cap_hex, "gwei")),
          detailRow("Simulation freshness", String(policy.simulation_freshness_secs ?? DEFAULT_SIM_FRESHNESS_SECS) + " seconds"),
          detailRow("Require simulation", gate(policy.require_simulation)),
          detailRow("Block cross-party linkage", gate(policy.block_cross_party_linkage)),
          detailRow("Allow claim execution", gate(policy.allow_claim_execution)),
          detailRow("Allow gas top-ups", gate(policy.allow_gas_topups)),
          detailRow("Allow treasury automation", gate(policy.allow_treasury_automation)),
          detailRow("Allow plan execution", gate(policy.allow_plan_execution)),
          detailRow("Allow sweep execution", gate(policy.allow_sweep_execution)),
          detailRow("Allow revoke execution", gate(policy.allow_revoke_execution)),
          detailRow("Allow DeFi exit execution", gate(policy.allow_exit_execution)),
          detailRow("Queue execution paused", gate(policy.execution_paused)),
        ),
      ),
    );
    shell.current.appendChild(wrap);
  }

  function prefillPolicyForm(shell: PolicyShell): void {
    const fingerprint = JSON.stringify(state.policy ?? null);
    if (fingerprint === policyFormFingerprint) return;
    if (policyFormDirty) return; // never clobber in-progress edits
    policyFormFingerprint = fingerprint;
    const policy = state.policy;
    shell.enabled.checked = policy ? policy.enabled : false;
    shell.requireSim.checked = policy ? policy.require_simulation : true;
    // Default-on (plan task 3.5): protection ON unless explicitly turned off.
    shell.blockLinkage.checked = policy
      ? Boolean(policy.block_cross_party_linkage)
      : true;
    shell.allowPlanExec.checked = Boolean(policy?.allow_plan_execution);
    shell.allowSweepExec.checked = Boolean(policy?.allow_sweep_execution);
    shell.allowRevokeExec.checked = Boolean(policy?.allow_revoke_execution);
    shell.allowExitExec.checked = Boolean(policy?.allow_exit_execution);
    shell.allowClaimExec.checked = Boolean(policy?.allow_claim_execution);
    shell.allowGasTopups.checked = Boolean(policy?.allow_gas_topups);
    shell.allowAutomation.checked = Boolean(policy?.allow_treasury_automation);
    shell.destinations.value = policy
      ? formatDestinationLines(policy.allowed_destinations)
      : "";
    shell.maxStep.value =
      (policy?.max_step_native_wei_hex &&
        formatWeiHexAsEth(policy.max_step_native_wei_hex)) ||
      "";
    shell.maxPlan.value =
      (policy?.max_plan_native_wei_hex &&
        formatWeiHexAsEth(policy.max_plan_native_wei_hex)) ||
      "";
    shell.maxGasTopup.value =
      (policy?.max_gas_topup_wei_hex &&
        formatWeiHexAsEth(policy.max_gas_topup_wei_hex)) ||
      "";
    shell.maxFee.value =
      (policy?.max_fee_per_gas_cap_hex &&
        formatWeiHexAsGwei(policy.max_fee_per_gas_cap_hex)) ||
      "";
    shell.hotFloor.value =
      formatWeiHexAsEth(policy?.hot_floor_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX) ||
      "";
    shell.hotTarget.value =
      formatWeiHexAsEth(policy?.hot_target_wei_hex ?? DEFAULT_HOT_REFILL_WEI_HEX) ||
      "";
    shell.hotOverflow.value =
      (policy?.hot_overflow_wei_hex &&
        formatWeiHexAsEth(policy.hot_overflow_wei_hex)) ||
      "";
    shell.freshness.value = String(
      policy?.simulation_freshness_secs ?? DEFAULT_SIM_FRESHNESS_SECS,
    );
    updatePolicyPreview();
  }

  /** Server validation_failed fields → editor inputs, by DTO field prefix. */
  function markPolicyFieldErrors(fields: FieldError[]): void {
    const shell = policyShell;
    if (!shell) return;
    const map: [string, HTMLElement][] = [
      ["allowed_destinations", shell.destinations],
      ["max_step_native_wei_hex", shell.maxStep],
      ["max_plan_native_wei_hex", shell.maxPlan],
      ["hot_floor_wei_hex", shell.hotFloor],
      ["hot_target_wei_hex", shell.hotTarget],
      ["hot_overflow_wei_hex", shell.hotOverflow],
      ["max_gas_topup_wei_hex", shell.maxGasTopup],
      ["max_fee_per_gas_cap_hex", shell.maxFee],
      ["simulation_freshness_secs", shell.freshness],
    ];
    const messages: string[] = [];
    for (const field of fields) {
      const match = map.find(([prefix]) => field.field.startsWith(prefix));
      if (match) match[1].classList.add("input-invalid");
      messages.push(field.message);
    }
    if (messages.length) showFormErrors(shell.fieldErrors, messages);
  }

  function clearPolicyFieldMarks(): void {
    const shell = policyShell;
    if (!shell) return;
    for (const input of [
      shell.destinations,
      shell.maxStep,
      shell.maxPlan,
      shell.hotFloor,
      shell.hotTarget,
      shell.hotOverflow,
      shell.maxGasTopup,
      shell.maxFee,
      shell.freshness,
    ]) {
      input.classList.remove("input-invalid");
    }
    clearChildren(shell.fieldErrors);
    shell.fieldErrors.classList.add("hidden");
  }

  /** Client-side validation + the update DTO (exact legacy request shape). */
  function buildPolicyDto(): TreasuryPolicyUpdateRequest | null {
    const shell = policyShell as PolicyShell;
    clearPolicyFieldMarks();
    const errors: { input: HTMLElement; message: string }[] = [];
    const ethField = (
      input: HTMLInputElement,
      message: string,
    ): string | null => {
      const text = input.value.trim();
      if (!text) return null;
      const parsed = parseEthToWeiHex(text);
      if (parsed === null) errors.push({ input, message });
      return parsed;
    };
    const maxStepWei = ethField(
      shell.maxStep,
      "Max per-step cap must be a decimal ETH amount with up to 18 decimals",
    );
    const maxPlanWei = ethField(
      shell.maxPlan,
      "Max per-plan cap must be a decimal ETH amount with up to 18 decimals",
    );
    const maxGasTopupWei = ethField(
      shell.maxGasTopup,
      "Max gas top-up must be a decimal ETH amount with up to 18 decimals",
    );
    if (shell.allowGasTopups.checked && !shell.maxGasTopup.value.trim()) {
      errors.push({
        input: shell.maxGasTopup,
        message:
          "Enter a finite max gas top-up before enabling sponsor gas top-ups",
      });
    }
    const hotFloorWei = ethField(
      shell.hotFloor,
      "Hot refill floor must be a decimal ETH amount with up to 18 decimals",
    );
    const hotTargetWei = ethField(
      shell.hotTarget,
      "Hot refill target must be a decimal ETH amount with up to 18 decimals",
    );
    const hotOverflowWei = ethField(
      shell.hotOverflow,
      "Hot overflow threshold must be a decimal ETH amount with up to 18 decimals",
    );
    const maxFeeText = shell.maxFee.value.trim();
    const maxFeeWei = maxFeeText ? parseGweiToWeiHex(maxFeeText) : null;
    if (maxFeeText && maxFeeWei === null) {
      errors.push({
        input: shell.maxFee,
        message: "Max fee per gas must be a decimal gwei amount with up to 9 decimals",
      });
    }
    const freshnessText = shell.freshness.value.trim();
    const freshnessSecs = freshnessText ? parseInt(freshnessText, 10) : null;
    if (
      freshnessText &&
      (freshnessSecs === null ||
        Number.isNaN(freshnessSecs) ||
        freshnessSecs <= 0)
    ) {
      errors.push({
        input: shell.freshness,
        message: "Simulation freshness must be a positive number of seconds",
      });
    }
    if (errors.length) {
      for (const error of errors) error.input.classList.add("input-invalid");
      showFormErrors(
        shell.fieldErrors,
        errors.map((error) => error.message),
      );
      return null;
    }

    const body: TreasuryPolicyUpdateRequest = {
      enabled: shell.enabled.checked,
      allowed_destinations: parseTreasuryDestinationLines(
        shell.destinations.value,
      ),
      max_step_native_wei_hex: maxStepWei,
      max_plan_native_wei_hex: maxPlanWei,
      require_simulation: shell.requireSim.checked,
      block_cross_party_linkage: shell.blockLinkage.checked,
      allow_claim_execution: shell.allowClaimExec.checked,
      allow_gas_topups: shell.allowGasTopups.checked,
      allow_treasury_automation: shell.allowAutomation.checked,
      max_gas_topup_wei_hex: maxGasTopupWei,
      allow_plan_execution: shell.allowPlanExec.checked,
      allow_sweep_execution: shell.allowSweepExec.checked,
      allow_revoke_execution: shell.allowRevokeExec.checked,
      allow_exit_execution: shell.allowExitExec.checked,
      max_fee_per_gas_cap_hex: maxFeeWei,
    };
    if (freshnessText) body.simulation_freshness_secs = freshnessSecs;
    if (shell.hotFloor.value.trim()) body.hot_floor_wei_hex = hotFloorWei;
    if (shell.hotTarget.value.trim()) body.hot_target_wei_hex = hotTargetWei;
    if (shell.hotOverflow.value.trim()) {
      body.hot_overflow_wei_hex = hotOverflowWei;
    }
    return body;
  }

  async function savePolicy(): Promise<void> {
    const shell = policyShell;
    if (!shell) return;
    const body = buildPolicyDto();
    if (!body) return;
    const sentences = treasuryPolicySummary(policyFromForm());
    const confirmed = await confirmDangerDialog({
      title: "Save treasury policy",
      body:
        "This changes what may execute on-chain. " +
        sentences.join(" ") +
        " The change is written to the audit log.",
      actionLabel: "Save policy",
    });
    if (!confirmed) return;
    setBusy(shell.saveButton, true);
    try {
      const response = await runtime.api.updateTreasuryPolicy(body);
      state.policy = response.policy;
      state.policyLoaded = true;
      state.policyFailure = null;
      policyFormDirty = false;
      policyFormFingerprint = null;
      state.policyNotice = { message: "Treasury policy saved.", tone: "success" };
      renderPolicyCard();
      // Eligibility follows the gates; the pause banner may have changed.
      void refreshPlans();
      renderQueueCard();
    } catch (error) {
      const failure = thrownFailure(error);
      if (failure.code === "validation_failed" && failure.fields?.length) {
        markPolicyFieldErrors(failure.fields);
        state.policyNotice = {
          message: "The daemon rejected some fields — they're highlighted in the form.",
          tone: "error",
        };
      } else {
        state.policyNotice = {
          message:
            failure.code === "vault_locked"
              ? "The vault is locked — unlock it in the Vault section, then save again."
              : failure.error || "Couldn't save the policy.",
          tone: "error",
        };
      }
      renderPolicyCard();
    } finally {
      setBusy(shell.saveButton, false);
    }
  }

  // ── Maintenance card ───────────────────────────────────────────────

  interface MaintenanceShell {
    result: HTMLElement;
  }
  let maintenanceShell: MaintenanceShell | null = null;

  function buildMaintenanceShell(card: HTMLElement): MaintenanceShell {
    clearChildren(card);
    const result = el("div", {
      class: "move-notice hidden",
      attrs: { role: "status", "data-move-region": "maintenance-result" },
    });
    const depositLimit = el("input", {
      class: "input-mid nums",
      attrs: {
        type: "number",
        min: "1",
        "aria-label": "Deposit refresh limit",
        "data-move-region": "maintenance-deposit-limit",
      },
    }) as HTMLInputElement;
    depositLimit.value = "50";
    const queueLimit = el("input", {
      class: "input-mid nums",
      attrs: {
        type: "number",
        min: "1",
        "aria-label": "Queue process limit",
        "data-move-region": "maintenance-queue-limit",
      },
    }) as HTMLInputElement;
    queueLimit.value = "20";
    const autoEnqueue = el("input", {
      attrs: { type: "checkbox", "data-move-region": "maintenance-auto-enqueue" },
    }) as HTMLInputElement;
    autoEnqueue.checked = true;
    const runAsync = el("input", {
      attrs: { type: "checkbox", "data-move-region": "maintenance-run-async" },
    }) as HTMLInputElement;
    const runButton = el("button", {
      class: "btn-primary",
      attrs: { type: "submit", "data-move-region": "maintenance-run" },
      text: "Run maintenance",
    }) as HTMLButtonElement;

    const form = el(
      "form",
      { class: "move-maintenance-form" },
      el(
        "label",
        { class: "field-label" },
        el("span", { text: "Deposit refresh limit" }),
        depositLimit,
      ),
      el(
        "label",
        { class: "field-label" },
        el("span", { text: "Queue process limit" }),
        queueLimit,
      ),
      el(
        "label",
        { class: "checkbox-row" },
        autoEnqueue,
        el("span", { text: " Auto-enqueue eligible sweeps" }),
      ),
      el(
        "label",
        {
          class: "checkbox-row",
          attrs: {
            title:
              "Start the cycle as a background operation you can cancel; progress shows in the queue card",
          },
        },
        runAsync,
        el("span", { text: " Run in background" }),
      ),
      runButton,
    );
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void runMaintenanceCycle(
        { depositLimit, queueLimit, autoEnqueue, runAsync },
        runButton,
      );
    });

    card.appendChild(
      el(
        "div",
        { class: "move-root" },
        el(
          "div",
          { class: "move-card-head" },
          el(
            "div",
            null,
            el("h3", { class: "move-card-title", text: "Maintenance" }),
            el("p", {
              class: "move-card-summary",
              text:
                "One local cycle: refresh deposits, auto-enqueue eligible sweeps, and drain the queue with current policy settings.",
            }),
          ),
        ),
        form,
        result,
      ),
    );
    return { result };
  }

  function renderMaintenanceCard(): void {
    const card = dom.cards.maintenanceCard;
    if (!card || !state.mounted) return;
    if (!maintenanceShell) maintenanceShell = buildMaintenanceShell(card);
    renderNotice(maintenanceShell.result, state.maintenanceNotice);
  }

  async function runMaintenanceCycle(
    inputs: {
      depositLimit: HTMLInputElement;
      queueLimit: HTMLInputElement;
      autoEnqueue: HTMLInputElement;
      runAsync: HTMLInputElement;
    },
    button: HTMLElement,
  ): Promise<void> {
    const confirmed = await confirmDangerDialog({
      title: "Run maintenance",
      body:
        "Run one local maintenance cycle now? It refreshes deposits, auto-enqueues eligible sweeps, and drains the queue with current policy settings — jobs that pass their checks are signed and broadcast on-chain.",
      actionLabel: "Run cycle",
    });
    if (!confirmed) return;
    const body: Record<string, unknown> = {
      deposit_refresh_limit: inputs.depositLimit.value.trim()
        ? parseInt(inputs.depositLimit.value.trim(), 10)
        : null,
      queue_process_limit: inputs.queueLimit.value.trim()
        ? parseInt(inputs.queueLimit.value.trim(), 10)
        : null,
      auto_enqueue: inputs.autoEnqueue.checked,
    };
    if (inputs.runAsync.checked) body.run_async = true;
    setBusy(button, true);
    try {
      const payload = await moveApi.runMaintenance(body);
      const failure = payloadFailure(payload);
      if (failure) {
        showFailureAsNotice("maintenanceNotice", failure);
        return;
      }
      const operation = payload.operation as { id?: string } | null | undefined;
      if (inputs.runAsync.checked && operation?.id) {
        state.maintenanceNotice = {
          message:
            "Maintenance cycle running in the background — progress shows in the queue card, and you can cancel it there.",
          tone: "info",
        };
      } else {
        state.maintenanceNotice = {
          message: maintenanceSummary(payload),
          tone: payload.status === "canceled" ? "warning" : "success",
        };
      }
      renderMaintenanceCard();
      await refreshQueue();
    } catch (error) {
      showFailureAsNotice("maintenanceNotice", thrownFailure(error));
    } finally {
      setBusy(button, false);
    }
  }

  // ── Routing, lifecycle ─────────────────────────────────────────────

  function setAuxCardsConcealed(concealed: boolean): void {
    for (const id of AUX_CARD_IDS) {
      dom.cards[id]?.classList.toggle("move-concealed", concealed);
    }
  }

  function focusCard(id: MoveCardId): void {
    const card = dom.cards[id] as
      | (HTMLElement & { focus?: () => void; scrollIntoView?: () => void })
      | undefined;
    if (!card) return;
    card.scrollIntoView?.();
    card.focus?.();
  }

  function applyRoute(route: Route): void {
    const planId =
      route.path[0] === "plan"
        ? (route.params.id ?? route.path[1] ?? null)
        : null;
    const nextMode: "list" | "detail" = planId ? "detail" : "list";
    const changed = nextMode !== state.mode || planId !== state.detailPlanId;
    state.mode = nextMode;
    state.detailPlanId = planId;
    setAuxCardsConcealed(nextMode === "detail");
    if (nextMode === "detail" && changed) {
      if (state.plans !== null) renderPlansCard(); // instant paint from cache
      // Unpaginated fetch so a deep link finds any plan; renders skeletons
      // synchronously when nothing is cached.
      void refreshPlans();
    } else {
      renderPlansCard();
    }
    if (route.path[0] === "queue") focusCard("queueCard");
    else if (route.path[0] === "policy") focusCard("policyCard");
  }

  function onStatusChange(
    status: StatusResponse | null,
    prev: StatusResponse | null,
  ): void {
    if (!status) return;
    if (status.locked && prev && !prev.locked) {
      const failure: ApiFailure = {
        code: "vault_locked",
        error: "The vault was locked.",
      };
      if (!state.plansFailure) state.plansFailure = failure;
      if (!state.queueFailure) state.queueFailure = failure;
      if (!state.policyFailure) state.policyFailure = failure;
      renderPlansCard();
      renderQueueCard();
      renderPolicyCard();
      return;
    }
    if (!status.locked && prev && prev.locked) {
      refreshAll();
    }
  }

  function resetShellRefs(): void {
    planListShell = null;
    queueShell = null;
    policyShell = null;
    maintenanceShell = null;
    detailFingerprint = null;
    policyFormFingerprint = null;
    policyFormDirty = false;
    activePreset = "custom";
    generateDetailsEl = null;
    generateDestinationInput = null;
    partyDestinationInputs = [];
    syncPartyRegionFn = null;
    plansFetching = false;
    plansFetchAgain = false;
    queueFetching = false;
    queueFetchAgain = false;
    dom.plansBody = null;
  }

  function mount(route: Route): void {
    runtime.router.register("move", "plan/:id");
    runtime.router.register("move", "queue");
    runtime.router.register("move", "policy");
    resetShellRefs();
    state.mounted = true;
    for (const id of MOVE_CARD_IDS) {
      const card = document.getElementById(id) as HTMLElement | null;
      if (!card) continue;
      dom.cards[id] = card;
      if (dom.savedHtml[id] === undefined) {
        dom.savedHtml[id] = card.innerHTML;
      }
      card.setAttribute("tabindex", "-1");
    }
    // Static shells: plans renders per state; queue/policy/maintenance build
    // once and patch their regions.
    if (dom.cards.plansCard) {
      clearChildren(dom.cards.plansCard);
      dom.plansBody = dom.cards.plansCard;
    }
    if (dom.cards.queueCard) queueShell = buildQueueShell(dom.cards.queueCard);
    if (dom.cards.policyCard) {
      policyShell = buildPolicyShell(dom.cards.policyCard);
    }
    if (dom.cards.maintenanceCard) {
      maintenanceShell = buildMaintenanceShell(dom.cards.maintenanceCard);
    }
    // First paint is skeletons, never a wrong "empty" flash.
    state.plansLoading = state.plans === null;
    state.queueLoading = state.queue === null;
    unsubscribes.push(
      runtime.store.subscribe("route", (next) => applyRoute(next)),
      runtime.store.subscribe("queueEvents", () => void refreshQueue()),
      runtime.store.subscribe("operations", () => renderQueueOps()),
      runtime.store.subscribe("resync", () => refreshAll()),
      runtime.store.subscribe("status", (next, prev) =>
        onStatusChange(next, prev),
      ),
    );
    applyRoute(route);
    refreshAll();
    renderPolicyCard();
    renderMaintenanceCard();
  }

  function unmount(): void {
    state.mounted = false;
    for (const unsubscribe of unsubscribes.splice(0)) unsubscribe();
    for (const id of MOVE_CARD_IDS) {
      const card = dom.cards[id];
      if (!card) continue;
      const saved = dom.savedHtml[id];
      // Clear first: in a real browser the innerHTML assignment below
      // replaces children; in the fake-DOM harness it only sets the property,
      // so the explicit clear keeps both consistent.
      clearChildren(card);
      if (saved !== undefined) card.innerHTML = saved;
      card.classList.remove("move-concealed");
      card.removeAttribute("tabindex");
    }
    resetShellRefs();
    state.mode = "list";
    state.detailPlanId = null;
    state.plans = null;
    state.plansPagination = null;
    state.plansOffset = 0;
    state.plansFailure = null;
    state.plansLoading = false;
    state.queue = null;
    state.queuePagination = null;
    state.queueOffset = 0;
    state.queueFailure = null;
    state.queueLoading = false;
    state.policy = null;
    state.policyLoaded = false;
    state.policyFailure = null;
    state.parties = [];
    state.chains = [];
    state.planNotice = null;
    state.queueNotice = null;
    state.policyNotice = null;
    state.maintenanceNotice = null;
  }

  return { id: "move", migrated: true, mount, unmount };
}
