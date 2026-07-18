/**
 * Shared modal lifecycle.
 *
 * Every modal in the console uses this coordinator so only one can be active,
 * keyboard focus cannot escape it, Escape/backdrop dismissal is consistent,
 * and focus returns to the invoking control when that control still exists.
 */

export interface ModalOptions {
  overlay: HTMLElement;
  dialog: HTMLElement;
  /** Safe caller-selected initial target. Falls back to the first focusable. */
  initialFocus?: HTMLElement | null;
  /** Called after Escape, backdrop dismissal, or replacement by a new modal. */
  onDismiss: () => void;
}

export interface ModalHandle {
  /** Close without treating the close as a cancellation. */
  close(): void;
  /** Close and invoke the cancellation callback. */
  dismiss(): void;
}

let activeModal: ModalHandle | null = null;

export function hasActiveModal(): boolean {
  return activeModal !== null;
}

function hasUsableTabIndex(element: HTMLElement): boolean {
  const raw = element.getAttribute("tabindex");
  return raw != null && raw !== "-1";
}

function isFocusableCandidate(element: HTMLElement): boolean {
  if (!element.isConnected) return false;
  if ((element as HTMLButtonElement | HTMLInputElement).disabled === true) return false;
  if (element.getAttribute("aria-hidden") === "true") return false;
  if (element.getAttribute("tabindex") === "-1") return false;

  const tag = element.tagName.toUpperCase();
  if (tag === "INPUT") return element.getAttribute("type") !== "hidden";
  if (tag === "BUTTON" || tag === "SELECT" || tag === "TEXTAREA" || tag === "SUMMARY") {
    return true;
  }
  if (tag === "A") return element.getAttribute("href") != null;
  return hasUsableTabIndex(element);
}

/** Return current focusable descendants in document order. */
export function focusableElements(root: HTMLElement): HTMLElement[] {
  const focusables: HTMLElement[] = [];
  const visit = (parent: HTMLElement): void => {
    for (const child of Array.from(parent.children) as HTMLElement[]) {
      if (isFocusableCandidate(child)) focusables.push(child);
      visit(child);
    }
  };
  visit(root);
  return focusables;
}

/** Mount and activate a modal. The lifecycle owns overlay removal. */
export function openModal(options: ModalOptions): ModalHandle {
  // Dismiss first so the previous modal restores its underlying invoker before
  // this modal records the focus it should eventually restore.
  activeModal?.dismiss();

  const previousFocus = document.activeElement as HTMLElement | null;
  let closed = false;

  const handle: ModalHandle = {
    close,
    dismiss,
  };

  function restoreFocus(): void {
    if (
      previousFocus &&
      previousFocus.isConnected &&
      typeof previousFocus.focus === "function"
    ) {
      previousFocus.focus();
    }
  }

  function close(): void {
    if (closed) return;
    closed = true;
    document.removeEventListener("keydown", onKeydown, true);
    options.overlay.removeEventListener("click", onBackdropClick);
    options.overlay.remove();
    if (activeModal === handle) activeModal = null;
    restoreFocus();
  }

  function dismiss(): void {
    if (closed) return;
    close();
    options.onDismiss();
  }

  function onBackdropClick(event: MouseEvent): void {
    if (event.target === options.overlay) dismiss();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault?.();
      event.stopPropagation?.();
      dismiss();
      return;
    }
    if (event.key !== "Tab") return;

    // Recompute on every keypress: typed confirmations enable a button while
    // open, and callers may add/remove controls as validation state changes.
    const focusables = focusableElements(options.dialog);
    event.preventDefault?.();
    if (focusables.length === 0) {
      options.dialog.focus();
      return;
    }
    const active = document.activeElement as HTMLElement | null;
    const index = active ? focusables.indexOf(active) : -1;
    const direction = event.shiftKey ? -1 : 1;
    const next =
      index === -1
        ? event.shiftKey
          ? focusables.length - 1
          : 0
        : (index + direction + focusables.length) % focusables.length;
    focusables[next].focus();
  }

  document.body.appendChild(options.overlay);
  activeModal = handle;
  options.overlay.addEventListener("click", onBackdropClick);
  document.addEventListener("keydown", onKeydown, true);

  const requested = options.initialFocus;
  const initial =
    requested && isFocusableCandidate(requested)
      ? requested
      : focusableElements(options.dialog)[0] ?? options.dialog;
  if (initial === options.dialog && !initial.hasAttribute("tabindex")) {
    initial.setAttribute("tabindex", "-1");
  }
  initial.focus();

  return handle;
}
