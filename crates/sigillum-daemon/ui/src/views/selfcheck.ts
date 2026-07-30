import { ROUTE_PATHS } from "../routePaths";
import type { SelfCheckResult, SelfCheckRunResponse } from "../contracts";
import { renderEmptyState } from "../render/forms";
import { esc, pillClass, statusPill } from "../render/html";

/**
 * Live configuration self-check: one button proves every configured input
 * still works — providers answer RPC on the right chain, wallets re-derive,
 * policy and allocations stay internally consistent.
 *
 * Results are deliberately session-local: yesterday's green proves nothing,
 * so there is no persisted "last run" loader — only "run it now".
 */

/** Stable rendering order for check domains, mirroring the daemon contract. */
export const SELF_CHECK_DOMAIN_ORDER = [
  "provider",
  "seed-wallet",
  "xpub-wallet",
  "stealth-wallet",
  "watch-book",
  "policy",
  "receive-allocation",
  "fido2",
];

/** Both run buttons get the busy affordance no matter which one fired. */
const RUN_BUTTON_IDS = ["selfCheckRunDiag", "selfCheckRunTreasury"];

/** Local wall-clock HH:MM:SS for the "ran at" suffix, locale-independent. */
export function formatClockTime(unix: number): string {
  const date = new Date(unix * 1000);
  const pad = (value: number) => String(value).padStart(2, "0");
  return (
    pad(date.getHours()) + ":" + pad(date.getMinutes()) + ":" + pad(date.getSeconds())
  );
}

export interface SelfCheckDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
}

interface StatusCounts {
  pass: number;
  warn: number;
  fail: number;
}

function countByStatus(checks: SelfCheckResult[]): StatusCounts {
  const counts: StatusCounts = { pass: 0, warn: 0, fail: 0 };
  checks.forEach((check) => {
    if (check.status === "pass") counts.pass += 1;
    else if (check.status === "warn") counts.warn += 1;
    else counts.fail += 1;
  });
  return counts;
}

interface DomainGroup {
  domain: string;
  checks: SelfCheckResult[];
}

function groupByDomain(checks: SelfCheckResult[]): DomainGroup[] {
  const byDomain = new Map<string, SelfCheckResult[]>();
  checks.forEach((check) => {
    const bucket = byDomain.get(check.domain);
    if (bucket) bucket.push(check);
    else byDomain.set(check.domain, [check]);
  });
  const groups: DomainGroup[] = [];
  SELF_CHECK_DOMAIN_ORDER.forEach((domain) => {
    const bucket = byDomain.get(domain);
    if (bucket) {
      groups.push({ domain, checks: bucket });
      byDomain.delete(domain);
    }
  });
  // Unknown domains (a newer daemon than this UI) still render, after the
  // known set, instead of disappearing silently.
  byDomain.forEach((bucket, domain) => groups.push({ domain, checks: bucket }));
  return groups;
}

export interface SelfCheckSummary {
  status: string;
  failCount: number;
  warnCount: number;
  atUnix: number;
}

/** Ambient surfaces re-check at most this often; explicit runs always probe. */
export const SELF_CHECK_TTL_MS = 5 * 60_000;

export function createSelfCheckActions(deps: SelfCheckDeps) {
  // Session-local only: results live here and in the DOM, never in storage.
  let lastResponse: SelfCheckRunResponse | null = null;
  let lastRunAtMs = 0;
  let inFlight: Promise<SelfCheckSummary | null> | null = null;

  function runButtons(): Element[] {
    const buttons: Element[] = [];
    RUN_BUTTON_IDS.forEach((id) => {
      const el = document.getElementById(id);
      if (el) buttons.push(el);
    });
    return buttons;
  }

  function renderCheckRow(check: SelfCheckResult): string {
    return (
      '<li><div class="entity-main">' +
      '<div class="entity-title">' +
      esc(check.subject) +
      " " +
      statusPill(check.status) +
      "</div>" +
      '<div class="entity-meta">' +
      esc(check.detail) +
      (check.latency_ms != null ? " · " + esc(String(check.latency_ms)) + "ms" : "") +
      "</div></div></li>"
    );
  }

  function renderSummary(response: SelfCheckRunResponse): void {
    const summary = document.getElementById("selfCheckSummary");
    if (!summary) return;
    const counts = countByStatus(response.checks || []);
    const ranAt = "ran " + formatClockTime(response.generated_at_unix);
    if (response.status === "pass") {
      summary.textContent =
        counts.pass +
        " pass · " +
        counts.warn +
        " warn · " +
        counts.fail +
        " fail · " +
        ranAt;
      return;
    }
    // Trouble: counts become pills so warn/fail totals read at a glance.
    summary.innerHTML =
      '<span class="pill ' +
      pillClass("pass") +
      '">' +
      counts.pass +
      " pass</span> · " +
      '<span class="pill ' +
      pillClass("warn") +
      '">' +
      counts.warn +
      " warn</span> · " +
      '<span class="pill ' +
      pillClass("fail") +
      '">' +
      counts.fail +
      " fail</span> · " +
      esc(ranAt);
  }

  function renderSelfCheckPanel(): void {
    const list = document.getElementById("selfCheckList");
    const summary = document.getElementById("selfCheckSummary");
    if (!lastResponse) {
      if (summary) summary.textContent = "";
      if (list) {
        list.innerHTML = renderEmptyState({
          message: "Not run yet in this session.",
          actionLabel: "Run Self-Check",
          action: "runSelfCheck",
        });
      }
      return;
    }
    renderSummary(lastResponse);
    if (!list) return;
    const groups = groupByDomain(lastResponse.checks || []);
    if (!groups.length) {
      list.innerHTML = renderEmptyState(
        "The daemon returned no checks — nothing is configured to verify yet.",
      );
      return;
    }
    list.innerHTML = groups
      .map(
        (group) =>
          '<div class="section-title">' +
          esc(group.domain) +
          ' <span class="text-meta">· ' +
          group.checks.length +
          "</span></div>" +
          '<ul class="entity-list">' +
          group.checks.map(renderCheckRow).join("") +
          "</ul>",
      )
      .join("");
  }

  async function runSelfCheck(): Promise<void> {
    const buttons = runButtons();
    buttons.forEach((button) => button.classList.add("btn-busy"));
    try {
      // Empty body = verify every configured domain.
      const r = await deps.api("POST", ROUTE_PATHS.API_SELFCHECK_RUN, {});
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      lastResponse = r as SelfCheckRunResponse;
      lastRunAtMs = Date.now();
      renderSelfCheckPanel();
      const counts = countByStatus(lastResponse.checks || []);
      if (lastResponse.status === "pass") {
        deps.toast("Self-check passed: " + counts.pass + " checks green.");
      } else {
        const issueCount = counts.warn + counts.fail;
        const message =
          "Self-check: " + issueCount + " issue(s) found — see System section";
        if (lastResponse.status === "fail") deps.toast(message, "error");
        else deps.toast(message);
      }
    } catch (e: any) {
      deps.toast(String(e?.message ?? e), "error");
    } finally {
      buttons.forEach((button) => button.classList.remove("btn-busy"));
    }
  }

  function lastSelfCheckSummary(): SelfCheckSummary | null {
    if (!lastResponse) return null;
    const counts = countByStatus(lastResponse.checks || []);
    return {
      status: lastResponse.status,
      failCount: counts.fail,
      warnCount: counts.warn,
      atUnix: lastResponse.generated_at_unix,
    };
  }

  /**
   * Silent, throttled run for ambient surfaces (the topbar status strip).
   * Self-check probes live RPC endpoints, so the periodic refresh cycle must
   * never turn into provider probe traffic: results are cached for
   * SELF_CHECK_TTL_MS and concurrent callers share one in-flight run.
   * Failures of the call itself degrade to the previous summary (or null)
   * rather than surfacing errors — the explicit Run buttons own loud paths.
   */
  async function ensureFreshSelfCheck(): Promise<SelfCheckSummary | null> {
    if (lastResponse && Date.now() - lastRunAtMs < SELF_CHECK_TTL_MS) {
      return lastSelfCheckSummary();
    }
    if (inFlight) return inFlight;
    inFlight = (async () => {
      try {
        const r = await deps.api("POST", ROUTE_PATHS.API_SELFCHECK_RUN, {});
        if (r.error) return lastSelfCheckSummary();
        lastResponse = r as SelfCheckRunResponse;
        lastRunAtMs = Date.now();
        renderSelfCheckPanel();
        return lastSelfCheckSummary();
      } catch (_) {
        return lastSelfCheckSummary();
      } finally {
        inFlight = null;
      }
    })();
    return inFlight;
  }

  return {
    runSelfCheck,
    renderSelfCheckPanel,
    ensureFreshSelfCheck,
    lastSelfCheckSummary,
  };
}
