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
}

export function createShellRenderer(deps: ShellRendererDeps) {
  function applySetupUi(): void {
    deps.setUiMode("setup");
    document.body.dataset.mode = "setup";
    deps.resetSetupWizard();
    clearSessionToken();
    deps.setStatusBadge("status-no-vault", "NO VAULT");
    setHidden("compartmentBadge", true);
    setHidden("setupCard", false);
    setHidden("authCard", true);
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.setSecretsAccess(false);
    deps.resetVaultCounts();
    deps.setUnlockGuidance("passphrase");
    deps.updateHeroState("setup");
    deps.updateWizardChrome(
      document.querySelector(".wizard-step.active")?.id || "wizStep0",
    );
  }

  function applyLockedUi(): void {
    deps.setUiMode("locked");
    document.body.dataset.mode = "locked";
    clearSessionToken();
    deps.setStatusBadge("status-locked", "LOCKED");
    setHidden("compartmentBadge", true);
    deps.setCardsHidden(deps.operatorCardIds, true);
    deps.resetVaultCounts();
    setHidden("lockForm", true);
    setHidden("authRecovery", false);
    setHidden("compSwitcher", true);
    setText("authTitle", "Unlock this local session");
    setText(
      "authLead",
      "Unlock with the passphrase or hardware-key threshold you configured during setup. The resulting session token stays only in this browser tab.",
    );
    deps.setSecretsAccess(false);
    deps.setUnlockGuidance("passphrase");
    deps.updateHeroState("locked");
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

    setHidden("compartmentCard", false);
    setHidden("pushCard", unlocked.length < 2);
    if (unlocked.length >= 2) deps.buildPushSelectors(unlocked);

    setHidden("guideCard", false);
    setHidden("walletManagerCard", false);
    setHidden("profilesCard", false);
    setHidden("xpubCard", false);
    setHidden("treasuryCard", false);
    setHidden("inventoryCard", false);
    setHidden("depositsCard", false);
    setHidden("queueCard", false);
    setHidden("maintenanceCard", false);
    setHidden("backupCard", false);
    setHidden("auditCard", false);
    setHidden("diagCard", false);
    deps.updateHeroState("unlocked", active, unlocked);
  }

  return {
    applySetupUi,
    applyLockedUi,
    applyUnlockedUi,
  };
}
