const SENSITIVE_TEXTAREA_IDS = [
  "walletImportSeedMnemonic",
  "seedMnemonic",
] as const;

export interface SessionPrivacyGuardOptions {
  cardIds: readonly string[];
  resetters: Array<() => void>;
  enhanceRestoredUi: () => void;
  reportResetError?: (error: unknown) => void;
  document?: Document;
}

/**
 * Capture the static operator-card shell once, then restore that safe shell at
 * every session boundary. Delegated data-action handlers survive because card
 * containers stay in place; all rendered account data, plaintext reveals,
 * one-time seed phrases, and unsaved secret-bearing controls are discarded.
 */
export function createSessionPrivacyGuard(options: SessionPrivacyGuardOptions) {
  const ownerDocument = options.document || document;
  const initialCardMarkup = new Map<string, string>();
  options.cardIds.forEach((id) => {
    const card = ownerDocument.getElementById(id);
    if (card) initialCardMarkup.set(id, card.innerHTML);
  });
  let generation = 0;

  function scrub(): void {
    generation += 1;
    const resetErrors: unknown[] = [];
    options.resetters.forEach((reset) => {
      try {
        reset();
      } catch (error) {
        resetErrors.push(error);
      }
    });

    // Remove plaintext reveals while they are still connected. Replacing a
    // card's markup first would detach the old list and let a pending reveal
    // timer retain its plaintext subtree outside document queries.
    ownerDocument.querySelectorAll(".secret-value").forEach((node) => node.remove());

    initialCardMarkup.forEach((markup, id) => {
      const card = ownerDocument.getElementById(id);
      if (card) card.innerHTML = markup;
    });

    ownerDocument
      .querySelectorAll<HTMLInputElement>('input[type="password"], input[type="file"]')
      .forEach((field) => {
        field.value = "";
      });
    SENSITIVE_TEXTAREA_IDS.forEach((id) => {
      const field = ownerDocument.getElementById(id) as HTMLTextAreaElement | null;
      if (field) field.value = "";
    });

    ["toastStack", "compSwitcher"].forEach((id) => {
      const element = ownerDocument.getElementById(id);
      if (element) element.innerHTML = "";
    });
    const compartmentBadge = ownerDocument.getElementById("compartmentBadge");
    if (compartmentBadge) compartmentBadge.textContent = "";

    options.enhanceRestoredUi();
    const reportResetError =
      options.reportResetError ||
      ((error: unknown) => console.error("session privacy reset failed", error));
    resetErrors.forEach((error) => {
      // Teardown is deliberately best-effort across every resetter. A broken
      // cache reset must never abort the DOM scrub or leave locked cards live.
      try {
        reportResetError(error);
      } catch (_) {}
    });
  }

  return {
    generation: () => generation,
    scrub,
  };
}
