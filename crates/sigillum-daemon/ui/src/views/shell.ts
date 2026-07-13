import { clearSessionToken } from "../api/session";
import { setHiddenById as setHidden, setTextById as setText } from "../render/dom";

export interface ShellRendererDeps {
  operatorCardIds: string[];
  setUiMode: (mode: "setup" | "locked" | "unlocked") => void;
  setCardsHidden: (ids: string[], hidden: boolean) => void;
  setStatusBadge: (className: string, label: string) => void;
  setSecretsAccess: (unlocked: boolean) => void;
  resetVaultCounts: () => void;
  setUnlockGuidance: (mode: string) => void;
  updateHeroState: (mode: string, active?: any, unlocked?: any[]) => void;
  updateWizardChrome: (id: string) => void;
  resetSetupWizard: () => void;
  renderCompartmentSwitcher: (unlocked: any[], active: any) => void;
  renderActiveCompartment: (active: any, unlocked: any[]) => void;
  buildPushSelectors: (unlocked: any[]) => void;
  resetStatusStrip: () => void;
  resetSelfCheck: () => void;
  scrubPrivateWorkspace: () => void;
}

export function createShellRenderer(deps: ShellRendererDeps) {
  function applySetupUi(forcePrivateReset = false): void {
    // The refresh cycle re-applies setup mode every few seconds. Resetting
    // the wizard on every pass silently wipes in-progress choices (preset,
    // compartments) while the rendered step still shows them — so only reset
    // when actually ENTERING setup mode.
    const alreadyInSetup = document.body.dataset.mode === "setup";
    deps.setUiMode("setup");
    document.body.dataset.mode = "setup";
    clearSessionToken();
    deps.resetStatusStrip();
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.setSecretsAccess(false);
    if (!alreadyInSetup || forcePrivateReset) deps.scrubPrivateWorkspace();
    deps.resetSelfCheck();
    if (!alreadyInSetup || forcePrivateReset) deps.resetSetupWizard();
    deps.setStatusBadge("status-no-vault", "NO VAULT");
    setHidden("lockForm", true);
    setHidden("compSwitcher", true);
    const switcher = document.getElementById("compSwitcher");
    if (switcher) switcher.innerHTML = "";
    setText("compartmentBadge", "");
    setHidden("compartmentBadge", true);
    setHidden("setupCard", false);
    setHidden("authCard", true);
    deps.resetVaultCounts();
    deps.setUnlockGuidance("passphrase");
    deps.updateHeroState("setup");
    deps.updateWizardChrome(
      document.querySelector(".wizard-step.active")?.id || "wizStep0",
    );
  }

  function applyLockedUi(forcePrivateReset = false): void {
    const alreadyLocked = document.body.dataset.mode === "locked";
    deps.setUiMode("locked");
    document.body.dataset.mode = "locked";
    clearSessionToken();
    deps.resetStatusStrip();
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.setSecretsAccess(false);
    if (!alreadyLocked || forcePrivateReset) deps.scrubPrivateWorkspace();
    deps.resetSelfCheck();
    deps.setStatusBadge("status-locked", "LOCKED");
    setHidden("compartmentBadge", true);
    deps.resetVaultCounts();
    setHidden("unlockPassphrase", false);
    setHidden("unlockFido2", true);
    setHidden("unlockTabs", true);
    setHidden("lockForm", true);
    setHidden("authRecovery", false);
    setHidden("compSwitcher", true);
    setText("authTitle", "Unlock Sigillum");
    setText(
      "authLead",
      "Enter the vault passphrase you chose during setup, or switch to the hardware-key tab. The session token stays only in this browser tab.",
    );
    deps.setUnlockGuidance("passphrase");
    deps.updateHeroState("locked");
    // The passphrase field is the only actionable control on this screen;
    // hand it focus once the locked layout has settled.
    if (!alreadyLocked) {
      setTimeout(() => {
        const passphrase = document.getElementById(
          "passphrase",
        ) as HTMLInputElement | null;
        passphrase?.focus?.();
      }, 0);
    }
  }

  function applyUnlockedUi(active: any, unlocked: any[]): void {
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

    // Setup and locked modes hide the complete operator surface. Restore it
    // from the same authoritative list so newly added destination wrappers
    // cannot remain silently hidden after unlock.
    deps.setCardsHidden(deps.operatorCardIds, false);
    setHidden("pushCard", unlocked.length < 2);
    if (unlocked.length >= 2) deps.buildPushSelectors(unlocked);
    deps.updateHeroState("unlocked", active, unlocked);
  }

  return {
    applySetupUi,
    applyLockedUi,
    applyUnlockedUi,
  };
}
