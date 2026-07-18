// Shared confirmation dialog — the ONLY way dangerous actions are gated in
// this UI. Native confirm()/prompt() are banned: they clash with the styled
// dark UI, are untestable in the fake-DOM harness, and scatter the danger
// model across ad-hoc call sites.
//
// Consequence matrix (which tier guards which action):
//
//   Tier     | Used for                                             | Examples
//   ---------+------------------------------------------------------+-------------------------
//   inform   | Irreversible-but-local notices that only need one    | clipboard-unavailable
//            | acknowledgement; no state is destroyed by the ack    | fallback
//   confirm  | Value-moving on-chain actions AND destructive local  | queue process, deposit
//            | deletes. Descriptive copy + Cancel / danger button.  | sweep enqueue, address
//            | Anything ≥ this tier needs an explicit danger click. | rotate, party/profile/
//            |                                                      | secret/key deletes, lock
//            |                                                      | all, snapshot restore
//   typed    | Bulk or irreversible vault-level operations only.    | bulk plan enqueue
//            | The action button stays disabled until the operator  | (server-computed phrase),
//            | types the exact required phrase shown in the dialog. | local data reset
//
// Rule of thumb: value-moving on-chain actions ≥ confirm; destructive local
// deletes = confirm; typed phrase is reserved for bulk/irreversible
// vault-level operations.

import { openModal, type ModalHandle } from "./modal";

export type ConfirmTier = "inform" | "confirm" | "typed";

export interface ConfirmDialogOptions {
  /** Short dialog title (also the accessible label). */
  title: string;
  /** Descriptive copy stating the consequence of proceeding. */
  body: string;
  /** Label of the primary action button. Defaults per tier. */
  actionLabel?: string;
  /** Label of the cancel button. Defaults to "Cancel". */
  cancelLabel?: string;
  /** Required exact phrase (typed tier only); shown to the operator. */
  phrase?: string;
  /** Optional value rendered in a read-only field (e.g. copy fallback). */
  valueDisplay?: string;
  /**
   * Optional single checkbox rendered between the body and the actions
   * (e.g. opting a profile delete into the forget-history cascade). The
   * decision carries its state; see confirmDangerDialogWithCheckbox.
   */
  checkbox?: { label: string; checked?: boolean };
}

/** A dialog decision: whether the operator confirmed, and the checkbox state. */
export interface ConfirmDecision {
  confirmed: boolean;
  checked: boolean;
}

let dialogSerial = 0;

function defaultActionLabel(tier: ConfirmTier): string {
  if (tier === "inform") return "OK";
  return "Confirm";
}

/**
 * Open a modal confirmation dialog and resolve with the operator's decision.
 * Resolves true only via the action button (gated by the typed phrase on the
 * typed tier); resolves false on Cancel, Escape, or backdrop click.
 */
export function confirmDialog(
  tier: ConfirmTier,
  options: ConfirmDialogOptions,
): Promise<boolean> {
  return confirmDialogDecision(tier, options).then((decision) => decision.confirmed);
}

/**
 * Full-decision variant: resolves the confirmation AND the optional
 * checkbox state (false when no checkbox was rendered or the dialog was
 * cancelled).
 */
export function confirmDialogDecision(
  tier: ConfirmTier,
  options: ConfirmDialogOptions,
): Promise<ConfirmDecision> {
  return new Promise((resolve) => {
    dialogSerial += 1;
    const titleId = "confirmDialogTitle" + String(dialogSerial);

    const overlay = document.createElement("div");
    overlay.className = "confirm-overlay";
    overlay.setAttribute("data-confirm-overlay", tier);

    const dialog = document.createElement("div");
    dialog.className = "card confirm-dialog";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-labelledby", titleId);

    const title = document.createElement("h2");
    title.id = titleId;
    title.setAttribute("data-confirm-title", "");
    title.textContent = options.title;

    const body = document.createElement("p");
    body.className = "confirm-body";
    body.setAttribute("data-confirm-body", "");
    body.textContent = options.body;

    dialog.appendChild(title);
    dialog.appendChild(body);

    let checkboxInput: HTMLInputElement | null = null;
    if (options.checkbox != null) {
      const checkboxLabel = document.createElement("label");
      checkboxLabel.className = "confirm-checkbox";
      checkboxInput = document.createElement("input") as HTMLInputElement;
      checkboxInput.type = "checkbox";
      checkboxInput.checked = options.checkbox.checked === true;
      checkboxInput.setAttribute("data-confirm-checkbox", "");
      const checkboxText = document.createElement("span");
      checkboxText.textContent = options.checkbox.label;
      checkboxLabel.appendChild(checkboxInput);
      checkboxLabel.appendChild(checkboxText);
      dialog.appendChild(checkboxLabel);
    }

    let valueInput: HTMLInputElement | null = null;
    if (options.valueDisplay != null) {
      valueInput = document.createElement("input") as HTMLInputElement;
      valueInput.className = "mono confirm-value";
      valueInput.readOnly = true;
      valueInput.value = options.valueDisplay;
      valueInput.setAttribute("data-confirm-value", "");
      valueInput.setAttribute("aria-label", options.title);
      dialog.appendChild(valueInput);
    }

    let phraseInput: HTMLInputElement | null = null;
    if (tier === "typed") {
      const phraseLabel = document.createElement("p");
      phraseLabel.className = "confirm-phrase-label";
      phraseLabel.textContent = "Type this phrase exactly to proceed:";
      dialog.appendChild(phraseLabel);

      const phrase = document.createElement("p");
      phrase.className = "mono confirm-phrase";
      phrase.setAttribute("data-confirm-phrase", "");
      phrase.textContent = options.phrase || "";
      dialog.appendChild(phrase);

      phraseInput = document.createElement("input") as HTMLInputElement;
      phraseInput.className = "mono confirm-input";
      phraseInput.setAttribute("data-confirm-input", "");
      phraseInput.setAttribute("aria-label", "Confirmation phrase");
      phraseInput.autocomplete = "off";
      phraseInput.spellcheck = false;
      dialog.appendChild(phraseInput);
    }

    const actions = document.createElement("div");
    actions.className = "confirm-actions";

    let cancelButton: HTMLButtonElement | null = null;
    if (tier !== "inform") {
      cancelButton = document.createElement("button") as HTMLButtonElement;
      cancelButton.type = "button";
      cancelButton.className = "btn-ghost";
      cancelButton.setAttribute("data-confirm-cancel", "");
      cancelButton.textContent = options.cancelLabel || "Cancel";
      actions.appendChild(cancelButton);
    }

    const actionButton = document.createElement("button") as HTMLButtonElement;
    actionButton.type = "button";
    actionButton.className = tier === "inform" ? "btn-primary" : "btn-danger";
    actionButton.setAttribute("data-confirm-action", "");
    actionButton.textContent = options.actionLabel || defaultActionLabel(tier);
    if (tier === "typed") actionButton.disabled = true;
    actions.appendChild(actionButton);

    dialog.appendChild(actions);
    overlay.appendChild(dialog);

    let settled = false;
    let lifecycle: ModalHandle | null = null;
    const settle = (confirmed: boolean): void => {
      if (settled) return;
      settled = true;
      document.removeEventListener("keydown", onConfirmKeydown, true);
      lifecycle?.close();
      resolve({
        confirmed,
        checked: confirmed && checkboxInput != null && checkboxInput.checked,
      });
    };

    const phraseMatches = (): boolean =>
      phraseInput != null &&
      options.phrase != null &&
      phraseInput.value.trim() === options.phrase;

    const onConfirmKeydown = (event: KeyboardEvent): void => {
      // Enter never triggers the danger action on the typed tier: it only
      // submits when the phrase input itself has focus and already matches.
      if (
        event.key === "Enter" &&
        tier === "typed" &&
        phraseInput != null &&
        event.target === phraseInput &&
        phraseMatches()
      ) {
        if (typeof event.preventDefault === "function") event.preventDefault();
        settle(true);
      }
    };

    document.addEventListener("keydown", onConfirmKeydown, true);
    cancelButton?.addEventListener("click", () => settle(false));
    actionButton.addEventListener("click", () => {
      if (tier === "typed" && !phraseMatches()) return;
      settle(true);
    });
    phraseInput?.addEventListener("input", () => {
      actionButton.disabled = !phraseMatches();
    });

    // Initial focus lands on the safe control: the typed phrase input (must
    // be filled before anything can fire) or Cancel. Inform has one button.
    const initialFocus =
      tier === "inform" ? actionButton : phraseInput || cancelButton || actionButton;
    lifecycle = openModal({
      overlay,
      dialog,
      initialFocus,
      onDismiss: () => settle(false),
    });
  });
}

/** Single-acknowledgement notice (irreversible-but-local information). */
export function informDialog(options: ConfirmDialogOptions): Promise<boolean> {
  return confirmDialog("inform", options);
}

/** Descriptive copy + Cancel / danger action. The default guard tier. */
export function confirmDangerDialog(options: ConfirmDialogOptions): Promise<boolean> {
  return confirmDialog("confirm", options);
}

/** Danger tier with the decision exposed (for the optional checkbox). */
export function confirmDangerDialogDecision(
  options: ConfirmDialogOptions,
): Promise<ConfirmDecision> {
  return confirmDialogDecision("confirm", options);
}

/** Like confirm, but the action stays disabled until the phrase is typed. */
export function confirmTypedDialog(options: ConfirmDialogOptions): Promise<boolean> {
  return confirmDialog("typed", options);
}
