import { clearSessionToken } from "../api/session";

export interface SessionActionDeps {
  api: (method: string, path: string, body?: unknown) => Promise<any>;
  toast: (message: string, type?: string) => void;
  refresh: () => unknown | Promise<unknown>;
  confirm?: (message: string) => boolean;
}

export function isAlreadyUnlockedConflict(message: unknown): boolean {
  return String(message || "").toLowerCase().includes("already unlocked");
}

function passphraseInput(): HTMLInputElement | null {
  return document.getElementById("passphrase") as HTMLInputElement | null;
}

export function createSessionActions(deps: SessionActionDeps) {
  const confirmAction =
    deps.confirm || ((message: string) => globalThis.confirm(message));

  async function unlock(): Promise<void> {
    const input = passphraseInput();
    const passphrase = input?.value || "";
    if (!passphrase) return;

    const response = await deps.api("POST", "/api/unlock", { passphrase });
    if (response.error) {
      if (isAlreadyUnlockedConflict(response.error)) {
        deps.toast("Session already active. Refreshing workspace...");
        await deps.refresh();
        return;
      }
      deps.toast(response.error, "error");
      return;
    }

    if (input) input.value = "";
    if (response.unlocked_compartments && response.unlocked_compartments.length > 0) {
      const labels = response.unlocked_compartments
        .map((compartment: { label: string }) => compartment.label)
        .join(", ");
      deps.toast("Unlocked: " + labels);
    } else {
      deps.toast("Unlocked");
    }
    await deps.refresh();
  }

  async function lock(): Promise<void> {
    if (!confirmAction("Lock all compartments? Master keys will be zeroized from memory.")) {
      return;
    }
    const response = await deps.api("POST", "/api/lock");
    if (response.error) {
      deps.toast(response.error, "error");
      return;
    }
    clearSessionToken();
    deps.toast("All compartments locked");
    await deps.refresh();
  }

  async function logoutSession(): Promise<void> {
    const response = await deps.api("POST", "/api/session/revoke");
    if (response.error) {
      deps.toast(response.error, "error");
      return;
    }
    clearSessionToken();
    deps.toast("Session logged out");
    await deps.refresh();
  }

  return {
    lock,
    logoutSession,
    unlock,
  };
}
