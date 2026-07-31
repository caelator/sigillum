import { el } from "../core/dom";
import { openModal, type ModalHandle } from "./modal";

export interface SecretPromptOptions {
  title: string;
  inputLabel: string;
  placeholder?: string;
  submitLabel?: string;
  cancelLabel?: string;
}

export interface SecretPromptDecision {
  submitted: boolean;
  value: string;
}

let promptSerial = 0;

/**
 * Prompt for a secret without conflating cancellation with an explicit blank.
 * Escape, Cancel, and backdrop return `submitted: false`; submitting an empty
 * field returns `submitted: true, value: ""`.
 */
export function promptSecret(options: SecretPromptOptions): Promise<SecretPromptDecision> {
  return new Promise((resolve) => {
    promptSerial += 1;
    const titleId = "secretPromptTitle" + String(promptSerial);

    const input = el("input", {
      class: "input-wide secret-prompt-input",
      attrs: {
        type: "password",
        placeholder: options.placeholder || "",
        "aria-label": options.inputLabel,
        "data-secret-prompt-input": "",
        autocomplete: "off",
        autocapitalize: "off",
        spellcheck: "false",
      },
    }) as HTMLInputElement;
    const cancelButton = el("button", {
      class: "btn-ghost",
      text: options.cancelLabel || "Cancel",
      attrs: { type: "button", "data-secret-prompt-cancel": "" },
    }) as HTMLButtonElement;
    const submitButton = el("button", {
      class: "btn-primary",
      text: options.submitLabel || "OK",
      attrs: { type: "submit", "data-secret-prompt-submit": "" },
    }) as HTMLButtonElement;
    const actions = el(
      "div",
      { class: "modal-actions" },
      cancelButton,
      submitButton,
    );
    const form = el(
      "form",
      {
        class: "secret-prompt-form",
        attrs: { "data-secret-prompt-form": "" },
      },
      input,
      actions,
    ) as HTMLFormElement;
    const dialog = el(
      "div",
      {
        class: "card modal-dialog secret-prompt-dialog",
        attrs: {
          role: "dialog",
          "aria-modal": "true",
          "aria-labelledby": titleId,
        },
      },
      el("h2", { text: options.title, attrs: { id: titleId } }),
      form,
    );
    const overlay = el(
      "div",
      {
        class: "modal-overlay",
        attrs: { "data-secret-prompt-overlay": "" },
      },
      dialog,
    );

    let settled = false;
    let lifecycle: ModalHandle | null = null;
    const settle = (decision: SecretPromptDecision): void => {
      if (settled) return;
      settled = true;
      lifecycle?.close();
      resolve(decision);
    };

    form.addEventListener("submit", (event) => {
      event.preventDefault();
      settle({ submitted: true, value: input.value });
    });
    cancelButton.addEventListener("click", () => {
      settle({ submitted: false, value: "" });
    });

    lifecycle = openModal({
      overlay,
      dialog,
      initialFocus: input,
      onDismiss: () => settle({ submitted: false, value: "" }),
    });
  });
}
