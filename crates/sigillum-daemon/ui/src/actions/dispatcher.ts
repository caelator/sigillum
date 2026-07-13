import { isSessionContextChangedError } from "../api/session";

export type UiAction = (...args: unknown[]) => unknown | Promise<unknown>;
export type UiActionMap = Record<string, UiAction>;

export interface DispatchOptions {
  actions: UiActionMap;
  toast: (message: string, type?: string) => void;
  quietActions?: string[];
}

function coerceActionArg(value: string | undefined, type: string | undefined): unknown {
  if (type === "number") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : value;
  }
  return value;
}

export function collectActionArgs(actionEl: HTMLElement): unknown[] {
  const args: unknown[] = [];
  for (let index = 0; ; index += 1) {
    const key = "arg" + index;
    if (!(key in actionEl.dataset)) break;
    args.push(coerceActionArg(actionEl.dataset[key], actionEl.dataset[key + "Type"]));
  }
  if (actionEl.dataset.self === "append") args.push(actionEl);
  return args;
}

export function dispatchDataAction(
  actionEl: HTMLElement,
  options: DispatchOptions,
): void {
  const actionName = actionEl.dataset.action || "";
  const action = options.actions[actionName];
  if (typeof action !== "function") {
    console.warn("Unknown UI action:", actionName);
    return;
  }

  const quietActions = options.quietActions || [];
  const shouldShowBusy =
    actionEl.tagName === "BUTTON" &&
    !(actionEl as HTMLButtonElement).disabled &&
    !quietActions.includes(actionName);
  if (shouldShowBusy) {
    (actionEl as HTMLButtonElement).disabled = true;
    actionEl.classList.add("is-busy");
    actionEl.setAttribute("aria-busy", "true");
  }

  Promise.resolve(action(...collectActionArgs(actionEl)))
    .catch((error) => {
      if (isSessionContextChangedError(error)) return;
      console.error("UI action failed:", actionName, error);
      options.toast("Action failed: " + actionName, "error");
    })
    .finally(() => {
      if (!shouldShowBusy || !actionEl.isConnected) return;
      (actionEl as HTMLButtonElement).disabled = false;
      actionEl.classList.remove("is-busy");
      actionEl.removeAttribute("aria-busy");
    });
}

export function handleActionEvent(event: Event, options: DispatchOptions): void {
  const actionEl =
    event.target instanceof Element
      ? (event.target.closest("[data-action]") as HTMLElement | null)
      : null;
  if (!actionEl) return;
  if (actionEl.tagName === "BUTTON") event.preventDefault();
  dispatchDataAction(actionEl, options);
}
