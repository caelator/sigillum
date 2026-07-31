import type { ActiveCompartment, UnlockedCompartment } from "../contracts";
import { clearSessionToken } from "../api/session";
import { clearList, el, renderList } from "../core/dom";
import { setHiddenById as setHidden, setTextById as setText } from "../render/dom";
import { focusableElements, hasActiveModal } from "../render/modal";

export interface WorkspaceSectionNavItem {
  id: string;
  label: string;
  summary: string;
}

/**
 * Patch the workspace navigation by stable section id. Periodic status
 * refreshes therefore update state without replacing the focused button.
 */
export function renderWorkspaceSectionNav(
  nav: HTMLElement,
  sections: readonly WorkspaceSectionNavItem[],
  activeSection: string,
): void {
  if (sections.length <= 1) {
    clearList(nav);
    return;
  }

  renderList(
    nav,
    sections,
    (section) => section.id,
    (section, existing) => {
      const button = (existing ?? el("button")) as HTMLButtonElement;
      const isActive = section.id === activeSection;
      button.type = "button";
      button.className = "nav-item" + (isActive ? " active" : "");
      if (isActive) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
      button.setAttribute("title", section.summary);
      button.dataset.action = "selectWorkspaceSection";
      button.dataset.arg0 = section.id;
      button.textContent = section.label;
      return button;
    },
  );
}

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
    clearList(switcher);
    setHidden("compSwitcher", true);
    return;
  }

  renderList(
    switcher,
    unlocked,
    (compartment) => String(compartment.id),
    (compartment, existing) => {
      const button = (existing ?? el("button")) as HTMLButtonElement;
      const isActive = active?.compartment_id === compartment.id;
      button.type = "button";
      button.className = isActive ? "active" : "";
      button.dataset.action = "switchCompartment";
      button.dataset.arg0 = String(compartment.id);
      button.dataset.arg0Type = "number";
      button.textContent = compartment.label;
      return button;
    },
  );
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
    const enteringLocked = document.body.dataset.mode !== "locked";
    const focusAtTransition = document.activeElement as HTMLElement | null;
    deps.setUiMode("locked");
    document.body.dataset.mode = "locked";
    clearStatusStrip();
    clearSessionToken();
    deps.setStatusBadge("status-locked", "LOCKED");
    setHidden("compartmentBadge", true);
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.resetVaultCounts();
    // An unlocked render hides both unlock panels. Reveal the passphrase
    // baseline synchronously so transition focus is valid while the async
    // hardware-key detection decides whether tabs should also be offered.
    setHidden("unlockPassphrase", false);
    setHidden("unlockFido2", true);
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
    // Focus once on the transition, after the locked layout settles. Refresh
    // re-renders must not keep pulling focus back from the operator, and the
    // deferred callback must yield if a modal opened or focus moved meanwhile.
    if (enteringLocked) {
      setTimeout(() => {
        if (document.body.dataset.mode !== "locked" || hasActiveModal()) return;
        const passphrase = document.getElementById(
          "passphrase",
        ) as HTMLInputElement | null;
        if (!passphrase?.isConnected || passphrase.disabled) return;

        const active = document.activeElement as HTMLElement | null;
        if (active === passphrase) return;
        const focusMoved = active !== focusAtTransition;
        if (
          focusMoved &&
          active &&
          focusableElements(document.body).includes(active)
        ) {
          return;
        }
        passphrase.focus();
      }, 0);
    }
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
