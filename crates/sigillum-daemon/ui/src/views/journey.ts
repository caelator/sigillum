import type { TreasuryOverviewResponse, TreasuryPolicy } from "../contracts";
import { esc, escAttr } from "../render/html";

/**
 * Guided treasury journey: four operator goals, computed from live daemon
 * state on every refresh, plus compact metrics on the Overview destination.
 *
 * The journey card answers "what do I do next?"; the status strip answers
 * "what is configured right now?" — both from the same five GETs.
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
      actionArg: "treasuryCard",
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

function baseStatusChips(state: JourneyState): StatusChipSpec[] {
  return [
    {
      value: state.providerCount,
      label: "Providers",
      targetCard: state.providerCount === 0 ? "walletManagerCard" : "profilesCard",
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
      targetCard: "treasuryOverviewCard",
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
    const steps = computeJourneySteps(state);
    const doneCount = steps.filter((step) => step.done).length;
    const progress = document.getElementById("journeyProgress");
    if (progress) progress.textContent = doneCount + " of " + steps.length;
    const completeLine = document.getElementById("journeyComplete");

    if (doneCount === steps.length) {
      // Every goal met: collapse to one quiet line instead of a checklist
      // the operator no longer needs. The card stays so the strip and the
      // progress count keep a stable home.
      list.innerHTML = "";
      list.classList.add("hidden");
      if (completeLine) {
        completeLine.textContent = "Treasury ready — all setup steps complete";
        completeLine.classList.remove("hidden");
      }
      return;
    }

    if (completeLine) {
      completeLine.textContent = "";
      completeLine.classList.add("hidden");
    }
    list.classList.remove("hidden");
    list.innerHTML = steps.map(renderJourneyStepRow).join("");
  }

  function renderJourneyMetrics(state: JourneyState): void {
    const metrics = document.getElementById("journeyMetrics");
    if (!metrics) return;
    metrics.innerHTML = baseStatusChips(state)
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
          deps.api("GET", "/api/profiles/evm"),
          deps.api("GET", "/api/profiles/eth-seed"),
          deps.api("GET", "/api/profiles/eth-xpub"),
          deps.api("GET", "/api/treasury/overview"),
          deps.api("GET", "/api/treasury/policy"),
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
      renderJourneyCard(state);
      renderJourneyMetrics(state);
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
      const r = await deps.api("POST", "/api/inventory/scan/evm", {});
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
