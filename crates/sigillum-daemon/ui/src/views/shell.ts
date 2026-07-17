import type { ActiveCompartment, UnlockedCompartment } from "../contracts";
import { clearSessionToken } from "../api/session";
import { setHiddenById as setHidden, setTextById as setText } from "../render/dom";
import { esc, escAttr } from "../render/html";

/// Shell chrome for the compartment switcher and the topbar badge. These
/// read the daemon's wire shape directly: `active_compartment` carries
/// `compartment_id`/`compartment_label` while `unlocked_compartments`
/// entries carry `id`/`label` (see contracts.ts).
export function renderCompartmentSwitcher(
  unlocked: UnlockedCompartment[],
  active: ActiveCompartment | null | undefined,
): void {
  const switcher = document.getElementById("compSwitcher");
  if (!switcher) return;
  if (unlocked.length <= 1) {
    switcher.innerHTML = "";
    setHidden("compSwitcher", true);
    return;
  }

  let html = "";
  unlocked.forEach((compartment) => {
    const isActive = active && active.compartment_id === compartment.id;
    html +=
      '<button class="' +
      (isActive ? "active" : "") +
      '" data-action="switchCompartment" data-arg0="' +
      escAttr(String(compartment.id)) +
      '" data-arg0-type="number">' +
      esc(compartment.label) +
      "</button>";
  });
  switcher.innerHTML = html;
  setHidden("compSwitcher", false);
}

export function renderActiveCompartment(
  active: ActiveCompartment | null | undefined,
  unlocked: UnlockedCompartment[],
): void {
  const compBadge = document.getElementById("compartmentBadge");
  if (active) {
    if (compBadge) {
      compBadge.textContent =
        active.compartment_label || "Compartment " + active.compartment_id;
    }
    setHidden("compartmentBadge", false);
    setText("apiKeyCount", active.api_key_count || 0);
    setText(
      "secretCount",
      active.secret_count != null ? active.secret_count : "(locked)",
    );
  } else {
    setHidden("compartmentBadge", true);
    setText("apiKeyCount", "-");
    setText("secretCount", "-");
  }

  setText("compartmentCount", unlocked.length);
}

export interface ShellRendererDeps {
  operatorCardIds: string[];
  setUiMode: (mode: "setup" | "locked" | "unlocked") => void;
  setCardsHidden: (ids: string[], hidden: boolean) => void;
  setStatusBadge: (className: string, label: string) => void;
  setSecretsAccess: (unlocked: boolean) => void;
  resetVaultCounts: () => void;
  setUnlockGuidance: (mode: string) => void;
  updateHeroState: (active?: any, unlocked?: any[]) => void;
  updateWizardChrome: (id: string) => void;
  resetSetupWizard: () => void;
  renderCompartmentSwitcher: (
    unlocked: UnlockedCompartment[],
    active: ActiveCompartment | null | undefined,
  ) => void;
  renderActiveCompartment: (
    active: ActiveCompartment | null | undefined,
    unlocked: UnlockedCompartment[],
  ) => void;
  buildPushSelectors: (unlocked: UnlockedCompartment[]) => void;
}

export function createShellRenderer(deps: ShellRendererDeps) {
  function clearStatusStrip(): void {
    // The topbar status strip only describes an unlocked workspace; leaving
    // counts visible on the locked/setup screens would leak state and
    // mislead. journey.ts also guards on body[data-mode] before rendering.
    const strip = document.getElementById("statusStrip");
    if (strip) strip.innerHTML = "";
  }

  function applySetupUi(): void {
    // The refresh cycle re-applies setup mode every few seconds. Resetting
    // the wizard on every pass silently wipes in-progress choices (preset,
    // compartments) while the rendered step still shows them — so only reset
    // when actually ENTERING setup mode.
    const alreadyInSetup = document.body.dataset.mode === "setup";
    deps.setUiMode("setup");
    document.body.dataset.mode = "setup";
    clearStatusStrip();
    if (!alreadyInSetup) deps.resetSetupWizard();
    clearSessionToken();
    deps.setStatusBadge("status-no-vault", "NO VAULT");
    setHidden("compartmentBadge", true);
    setHidden("setupCard", false);
    setHidden("authCard", true);
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.setSecretsAccess(false);
    deps.resetVaultCounts();
    deps.setUnlockGuidance("passphrase");
    deps.updateWizardChrome(
      document.querySelector(".wizard-step.active")?.id || "wizStep0",
    );
  }

  function applyLockedUi(): void {
    deps.setUiMode("locked");
    document.body.dataset.mode = "locked";
    clearStatusStrip();
    clearSessionToken();
    deps.setStatusBadge("status-locked", "LOCKED");
    setHidden("compartmentBadge", true);
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.resetVaultCounts();
    setHidden("lockForm", true);
    setHidden("authRecovery", false);
    setHidden("compSwitcher", true);
    setText("authTitle", "Unlock Sigillum");
    setText(
      "authLead",
      "Enter the vault passphrase you chose during setup, or switch to the hardware-key tab. The session token stays only in this browser tab.",
    );
    deps.setSecretsAccess(false);
    deps.setUnlockGuidance("passphrase");
    // The passphrase field is the only actionable control on this screen;
    // hand it focus once the locked layout has settled.
    setTimeout(() => {
      const passphrase = document.getElementById(
        "passphrase",
      ) as HTMLInputElement | null;
      passphrase?.focus?.();
    }, 0);
  }

  function applyUnlockedUi(
    active: ActiveCompartment | null | undefined,
    unlocked: UnlockedCompartment[],
  ): void {
    deps.setUiMode("unlocked");
    document.body.dataset.mode = "unlocked";
    deps.setStatusBadge("status-unlocked", "UNLOCKED");
    setHidden("unlockPassphrase", true);
    setHidden("unlockFido2", true);
    setHidden("unlockTabs", true);
    setHidden("lockForm", false);
    setHidden("authRecovery", true);
    setText("authTitle", "Session controls");
    setText(
      "authLead",
      "This browser currently holds a valid local session token. Locking clears unlocked keys from daemon memory; logging out only clears this browser session.",
    );
    deps.setUnlockGuidance("session");

    deps.renderCompartmentSwitcher(unlocked, active);
    deps.renderActiveCompartment(active, unlocked);
    deps.setSecretsAccess(true);

    setHidden("compartmentCard", false);
    setHidden("pushCard", unlocked.length < 2);
    if (unlocked.length >= 2) deps.buildPushSelectors(unlocked);

    setHidden("guideCard", false);
    setHidden("journeyCard", false);
    setHidden("walletManagerCard", false);
    setHidden("profilesCard", false);
    setHidden("xpubCard", false);
    setHidden("receivingCard", false);
    setHidden("receiveBookCard", false);
    setHidden("treasuryCard", false);
    setHidden("inventoryCard", false);
    setHidden("depositsCard", false);
    setHidden("plansCard", false);
    setHidden("policyCard", false);
    setHidden("queueCard", false);
    setHidden("maintenanceCard", false);
    setHidden("backupCard", false);
    setHidden("auditCard", false);
    setHidden("diagCard", false);
    deps.updateHeroState(active, unlocked);
  }

  return {
    applySetupUi,
    applyLockedUi,
    applyUnlockedUi,
  };
}
