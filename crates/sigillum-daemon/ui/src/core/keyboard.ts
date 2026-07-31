import { hasActiveModal } from "../render/modal";

export interface LegacyEnterActions {
  unlock: () => unknown;
  fido2Unlock: () => unknown;
  wizInitPassphrase: () => unknown;
  wizRegisterKey: () => unknown;
  wizRegisterAdditionalKey: () => unknown;
  wizSetNewPin: () => unknown;
  wizSetAdditionalKeyPin: () => unknown;
  wizAddCustomComp: () => unknown;
}

export type LegacyEnterActionName = keyof LegacyEnterActions;

const ACTION_BY_TARGET_ID: Readonly<Record<string, LegacyEnterActionName>> = {
  passphrase: "unlock",
  fido2Pin: "fido2Unlock",
  fido2TapCount: "fido2Unlock",
  wizPassphraseConfirm: "wizInitPassphrase",
  wizFido2Label: "wizRegisterKey",
  wizAdditionalKeyLabel: "wizRegisterAdditionalKey",
  wizNewFido2PinConfirm: "wizSetNewPin",
  wizAdditionalNewPinConfirm: "wizSetAdditionalKeyPin",
  wizCustomThreshold: "wizAddCustomComp",
};

/** Resolve a legacy standalone input to its exact Enter action. */
export function legacyEnterActionForId(id: string): LegacyEnterActionName | null {
  return ACTION_BY_TARGET_ID[id] ?? null;
}

/**
 * Dispatch Enter for the remaining standalone inputs that predate forms.
 * Native forms retain their own submit semantics, and an active modal owns
 * all keyboard interaction until it closes.
 */
export function handleLegacyEnter(
  event: KeyboardEvent,
  actions: LegacyEnterActions,
  modalActive = hasActiveModal(),
): boolean {
  if (
    event.key !== "Enter" ||
    event.isComposing ||
    event.repeat ||
    modalActive
  ) {
    return false;
  }

  const target = event.target as HTMLInputElement | null;
  const actionName = legacyEnterActionForId(target?.id ?? "");
  if (!target || !actionName) return false;
  if (target.form != null) return false;

  event.preventDefault();
  actions[actionName]();
  return true;
}
