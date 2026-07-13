import type { ActiveCompartment } from "../contracts";
import type { SelfCheckSummary } from "./selfcheck";

function setText(id: string, value: string): void {
  const element = document.getElementById(id);
  if (element) element.textContent = value;
}

function setState(id: string, value: string): void {
  const element = document.getElementById(id);
  if (element) element.dataset.state = value;
}

function selfCheckLabel(summary: SelfCheckSummary | null): string {
  if (!summary) return "Self-check pending";
  if (summary.status === "pass") return "Self-check healthy";
  if (summary.status === "warn") {
    return "Self-check: " + String(summary.warnCount) + " warning(s)";
  }
  return "Self-check: " + String(summary.failCount) + " failure(s)";
}

export function createStatusStripRenderer() {
  function reset(): void {
    document.getElementById("statusStrip")?.classList.add("hidden");
    setText("stripLockState", "Locked");
    setState("stripLockState", "locked");
    setText("stripCompartment", "No active compartment");
    setText("stripSelfCheck", "Self-check unavailable");
    setState("stripSelfCheck", "idle");
  }

  function renderUnlocked(
    active: ActiveCompartment | null | undefined,
    summary: SelfCheckSummary | null,
  ): void {
    document.getElementById("statusStrip")?.classList.remove("hidden");
    setText("stripLockState", "Unlocked");
    setState("stripLockState", "unlocked");
    setText(
      "stripCompartment",
      active?.compartment_label || "No active compartment",
    );
    setText("stripSelfCheck", selfCheckLabel(summary));
    setState("stripSelfCheck", summary?.status || "pending");
  }

  return { renderUnlocked, reset };
}
