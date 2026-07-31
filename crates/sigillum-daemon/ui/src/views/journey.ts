import { ROUTE_PATHS } from "../routePaths";
import type { TreasuryOverviewResponse, TreasuryPolicy } from "../contracts";
import { esc, escAttr } from "../render/html";

/**
 * Guided treasury journey: four operator goals, computed from live daemon
 * state on every refresh, plus the always-visible topbar status strip.
 *
 * The journey card answers "what do I do next?"; the status strip answers
 * "is anything wrong right now?" — both from the same five GETs.
 */

export interface JourneyState {
  providerCount: number;
  walletCount: number;
  trackedAddressCount: number;
  policyConfigured: boolean;
  reviewNeededCount: number;
}

export interface JourneyStep {
  title: string;
  hint: string;
  done: boolean;
  /** data-action dispatched by the step button (hidden once done). */
  action: string;
  actionArg?: string;
  actionLabel: string;
}

export function computeJourneySteps(state: JourneyState): JourneyStep[] {
  return [
    {
      title: "Add an RPC provider",
      hint: "The endpoint Sigillum uses to read balances — your node or any endpoint you trust.",
      done: state.providerCount > 0,
      action: "journeyJump",
      actionArg: "walletManagerCard",
      actionLabel: "Add provider",
    },
    {
      title: "Create or import a wallet",
      hint: "Generate a fresh seed wallet, or import a seed phrase or watch-only xpub you already use.",
      done: state.walletCount > 0,
      action: "journeyJump",
      actionArg: "walletManagerCard",
      actionLabel: "Open wallets",
    },
    {
      title: "Run a balance scan",
      hint: "Reads every wallet across all providers to discover funded addresses — nothing moves.",
      done: state.trackedAddressCount > 0,
      action: "journeyRunScan",
      actionLabel: "Run scan",
    },
    {
      title: "Set treasury guardrails",
      hint: "Allowed destinations, value caps, and required simulation before any plan can move funds.",
      done: state.policyConfigured,
      action: "journeyJump",
      actionArg: "policyCard",
      actionLabel: "Set guardrails",
    },
  ];
}

interface StatusChipSpec {
  value: number;
  label: string;
  targetCard: string;
  tone: "neutral" | "warn" | "danger";
}

export interface StripSelfCheckSummary {
  status: string;
  failCount: number;
  warnCount: number;
}

function statusChips(
  state: JourneyState,
  selfCheck?: StripSelfCheckSummary | null,
): StatusChipSpec[] {
  const chips = baseStatusChips(state);
  if (selfCheck) {
    chips.push({
      value: selfCheck.failCount + selfCheck.warnCount,
      label: "Self-check issues",
      targetCard: "diagCard",
      tone:
        selfCheck.status === "fail"
          ? "danger"
          : selfCheck.status === "warn"
            ? "warn"
            : "neutral",
    });
  }
  return chips;
}

function baseStatusChips(state: JourneyState): StatusChipSpec[] {
  return [
    {
      value: state.providerCount,
      label: "Providers",
      targetCard: "walletManagerCard",
      tone: state.providerCount === 0 ? "warn" : "neutral",
    },
    {
      value: state.walletCount,
      label: "Wallets",
      targetCard: "walletManagerCard",
      tone: state.walletCount === 0 ? "warn" : "neutral",
    },
    {
      value: state.trackedAddressCount,
      label: "Tracked addresses",
      targetCard: "inventoryCard",
      tone: "neutral",
    },
    {
      value: state.reviewNeededCount,
      label: "Review needed",
      targetCard: "treasuryCard",
      tone: state.reviewNeededCount > 0 ? "danger" : "neutral",
    },
  ];
}

export interface JourneyDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  /** app.ts owns section switching + scroll; the journey only requests it. */
  jumpToCard: (cardId: string) => void;
  /** Re-render the treasury card after a journey-triggered scan. */
  refreshTreasury: () => Promise<unknown> | unknown;
  /**
   * Throttled silent self-check (selfcheck.ts owns the TTL cache). The strip
   * renders immediately without it, then re-renders when the summary lands —
   * provider probes must never block the refresh cycle.
   */
  ensureSelfCheck?: () => Promise<StripSelfCheckSummary | null>;
}

export function createJourneyActions(deps: JourneyDeps) {
  function renderJourneyStepRow(step: JourneyStep, index: number): string {
    const marker = step.done
      ? '<span class="journey-step-check">✓</span>'
      : '<span class="journey-step-num">' + (index + 1) + "</span>";
    const action = step.done
      ? ""
      : '<button type="button" class="btn-ghost journey-step-action" data-action="' +
        escAttr(step.action) +
        '"' +
        (step.actionArg ? ' data-arg0="' + escAttr(step.actionArg) + '"' : "") +
        ">" +
        esc(step.actionLabel) +
        "</button>";
    return (
      '<div class="journey-step' +
      (step.done ? " journey-step-done" : "") +
      '">' +
      marker +
      '<div class="journey-step-title">' +
      esc(step.title) +
      "</div>" +
      '<div class="journey-step-hint">' +
      esc(step.hint) +
      "</div>" +
      action +
      "</div>"
    );
  }

  function renderJourneyCard(state: JourneyState): void {
    const list = document.getElementById("journeyList");
    if (!list) return;
    const card = document.getElementById("journeyCard");
    const steps = computeJourneySteps(state);
    const doneCount = steps.filter((step) => step.done).length;
    const progress = document.getElementById("journeyProgress");
    if (progress) progress.textContent = doneCount + " of " + steps.length;
    const completeLine = document.getElementById("journeyComplete");

    if (doneCount === steps.length) {
      // Every goal met: the whole card collapses into one compact ready line
      // (check icon + text) — header, blurb, and checklist are hidden by the
      // journey-card-complete styles. The card itself stays so the overview
      // keeps a stable anchor.
      list.innerHTML = "";
      list.classList.add("hidden");
      if (card) card.classList.add("journey-card-complete");
      if (completeLine) {
        completeLine.textContent = "Treasury ready — all setup steps complete";
        completeLine.classList.add("journey-complete");
        completeLine.classList.remove("hidden");
      }
      return;
    }

    if (card) card.classList.remove("journey-card-complete");
    if (completeLine) {
      completeLine.textContent = "";
      completeLine.classList.remove("journey-complete");
      completeLine.classList.add("hidden");
    }
    list.classList.remove("hidden");
    list.innerHTML = steps.map(renderJourneyStepRow).join("");
  }

  let lastState: JourneyState | null = null;
  let lastSelfCheck: StripSelfCheckSummary | null = null;

  function renderStatusStrip(state: JourneyState): void {
    const strip = document.getElementById("statusStrip");
    if (!strip) return;
    // The strip is workspace chrome: never show stale counts on the locked
    // or setup screens (shell also clears it when leaving unlocked mode).
    if (document.body.dataset.mode !== "unlocked") {
      strip.innerHTML = "";
      return;
    }
    strip.innerHTML = statusChips(state, lastSelfCheck)
      .map((chip) => {
        const toneClass =
          chip.tone === "warn"
            ? " status-chip-warn"
            : chip.tone === "danger"
              ? " status-chip-danger"
              : "";
        return (
          '<button type="button" class="status-chip' +
          toneClass +
          '" data-action="journeyJump" data-arg0="' +
          escAttr(chip.targetCard) +
          '" title="' +
          escAttr("Open " + chip.label.toLowerCase()) +
          '">' +
          '<span class="status-chip-value">' +
          esc(String(chip.value)) +
          "</span>" +
          '<span class="status-chip-label">' +
          esc(chip.label) +
          "</span></button>"
        );
      })
      .join("");
  }

  async function loadJourney(): Promise<void> {
    try {
      const [evmResp, seedResp, xpubResp, overviewResp, policyResp] =
        await Promise.all([
          deps.api("GET", ROUTE_PATHS.API_PROFILES_EVM),
          deps.api("GET", ROUTE_PATHS.API_PROFILES_ETH_SEED),
          deps.api("GET", ROUTE_PATHS.API_PROFILES_ETH_XPUB),
          deps.api("GET", ROUTE_PATHS.API_TREASURY_OVERVIEW),
          deps.api("GET", ROUTE_PATHS.API_TREASURY_POLICY),
        ]);
      const providerCount = evmResp.error ? 0 : (evmResp.profiles || []).length;
      const seedCount = seedResp.error ? 0 : (seedResp.profiles || []).length;
      const xpubCount = xpubResp.error ? 0 : (xpubResp.profiles || []).length;
      const overview = overviewResp.error
        ? null
        : (overviewResp as TreasuryOverviewResponse);
      const policy = policyResp.error
        ? null
        : ((policyResp.policy || null) as TreasuryPolicy | null);
      const state: JourneyState = {
        providerCount,
        walletCount: seedCount + xpubCount,
        trackedAddressCount: overview ? overview.tracked_address_count || 0 : 0,
        policyConfigured: policy != null,
        reviewNeededCount: overview
          ? (overview.plans?.latest_review_required_steps || 0) +
            (overview.risk?.high_findings || 0) +
            (overview.risk?.critical_findings || 0)
          : 0,
      };
      lastState = state;
      renderJourneyCard(state);
      renderStatusStrip(state);
      // Ambient self-check: render the strip now, upgrade it when the
      // (cached or fresh) summary resolves. Never blocks the refresh cycle.
      if (deps.ensureSelfCheck) {
        void deps.ensureSelfCheck().then((summary) => {
          lastSelfCheck = summary;
          if (lastState) renderStatusStrip(lastState);
        });
      }
    } catch (_) {}
  }

  function journeyJump(cardId: string): void {
    if (cardId) deps.jumpToCard(cardId);
  }

  function scanButtons(): Element[] {
    try {
      return Array.from(
        document.querySelectorAll('[data-action="journeyRunScan"]'),
      );
    } catch (_) {
      return [];
    }
  }

  async function journeyRunScan(): Promise<void> {
    const buttons = scanButtons();
    buttons.forEach((button) => button.classList.add("btn-busy"));
    deps.toast("Balance scan started — reading every wallet across all providers…");
    try {
      // Empty body = the guided default: scan all profiles on all providers.
      const r = await deps.api("POST", ROUTE_PATHS.API_INVENTORY_SCAN_EVM, {});
      if (r.error) {
        deps.toast(r.error, "error");
        return;
      }
      deps.toast("Balance scan complete");
      await Promise.all([loadJourney(), deps.refreshTreasury()]);
    } catch (e: any) {
      deps.toast(String(e?.message ?? e), "error");
    } finally {
      buttons.forEach((button) => button.classList.remove("btn-busy"));
    }
  }

  return {
    loadJourney,
    journeyJump,
    journeyRunScan,
  };
}
