/**
 * destinations/Overview.ts — the Overview destination controller (plan 4.3:
 * "what needs my attention?").
 *
 * Takes over the legacy hero card (#statusCard) while mounted and renders:
 *
 * - a compact workspace-status strip (lock state, active compartment,
 *   providers/wallets/addresses/keys/secrets counts) driven by the store's
 *   `status` slice plus one batched resource refresh;
 * - a freshness watermark ("data current as of …") computed from the newest
 *   per-resource timestamps, with the events-transport dot from `sync`;
 * - a ranked attention queue (`.attention-item` rows, `[data-tier]`):
 *   `operator_action_required` queue jobs (danger), review-required plans
 *   (review, deep link `#/move/plan/{id}`), failed/warning self-check
 *   domains, critical/high risk findings (danger; medium → review), stale
 *   or never-run scans, and failed background operations — each with ONE
 *   primary action that deep-links into the owning destination;
 * - the calm quiet-tier empty state when nothing needs attention (the empty
 *   state IS the design) with an on-demand self-check as its next action;
 * - a recent audit digest (humanized descriptions, "show more" pagination
 *   via the `limit` parameter).
 *
 * Live-ness comes from the store only: `status` re-renders the strip and
 * re-fetches on lock/compartment changes, `operations` feeds the failed-ops
 * item, `queueEvents` triggers a queue-jobs refetch, `resync` re-fetches
 * everything, `sync` drives the watermark dot. No new pollers.
 *
 * Refresh failures keep the last good data on screen and surface ONE
 * persistent banner (not a toast) naming the stale resources, with Retry.
 * `vault_locked` swaps the whole view to unlock guidance instead.
 */

import type {
  AuditEvent,
  ChainProfile,
  ConsolidationPlan,
  Operation,
  QueueJob,
  SelfCheckResult,
  StatusResponse,
  TreasuryOverviewResponse,
} from "../contracts";
import { requestWithSession } from "../api/session";
import { ApiError, apiFailure } from "../core/api";
import { el, renderList } from "../core/dom";
import type { CoreRuntime } from "../core/live";
import type { DestinationController, Route } from "../core/router";
import type { Unsubscribe } from "../core/store";
import { chainLabel, formatTimestamp } from "../render/format";

/** The legacy card this destination takes over (see index fragment). */
export const OVERVIEW_HOST_ID = "statusCard";

/** Scans older than this count as stale in the attention queue. */
export const STALE_SCAN_THRESHOLD_SECS = 24 * 60 * 60;

/** Ambient self-checks re-probe at most this often (mirrors legacy TTL). */
export const SELF_CHECK_TTL_MS = 5 * 60_000;

/** Audit digest page size; "show more" grows the limit by this step. */
export const AUDIT_PAGE_SIZE = 10;

/** Client-side cap for the audit digest (daemon enforces its own max). */
const AUDIT_LIMIT_CAP = 100;

/** Individual plan rows before the remainder collapses into one aggregate. */
const PLAN_ROW_CAP = 5;

type ResourceName =
  | "plans"
  | "queue"
  | "treasury"
  | "providers"
  | "chains"
  | "inventory"
  | "audit"
  | "selfcheck";

const RESOURCE_LABELS: Record<ResourceName, string> = {
  plans: "plans",
  queue: "the queue",
  treasury: "workspace totals",
  providers: "providers",
  chains: "the chain registry",
  inventory: "wallet inventory",
  audit: "recent activity",
  selfcheck: "self-check",
};

// ── Thin API wrappers (endpoints the typed core client does not cover) ──
// Kept inside this destination module per the migration rules; same error
// envelope semantics as core/api.ts (throw ApiError on `error` payloads).

interface ProviderProfileWire {
  name?: string;
}

/** `GET /api/profiles/evm` — provider profile list (count for the strip). */
async function listEvmProviderProfiles(): Promise<ProviderProfileWire[]> {
  const payload = await requestWithSession("GET", "/api/profiles/evm");
  if (payload && payload.error) {
    throw new ApiError({
      code: payload.code ?? "unknown",
      error: String(payload.error),
    });
  }
  return (payload.profiles as ProviderProfileWire[] | undefined) ?? [];
}

/** `GET /api/chains` — chain registry for human chain names. */
async function listChainProfiles(): Promise<ChainProfile[]> {
  const payload = await requestWithSession("GET", "/api/chains");
  if (payload && payload.error) {
    throw new ApiError({
      code: payload.code ?? "unknown",
      error: String(payload.error),
    });
  }
  return (payload.profiles as ChainProfile[] | undefined) ?? [];
}

// ── Pure helpers (exported for tests) ─────────────────────────────────

/** Short relative age ("5m ago") for unix-second timestamps. */
export function relativeAge(
  unixSecs: number | null | undefined,
  nowMs: number = Date.now(),
): string {
  if (!unixSecs) return "never";
  const deltaMs = nowMs - unixSecs * 1000;
  if (deltaMs < 45_000) return "just now";
  const minutes = Math.floor(deltaMs / 60_000);
  if (minutes < 60) return minutes + "m ago";
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return hours + "h ago";
  const days = Math.floor(hours / 24);
  return days + "d ago";
}

/** "inventory_scan_evm" → "Inventory scan evm" (fallback for unknown enums). */
export function humanToken(value: string): string {
  const words = String(value || "").replace(/[_.]+/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : "Unknown";
}

/** Humanized audit descriptions, ported from the legacy app.ts label map. */
const AUDIT_LABELS: Record<string, string> = {
  "unlock.passphrase": "Unlocked with passphrase",
  "unlock.fido2": "Unlocked with FIDO2",
  "lock.all": "Locked all compartments",
  "session.revoke": "Revoked session",
  "compartment.add": "Added compartment",
  "compartment.init": "Initialized compartment",
  "compartment.remove": "Removed compartment",
  "compartment.switch": "Switched compartment",
  "api_key.set": "Stored API key",
  "api_key.delete": "Deleted API key",
  "secret.set": "Stored encrypted secret",
  "secret.delete": "Deleted encrypted secret",
  "secret.push": "Pushed secret between compartments",
  "profiles.eth_xpub_wallet.upsert": "Saved xpub wallet profile",
  "profiles.eth_xpub_wallet.delete": "Deleted xpub wallet profile",
  "profiles.eth_seed_wallet.upsert": "Imported seed wallet profile",
  "profiles.eth_seed_wallet.delete": "Deleted seed wallet profile",
  "wallet_inventory.risk_catalog.upsert": "Saved risk catalog entry",
  "wallet_inventory.risk_catalog.delete": "Deleted risk catalog entry",
  "wallet.eth_xpub.export": "Exported xpub receive branch",
  "fido2.setup": "Completed FIDO2 setup",
  "fido2.register": "Registered FIDO2 key",
  "fido2.register_poison": "Registered poison FIDO2 key",
  "fido2.remove": "Removed FIDO2 key",
  "snapshot.export": "Exported encrypted snapshot",
  "snapshot.restore": "Restored encrypted snapshot",
};

export function describeAuditEvent(event: AuditEvent): string {
  const details = event.details ?? {};
  const base = AUDIT_LABELS[event.kind] ?? humanToken(event.kind);
  const suffix =
    stringDetail(details.label) ??
    stringDetail(details.key) ??
    stringDetail(details.name) ??
    stringDetail(details.address) ??
    stringDetail(details.wallet_profile) ??
    countDetail(details.compartment_count, "compartments") ??
    countDetail(details.count, "compartments") ??
    countDetail(details.file_count, "files");
  return suffix ? base + " — " + suffix : base;
}

function stringDetail(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}

function countDetail(value: unknown, noun: string): string | null {
  return typeof value === "number" && value > 0 ? value + " " + noun : null;
}

// ── Attention queue model ─────────────────────────────────────────────

export interface AttentionItem {
  /** Stable key for the keyed list. */
  key: string;
  tier: "danger" | "review";
  /** Recency input to ranking (unix seconds; 0 = unknown). */
  rankUnix: number;
  title: string;
  body: string;
  actionLabel: string;
  /** Deep link into the owning destination. */
  href: string;
}

export interface OverviewSelfCheck {
  status: string;
  failCount: number;
  warnCount: number;
  failDomains: string[];
  warnDomains: string[];
  atUnix: number;
}

/** Everything the attention computation needs (pure → unit-testable). */
export interface AttentionInput {
  locked: boolean;
  plans: ConsolidationPlan[] | null;
  chains: ChainProfile[] | null;
  queueJobs: QueueJob[] | null;
  queueTotal: number;
  treasury: TreasuryOverviewResponse | null;
  newestScanUnix: number | null;
  trackedAddressCount: number | null;
  selfCheck: OverviewSelfCheck | null;
  failedOperations: Operation[];
}

function newestUnix(values: Array<number | null | undefined>): number {
  let newest = 0;
  for (const value of values) {
    if (typeof value === "number" && value > newest) newest = value;
  }
  return newest;
}

function summarizeSelfCheck(
  response: { status: string; generated_at_unix: number; checks: SelfCheckResult[] },
): OverviewSelfCheck {
  const failed = response.checks.filter((check) => check.status === "fail");
  const warned = response.checks.filter((check) => check.status === "warn");
  return {
    status: response.status,
    failCount: failed.length,
    warnCount: warned.length,
    failDomains: Array.from(new Set(failed.map((check) => check.domain))),
    warnDomains: Array.from(new Set(warned.map((check) => check.domain))),
    atUnix: response.generated_at_unix,
  };
}

export function computeAttentionItems(
  input: AttentionInput,
  nowMs: number = Date.now(),
): AttentionItem[] {
  const items: AttentionItem[] = [];

  // `operator_action_required` queue jobs — the loudest signal: execution
  // stopped and only a human can move it (reverted receipts, gate denials).
  const jobs = input.queueJobs ?? [];
  if (jobs.length > 0) {
    const count = Math.max(input.queueTotal, jobs.length);
    const newest = jobs.reduce(
      (best, job) => (job.updated_at_unix > best.updated_at_unix ? job : best),
      jobs[0],
    );
    const newestError = newest.last_error ? String(newest.last_error) : null;
    items.push({
      key: "queue-operator-action",
      tier: "danger",
      rankUnix: newestUnix(jobs.map((job) => job.updated_at_unix)),
      title:
        count +
        (count === 1 ? " queue job needs" : " queue jobs need") +
        " operator action",
      body: newestError
        ? "Latest: " + newestError
        : "Execution stopped — open the queue to see why and decide the next step.",
      actionLabel: "Open queue",
      href: "#/move",
    });
  }

  // Plans waiting for review — one row each (deep link into Move), the
  // remainder aggregated so the queue stays scannable.
  const plans = input.plans ?? [];
  plans.slice(0, PLAN_ROW_CAP).forEach((plan) => {
    const summary = plan.summary;
    const blocked =
      summary.blocked_steps > 0 ? " · " + summary.blocked_steps + " blocked" : "";
    items.push({
      key: "plan-" + plan.id,
      tier: "review",
      rankUnix: plan.updated_at_unix ?? 0,
      title: "Plan on " + chainLabel(plan.chain_id, input.chains) + " needs review",
      body:
        summary.review_required_steps +
        " of " +
        summary.total_steps +
        " steps need your approval" +
        blocked +
        " · updated " +
        relativeAge(plan.updated_at_unix, nowMs),
      actionLabel: "Review plan",
      href: "#/move/plan/" + encodeURIComponent(plan.id),
    });
  });
  if (plans.length > PLAN_ROW_CAP) {
    const remaining = plans.length - PLAN_ROW_CAP;
    items.push({
      key: "plans-aggregate",
      tier: "review",
      rankUnix: newestUnix(plans.slice(PLAN_ROW_CAP).map((plan) => plan.updated_at_unix)),
      title: remaining + (remaining === 1 ? " more plan needs" : " more plans need") + " review",
      body: "Open Move to work through the full list of plans waiting for review.",
      actionLabel: "Open Move",
      href: "#/move",
    });
  }

  // Self-check: failing domains are danger, warn-only is review.
  const selfCheck = input.selfCheck;
  if (selfCheck && selfCheck.failCount > 0) {
    items.push({
      key: "selfcheck-fail",
      tier: "danger",
      rankUnix: selfCheck.atUnix,
      title: "Self-check is failing",
      body:
        selfCheck.failCount +
        (selfCheck.failCount === 1 ? " check failed" : " checks failed") +
        (selfCheck.failDomains.length
          ? " in " + selfCheck.failDomains.map(humanToken).join(", ")
          : "") +
        " · ran " +
        relativeAge(selfCheck.atUnix, nowMs),
      actionLabel: "Open diagnostics",
      href: "#/vault",
    });
  } else if (selfCheck && selfCheck.warnCount > 0) {
    items.push({
      key: "selfcheck-warn",
      tier: "review",
      rankUnix: selfCheck.atUnix,
      title: "Self-check has warnings",
      body:
        selfCheck.warnCount +
        (selfCheck.warnCount === 1 ? " check warns" : " checks warn") +
        (selfCheck.warnDomains.length
          ? " in " + selfCheck.warnDomains.map(humanToken).join(", ")
          : "") +
        " · ran " +
        relativeAge(selfCheck.atUnix, nowMs),
      actionLabel: "Open diagnostics",
      href: "#/vault",
    });
  }

  // Risk findings by severity (counts from the treasury overview aggregate).
  const risk = input.treasury?.risk ?? null;
  if (risk) {
    const severe = (risk.critical_findings ?? 0) + (risk.high_findings ?? 0);
    if (severe > 0) {
      items.push({
        key: "risk-severe",
        tier: "danger",
        rankUnix: input.treasury?.generated_at_unix ?? 0,
        title: severe + (severe === 1 ? " high-severity risk finding" : " high-severity risk findings"),
        body:
          (risk.critical_findings ?? 0) +
          " critical · " +
          (risk.high_findings ?? 0) +
          " high — review before moving funds.",
        actionLabel: "Review findings",
        href: "#/portfolio",
      });
    } else if ((risk.medium_findings ?? 0) > 0) {
      items.push({
        key: "risk-medium",
        tier: "review",
        rankUnix: input.treasury?.generated_at_unix ?? 0,
        title:
          risk.medium_findings +
          (risk.medium_findings === 1 ? " medium risk finding" : " medium risk findings"),
        body: "Nothing critical — review the findings when convenient.",
        actionLabel: "Review findings",
        href: "#/portfolio",
      });
    }
  }

  // Stale or never-run scans: balances and holdings may be out of date.
  const nowSecs = Math.floor(nowMs / 1000);
  if ((input.trackedAddressCount ?? 0) > 0) {
    if (!input.newestScanUnix) {
      items.push({
        key: "scan-never",
        tier: "review",
        rankUnix: 0,
        title: "Addresses have never been scanned",
        body: "Run a scan so balances and holdings reflect the chain.",
        actionLabel: "Open portfolio",
        href: "#/portfolio",
      });
    } else if (nowSecs - input.newestScanUnix > STALE_SCAN_THRESHOLD_SECS) {
      items.push({
        key: "scan-stale",
        tier: "review",
        rankUnix: input.newestScanUnix,
        title: "Address data is stale",
        body:
          "The newest scan is " +
          relativeAge(input.newestScanUnix, nowMs) +
          " old — balances and holdings may be out of date.",
        actionLabel: "Open portfolio",
        href: "#/portfolio",
      });
    }
  }

  // Failed background operations (from the operations slice).
  const failedOps = input.failedOperations;
  if (failedOps.length === 1) {
    const op = failedOps[0];
    items.push({
      key: "op-" + op.id,
      tier: "review",
      rankUnix: op.updated_at_unix ?? 0,
      title: "Background job failed",
      body:
        humanToken(op.kind) +
        (op.error ? " — " + op.error : " — see Portfolio for details."),
      actionLabel: "Open portfolio",
      href: "#/portfolio",
    });
  } else if (failedOps.length > 1) {
    const newest = failedOps.reduce(
      (best, op) => (op.updated_at_unix > best.updated_at_unix ? op : best),
      failedOps[0],
    );
    items.push({
      key: "ops-failed",
      tier: "review",
      rankUnix: newestUnix(failedOps.map((op) => op.updated_at_unix)),
      title: failedOps.length + " background jobs failed",
      body: "Latest: " + humanToken(newest.kind) + (newest.error ? " — " + newest.error : ""),
      actionLabel: "Open portfolio",
      href: "#/portfolio",
    });
  }

  // Rank: danger first, then by recency (unknown recency sinks).
  const tierWeight = { danger: 0, review: 1 } as const;
  return items.sort(
    (a, b) => tierWeight[a.tier] - tierWeight[b.tier] || b.rankUnix - a.rankUnix,
  );
}

/** Newest timestamp across loaded resources (unix seconds; null if none). */
export function freshnessWatermark(input: {
  treasury: TreasuryOverviewResponse | null;
  newestScanUnix: number | null;
  selfCheck: OverviewSelfCheck | null;
  audit: AuditEvent[] | null;
  plans: ConsolidationPlan[] | null;
  queueJobs: QueueJob[] | null;
}): number | null {
  const newest = newestUnix([
    input.treasury?.generated_at_unix ?? null,
    input.newestScanUnix,
    input.selfCheck?.atUnix ?? null,
    input.audit && input.audit.length ? input.audit[0].created_at_unix : null,
    newestUnix((input.plans ?? []).map((plan) => plan.updated_at_unix)),
    newestUnix((input.queueJobs ?? []).map((job) => job.updated_at_unix)),
  ]);
  return newest > 0 ? newest : null;
}

// ── Controller ────────────────────────────────────────────────────────

interface ResourceState {
  loadedOnce: boolean;
  locked: boolean;
  plans: ConsolidationPlan[] | null;
  chains: ChainProfile[] | null;
  queueJobs: QueueJob[] | null;
  queueTotal: number;
  treasury: TreasuryOverviewResponse | null;
  providerCount: number | null;
  trackedAddressCount: number | null;
  newestScanUnix: number | null;
  selfCheck: OverviewSelfCheck | null;
  selfCheckAtMs: number;
  audit: AuditEvent[] | null;
  auditLimit: number;
  failures: Map<ResourceName, { code: string; message: string; action?: string }>;
  lastFullRefreshMs: number | null;
}

interface OverviewNodes {
  root: HTMLElement;
  bannerSlot: HTMLElement;
  watermarkDot: HTMLElement;
  watermarkText: HTMLElement;
  refreshButton: HTMLButtonElement;
  lockRow: HTMLElement;
  statsRow: HTMLElement;
  attentionList: HTMLElement;
  attentionEmpty: HTMLElement;
  attentionEmptyTitle: HTMLElement;
  attentionEmptyBody: HTMLElement;
  attentionSkeleton: HTMLElement;
  selfCheckButton: HTMLButtonElement;
  unlockLink: HTMLAnchorElement;
  auditList: HTMLElement;
  auditEmpty: HTMLElement;
  auditSkeleton: HTMLElement;
  moreRow: HTMLElement;
  moreButton: HTMLButtonElement;
}

/** Remove every child (fake-DOM safe: no `firstChild`, no `innerHTML=""`). */
function clearChildren(node: Element): void {
  while (node.childNodes.length > 0) {
    (node.childNodes[0] as HTMLElement).remove();
  }
}

/** Positional child access (fake-DOM safe: no class `querySelector`). */
function childAt(node: Element, index: number): HTMLElement {
  return node.children[index] as HTMLElement;
}

export function createOverviewDestination(
  runtime: CoreRuntime,
): DestinationController {
  const { store, api } = runtime;

  let mounted = false;
  let generation = 0;
  let host: HTMLElement | null = null;
  let savedHostHtml = "";
  let nodes: OverviewNodes | null = null;
  const unsubscribes: Unsubscribe[] = [];
  let refreshInFlight = false;
  let refreshQueued = false;

  const state: ResourceState = {
    loadedOnce: false,
    locked: false,
    plans: null,
    chains: null,
    queueJobs: null,
    queueTotal: 0,
    treasury: null,
    providerCount: null,
    trackedAddressCount: null,
    newestScanUnix: null,
    selfCheck: null,
    selfCheckAtMs: 0,
    audit: null,
    auditLimit: AUDIT_PAGE_SIZE,
    failures: new Map(),
    lastFullRefreshMs: null,
  };

  // ── Data loading ────────────────────────────────────────────────────

  async function loadResource(
    name: ResourceName,
    fn: () => Promise<void>,
  ): Promise<void> {
    try {
      await fn();
      state.failures.delete(name);
    } catch (error) {
      const failure = apiFailure(error);
      state.failures.set(name, {
        code: failure?.code ?? "unknown",
        message: failure?.error ?? String(error),
        action: failure?.action,
      });
    }
  }

  /** Silent, TTL'd self-check; explicit runs pass force=true. */
  async function ensureSelfCheck(force: boolean): Promise<void> {
    if (
      !force &&
      state.selfCheck &&
      Date.now() - state.selfCheckAtMs < SELF_CHECK_TTL_MS
    ) {
      return;
    }
    const response = await api.runSelfCheck();
    state.selfCheck = summarizeSelfCheck(response);
    state.selfCheckAtMs = Date.now();
  }

  async function refreshAll(): Promise<void> {
    if (refreshInFlight) {
      refreshQueued = true;
      return;
    }
    refreshInFlight = true;
    setBusy(nodes?.refreshButton ?? null, true, "Refreshing…");
    const gen = generation;
    try {
      await Promise.all([
        loadResource("plans", async () => {
          const response = await api.listPlans({
            status: "review_required",
            sort: "updated",
            order: "desc",
            limit: 20,
          });
          state.plans = response.plans ?? [];
        }),
        loadResource("queue", async () => {
          const response = await api.listQueueJobs({
            state: "operator_action_required",
            sort: "updated",
            order: "desc",
            limit: 20,
          });
          state.queueJobs = response.jobs ?? [];
          state.queueTotal =
            response.pagination?.total ?? (response.jobs ?? []).length;
        }),
        loadResource("treasury", async () => {
          state.treasury = await api.getTreasuryOverview();
        }),
        loadResource("providers", async () => {
          state.providerCount = (await listEvmProviderProfiles()).length;
        }),
        loadResource("chains", async () => {
          state.chains = await listChainProfiles();
        }),
        loadResource("inventory", async () => {
          const response = await api.listInventoryWallets({
            sort: "last_scanned",
            order: "desc",
            limit: 1,
          });
          const newest = (response.addresses ?? [])[0] ?? null;
          state.newestScanUnix = newest ? newest.last_checked_at_unix : null;
          state.trackedAddressCount =
            response.pagination?.total ?? (response.addresses ?? []).length;
        }),
        loadResource("audit", async () => {
          const response = await api.listAudit({ limit: state.auditLimit });
          state.audit = response.events ?? [];
        }),
        loadResource("selfcheck", () => ensureSelfCheck(false)),
      ]);
    } finally {
      refreshInFlight = false;
      const queued = refreshQueued;
      refreshQueued = false;
      if (mounted && gen === generation) {
        if (state.failures.size === 0) {
          state.lastFullRefreshMs = Date.now();
        }
        state.loadedOnce = true;
        setBusy(nodes?.refreshButton ?? null, false, "Refresh");
        renderDynamic();
      }
      // A refresh requested mid-flight (store event, remount) still runs —
      // even when this flight's render was skipped as stale.
      if (queued && mounted) {
        void refreshAll();
      }
    }
  }

  async function refreshQueueOnly(): Promise<void> {
    const gen = generation;
    await loadResource("queue", async () => {
      const response = await api.listQueueJobs({
        state: "operator_action_required",
        sort: "updated",
        order: "desc",
        limit: 20,
      });
      state.queueJobs = response.jobs ?? [];
      state.queueTotal =
        response.pagination?.total ?? (response.jobs ?? []).length;
    });
    if (!mounted || gen !== generation) return;
    renderBanner();
    renderAttention();
    renderWatermark();
  }

  async function loadMoreAudit(): Promise<void> {
    if (state.auditLimit >= AUDIT_LIMIT_CAP) return;
    state.auditLimit = Math.min(
      state.auditLimit + AUDIT_PAGE_SIZE,
      AUDIT_LIMIT_CAP,
    );
    setBusy(nodes?.moreButton ?? null, true, "Loading…");
    const gen = generation;
    await loadResource("audit", async () => {
      const response = await api.listAudit({ limit: state.auditLimit });
      state.audit = response.events ?? [];
    });
    if (!mounted || gen !== generation) return;
    setBusy(nodes?.moreButton ?? null, false, "Show more");
    renderAudit();
    renderBanner();
  }

  async function runSelfCheckExplicit(): Promise<void> {
    setBusy(nodes?.selfCheckButton ?? null, true, "Running…");
    const gen = generation;
    await loadResource("selfcheck", () => ensureSelfCheck(true));
    if (!mounted || gen !== generation) return;
    setBusy(nodes?.selfCheckButton ?? null, false, "Run self-check");
    renderAttention();
    renderBanner();
    renderWatermark();
  }

  // ── Rendering ───────────────────────────────────────────────────────

  function setBusy(
    button: HTMLButtonElement | null,
    busy: boolean,
    label: string,
  ): void {
    if (!button) return;
    button.disabled = busy;
    button.textContent = label;
    if (busy) button.setAttribute("aria-busy", "true");
    else button.removeAttribute("aria-busy");
  }

  function setVisible(node: HTMLElement, visible: boolean): void {
    node.classList.toggle("hidden", !visible);
  }

  function skeletonStack(blocks: number): HTMLElement {
    const stack = el("div", { class: "dest-overview-skeleton-stack" });
    for (let i = 0; i < blocks; i += 1) {
      stack.appendChild(el("div", { class: "skeleton skeleton-block" }));
    }
    return stack;
  }

  function attentionInput(): AttentionInput {
    return {
      locked: state.locked,
      plans: state.plans,
      chains: state.chains,
      queueJobs: state.queueJobs,
      queueTotal: state.queueTotal,
      treasury: state.treasury,
      newestScanUnix: state.newestScanUnix,
      trackedAddressCount: state.trackedAddressCount,
      selfCheck: state.selfCheck,
      failedOperations: store
        .get("operations")
        .filter((op) => op.state === "failed"),
    };
  }

  function renderDynamic(): void {
    renderBanner();
    renderStatusStrip();
    renderAttention();
    renderAudit();
    renderWatermark();
  }

  function lockedMode(): boolean {
    if (state.locked) return true;
    for (const failure of state.failures.values()) {
      if (failure.code === "vault_locked") return true;
    }
    return false;
  }

  /** Persistent stale-data banner (never a toast). */
  function renderBanner(): void {
    if (!nodes) return;
    const slot = nodes.bannerSlot;
    clearChildren(slot);

    if (!state.loadedOnce || state.failures.size === 0 || lockedMode()) return;

    const names = Array.from(state.failures.keys()).map(
      (name) => RESOURCE_LABELS[name],
    );
    const unavailable = Array.from(state.failures.values()).some(
      (failure) => failure.code === "unavailable",
    );
    const lastFull = state.lastFullRefreshMs
      ? " Last full refresh " + relativeAge(state.lastFullRefreshMs / 1000) + "."
      : " No full refresh has completed yet.";

    const banner = el(
      "div",
      {
        class: "dest-overview-banner",
        dataset: { tier: "review" },
        attrs: { role: "alert" },
      },
      el(
        "div",
        { class: "dest-overview-banner-copy" },
        el("p", {
          class: "dest-overview-banner-title",
          text: unavailable
            ? "The daemon is not responding"
            : "Some workspace data may be out of date",
        }),
        el("p", {
          class: "dest-overview-banner-body",
          text:
            "Couldn't refresh " +
            names.join(", ") +
            ". The last good data stays on screen." +
            lastFull,
        }),
      ),
      el("button", {
        class: "btn-ghost btn-small",
        text: "Retry",
        attrs: { type: "button" },
        on: { click: () => void refreshAll() },
      }),
    );
    slot.appendChild(banner);
  }

  /** Compact workspace status: lock/compartment + counts. */
  function renderStatusStrip(): void {
    if (!nodes) return;
    const status = store.get("status");

    clearChildren(nodes.lockRow);
    clearChildren(nodes.statsRow);

    if (!status && !state.loadedOnce) {
      nodes.lockRow.appendChild(skeletonStack(1));
      return;
    }

    const locked = status ? status.locked : state.locked;
    nodes.lockRow.appendChild(
      el("span", {
        class: "pill " + (locked ? "pill-warn" : "pill-good"),
        text: locked ? "Locked" : "Unlocked",
      }),
    );
    const active = status?.active_compartment ?? null;
    nodes.lockRow.appendChild(
      el("span", {
        class: "dest-overview-comp",
        text: active
          ? "Compartment " + (active.compartment_label || "#" + active.compartment_id)
          : "No active compartment",
      }),
    );
    const unlockedCount = status?.unlocked_compartments?.length ?? 0;
    nodes.lockRow.appendChild(
      el("span", {
        class: "dest-overview-comp-meta",
        text:
          unlockedCount === 1
            ? "1 compartment unlocked in this session"
            : unlockedCount + " compartments unlocked in this session",
      }),
    );

    const wallets = walletCount(state.treasury);
    const stats: Array<[string, string]> = [
      ["Providers", state.providerCount !== null ? String(state.providerCount) : "-"],
      ["Wallets", wallets],
      [
        "Addresses",
        state.trackedAddressCount !== null
          ? String(state.trackedAddressCount) + fundedSuffix(state.treasury)
          : "-",
      ],
      [
        "Connection keys",
        active ? String(active.api_key_count ?? 0) : "-",
      ],
      [
        "Secrets",
        active
          ? active.secret_count != null
            ? String(active.secret_count)
            : "locked"
          : "-",
      ],
    ];
    for (const [label, value] of stats) {
      nodes.statsRow.appendChild(
        el(
          "div",
          { class: "stat" },
          el("div", { class: "value", text: value }),
          el("div", { class: "label", text: label }),
        ),
      );
    }
  }

  function walletCount(treasury: TreasuryOverviewResponse | null): string {
    if (!treasury) return "-";
    if (!treasury.groups) return "-";
    const profiles = new Set(
      treasury.groups.map((group) => group.wallet_family + "/" + group.wallet_profile),
    );
    return String(profiles.size);
  }

  function fundedSuffix(treasury: TreasuryOverviewResponse | null): string {
    if (!treasury) return "";
    return " (" + (treasury.funded_address_count ?? 0) + " funded)";
  }

  /** The ranked attention queue (or the calm empty state). */
  function renderAttention(): void {
    if (!nodes) return;
    const items = computeAttentionItems(attentionInput());
    const locked = lockedMode();

    setVisible(nodes.attentionSkeleton, !state.loadedOnce);
    setVisible(nodes.attentionList, state.loadedOnce && !locked && items.length > 0);
    setVisible(
      nodes.attentionEmpty,
      state.loadedOnce && (locked || items.length === 0),
    );

    if (locked) {
      nodes.attentionEmpty.dataset.tier = "review";
      nodes.attentionEmptyTitle.textContent = "The vault is locked";
      nodes.attentionEmptyBody.textContent =
        "Unlock the vault to load the workspace — plans, queue state, scans, and findings all need the vault open.";
      setVisible(nodes.selfCheckButton, false);
      setVisible(nodes.unlockLink, true);
      renderList(nodes.attentionList, [], attentionKey, renderAttentionRow);
      return;
    }
    nodes.attentionEmpty.dataset.tier = "quiet";
    nodes.attentionEmptyTitle.textContent = "Nothing needs your attention";
    nodes.attentionEmptyBody.textContent = state.selfCheck
      ? "Queue is clear, no plans wait for review, self-check is " +
        (state.selfCheck.status === "pass" ? "green" : "not failing") +
        ", and scans are fresh. This screen stays quiet until something changes."
      : "Queue is clear, no plans wait for review, and scans are fresh. Run a self-check to prove every configured input still works.";
    setVisible(nodes.selfCheckButton, true);
    setVisible(nodes.unlockLink, false);

    renderList(nodes.attentionList, items, attentionKey, renderAttentionRow);
  }

  function attentionKey(item: AttentionItem): string {
    return item.key;
  }

  function renderAttentionRow(
    item: AttentionItem,
    existing: HTMLElement | null,
  ): HTMLElement {
    const row = existing ?? buildAttentionRow();
    row.dataset.tier = item.tier;
    const main = childAt(row, 0);
    childAt(main, 0).textContent = item.title;
    childAt(main, 1).textContent = item.body;
    const action = childAt(row, 1);
    action.textContent = item.actionLabel;
    action.setAttribute("href", item.href);
    return row;
  }

  function buildAttentionRow(): HTMLElement {
    return el(
      "li",
      { class: "attention-item", dataset: { tier: "review" } },
      el(
        "div",
        { class: "attention-item-main" },
        el("p", { class: "attention-item-title" }),
        el("p", { class: "attention-item-body" }),
      ),
      el("a", {
        class: "btn-primary btn-small attention-item-action",
        attrs: { href: "#/move" },
      }),
    );
  }

  /** Recent audit digest with "show more" pagination. */
  function renderAudit(): void {
    if (!nodes) return;
    const events = state.audit ?? [];

    setVisible(nodes.auditSkeleton, !state.loadedOnce);
    setVisible(nodes.auditList, state.loadedOnce && events.length > 0);
    setVisible(nodes.auditEmpty, state.loadedOnce && events.length === 0);

    renderList(
      nodes.auditList,
      events.map((event, index) => ({ event, key: auditKey(event, index, events) })),
      (row) => row.key,
      (row, existing) => renderAuditRow(row.event, existing),
    );

    const exhausted =
      events.length < state.auditLimit || state.auditLimit >= AUDIT_LIMIT_CAP;
    setVisible(nodes.moreRow, state.loadedOnce && events.length > 0 && !exhausted);
  }

  /** Stable-enough key: timestamp+kind+compartment plus collision index. */
  function auditKey(event: AuditEvent, index: number, events: AuditEvent[]): string {
    const base =
      event.created_at_unix +
      ":" +
      event.kind +
      ":" +
      (event.compartment_id ?? "g");
    let collision = 0;
    for (let i = 0; i < index; i += 1) {
      const other = events[i];
      if (
        other.created_at_unix === event.created_at_unix &&
        other.kind === event.kind &&
        (other.compartment_id ?? "g") === (event.compartment_id ?? "g")
      ) {
        collision += 1;
      }
    }
    return base + ":" + collision;
  }

  function renderAuditRow(
    event: AuditEvent,
    existing: HTMLElement | null,
  ): HTMLElement {
    const row = existing ?? buildAuditRow();
    childAt(row, 0).textContent = describeAuditEvent(event);
    const meta = childAt(row, 1);
    meta.textContent =
      relativeAge(event.created_at_unix) +
      " · " +
      (event.compartment_id != null
        ? "compartment " + event.compartment_id
        : "global");
    meta.setAttribute("title", formatTimestamp(event.created_at_unix));
    return row;
  }

  function buildAuditRow(): HTMLElement {
    return el(
      "li",
      { class: "dest-overview-audit-row" },
      el("p", { class: "dest-overview-audit-text" }),
      el("p", { class: "dest-overview-audit-meta" }),
    );
  }

  /** Freshness watermark + transport dot. */
  function renderWatermark(): void {
    if (!nodes) return;
    const sync = store.get("sync");
    nodes.watermarkDot.dataset.state =
      sync.transport === "sse"
        ? "live"
        : sync.transport === "connecting"
          ? "busy"
          : sync.transport === "error"
            ? "error"
            : "paused";

    const newest = freshnessWatermark({
      treasury: state.treasury,
      newestScanUnix: state.newestScanUnix,
      selfCheck: state.selfCheck,
      audit: state.audit,
      plans: state.plans,
      queueJobs: state.queueJobs,
    });
    nodes.watermarkText.textContent = newest
      ? "Data current as of " +
        formatTimestamp(newest) +
        " (" +
        relativeAge(newest) +
        ")"
      : state.loadedOnce
        ? "No workspace data yet"
        : "Loading workspace data…";
  }

  // ── Shell DOM ───────────────────────────────────────────────────────

  function buildShell(): OverviewNodes {
    const watermarkDot = el("span", {
      class: "status-dot",
      dataset: { state: "busy" },
    });
    const watermarkText = el("span", { text: "Loading workspace data…" });
    const refreshButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Refresh",
      attrs: { type: "button" },
      on: { click: () => void refreshAll() },
    }) as HTMLButtonElement;

    const lockRow = el("div", { class: "dest-overview-lock-row" });
    const statsRow = el("div", { class: "stats dest-overview-stats" });

    const attentionList = el("ul", { class: "dest-overview-attention-list" });
    const attentionEmptyTitle = el("p", {
      class: "section-empty-title",
      text: "Nothing needs your attention",
    });
    const attentionEmptyBody = el("p", { class: "section-empty-body" });
    const selfCheckButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Run self-check",
      attrs: { type: "button" },
      on: { click: () => void runSelfCheckExplicit() },
    }) as HTMLButtonElement;
    const unlockLink = el("a", {
      class: "btn-primary btn-small hidden",
      text: "Open vault",
      attrs: { href: "#/vault" },
    }) as HTMLAnchorElement;
    const attentionEmpty = el(
      "div",
      { class: "section-empty", dataset: { tier: "quiet" } },
      attentionEmptyTitle,
      attentionEmptyBody,
      selfCheckButton,
      unlockLink,
    );
    const attentionSkeleton = skeletonStack(2);

    const auditList = el("ul", { class: "dest-overview-audit-list" });
    const auditEmpty = el(
      "div",
      { class: "section-empty", dataset: { tier: "quiet" } },
      el("p", { class: "section-empty-title", text: "No activity yet" }),
      el("p", {
        class: "section-empty-body",
        text: "Audit events appear here as you unlock the vault, configure wallets, and run operator actions.",
      }),
      el("a", {
        class: "btn-ghost btn-small",
        text: "Open audit viewer",
        attrs: { href: "#/vault" },
      }),
    );
    const auditSkeleton = skeletonStack(2);
    const moreButton = el("button", {
      class: "btn-ghost btn-small",
      text: "Show more",
      attrs: { type: "button" },
      on: { click: () => void loadMoreAudit() },
    }) as HTMLButtonElement;
    const moreRow = el(
      "div",
      { class: "dest-overview-more-row hidden" },
      moreButton,
    );

    const root = el(
      "div",
      { class: "dest-overview" },
      el(
        "div",
        { class: "page-header" },
        el(
          "div",
          null,
          el("h2", {
            class: "page-header-title",
            text: "What needs my attention?",
          }),
          el("p", {
            class: "page-header-summary",
            text: "Plans waiting for review, queue jobs that stopped, self-check failures, stale scans, and risk findings — ranked, each with one next step.",
          }),
        ),
        el(
          "div",
          { class: "page-header-actions" },
          el(
            "p",
            { class: "dest-overview-watermark", attrs: { role: "status" } },
            watermarkDot,
            watermarkText,
          ),
          refreshButton,
        ),
      ),
      el("div", { class: "dest-overview-banner-slot" }),
      el(
        "section",
        { class: "dest-overview-status", attrs: { "aria-label": "Workspace status" } },
        lockRow,
        statsRow,
      ),
      el(
        "section",
        { class: "dest-overview-section", attrs: { "aria-label": "Needs attention" } },
        el("h3", { class: "section-title", text: "Needs attention" }),
        attentionSkeleton,
        attentionList,
        attentionEmpty,
      ),
      el(
        "section",
        { class: "dest-overview-section", attrs: { "aria-label": "Recent activity" } },
        el("h3", { class: "section-title", text: "Recent activity" }),
        auditSkeleton,
        auditList,
        auditEmpty,
        moreRow,
      ),
    );

    return {
      root,
      bannerSlot: root.children[1] as HTMLElement,
      watermarkDot,
      watermarkText,
      refreshButton,
      lockRow,
      statsRow,
      attentionList,
      attentionEmpty,
      attentionEmptyTitle,
      attentionEmptyBody,
      attentionSkeleton,
      selfCheckButton,
      unlockLink,
      auditList,
      auditEmpty,
      auditSkeleton,
      moreRow,
      moreButton,
    };
  }

  // ── Store wiring ────────────────────────────────────────────────────

  // Status baseline: the store batches notifications on a microtask, so a
  // status set BEFORE mount re-fires this subscription after it. Treating
  // the reference mount already saw as the baseline keeps that pending
  // notification from double-triggering a full refresh.
  let lastStatus: StatusResponse | null = null;

  function subscribeStore(): void {
    unsubscribes.push(
      store.subscribe("status", (next) => {
        if (!mounted || next === lastStatus) return;
        const wasLocked = lastStatus?.locked ?? null;
        const wasCompartment =
          lastStatus?.active_compartment?.compartment_id ?? null;
        lastStatus = next;
        state.locked = next?.locked ?? false;
        renderStatusStrip();
        // Refetch only on genuine lock flips / compartment switches. The
        // null → known transition is covered by mount's own refresh (or by
        // the vault_locked failure path), so it must not refetch.
        if (
          next &&
          wasLocked !== null &&
          (next.locked !== wasLocked ||
            (next.active_compartment?.compartment_id ?? null) !==
              wasCompartment)
        ) {
          void refreshAll();
          return;
        }
        renderAttention();
      }),
      store.subscribe("operations", () => {
        if (!mounted) return;
        renderAttention();
      }),
      store.subscribe("queueEvents", () => {
        if (!mounted) return;
        void refreshQueueOnly();
      }),
      store.subscribe("resync", () => {
        if (!mounted) return;
        void refreshAll();
      }),
      store.subscribe("sync", () => {
        if (!mounted) return;
        renderWatermark();
      }),
    );
  }

  return {
    id: "overview",
    migrated: true,
    mount(_route: Route) {
      // Overview owns no sub-routes: every deep-linkable detail lives in the
      // destination that owns the resource (plan review in Move, diagnostics
      // in Vault, scans in Portfolio), so mount ignores the sub-path.
      const target = document.getElementById(OVERVIEW_HOST_ID);
      if (!target) return;
      host = target;
      mounted = true;
      generation += 1;

      // Preserve the legacy hero markup so unmount restores it byte-for-byte
      // and the legacy refresh loop picks it back up on its next pass.
      savedHostHtml = host.innerHTML;
      clearChildren(host);

      nodes = buildShell();
      host.appendChild(nodes.root);

      state.locked = store.get("status")?.locked ?? false;
      lastStatus = store.get("status");
      subscribeStore();
      renderDynamic();
      if (!state.locked) {
        void refreshAll();
      } else {
        state.loadedOnce = true;
        renderDynamic();
      }
    },
    unmount() {
      mounted = false;
      generation += 1;
      lastStatus = null;
      while (unsubscribes.length) unsubscribes.pop()?.();
      if (host) {
        clearChildren(host);
        host.innerHTML = savedHostHtml;
      }
      host = null;
      nodes = null;
    },
  };
}
