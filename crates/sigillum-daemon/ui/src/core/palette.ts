import { hasActiveModal, openModal, type ModalHandle } from "../render/modal";

export type CommandPaletteCommandId =
  | "navigate-overview"
  | "navigate-receive"
  | "navigate-portfolio"
  | "navigate-move"
  | "navigate-vault"
  | "refresh-workspace"
  | "run-self-check";

export interface CommandPaletteCommand {
  readonly id: CommandPaletteCommandId;
  readonly label: string;
  readonly description: string;
  readonly keywords: readonly string[];
  run(): unknown;
}

export interface CommandRegistryActions {
  navigate(hash: string): unknown;
  refreshWorkspace(): unknown;
  runSelfCheck(): unknown;
}

export interface CommandPaletteOptions {
  /** The palette is an authenticated workspace affordance, never a lock-screen action. */
  isUnlocked(): boolean;
  commands: readonly CommandPaletteCommand[];
  onError(error: unknown, command: CommandPaletteCommand): void;
}

export interface CommandPaletteController {
  /** Open from a trusted UI affordance. Returns false when policy refuses it. */
  open(): boolean;
  /** Handle Cmd/Ctrl-K. Returns true when opened or consumed for modal safety. */
  handleKeydown(event: KeyboardEvent): boolean;
  /** Close without executing a command. */
  close(): void;
  isOpen(): boolean;
}

export interface CommandPaletteShortcut {
  key: string;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  repeat?: boolean;
  isComposing?: boolean;
}

const NAVIGATION_COMMANDS = [
  {
    id: "navigate-overview",
    label: "Go to Overview",
    description: "Open workspace status and next actions.",
    keywords: ["navigate", "status", "home"],
    hash: "#/overview",
  },
  {
    id: "navigate-receive",
    label: "Go to Receive",
    description: "Open allocations, deposits, and counterparties.",
    keywords: ["navigate", "receiving", "address", "deposit"],
    hash: "#/receive",
  },
  {
    id: "navigate-portfolio",
    label: "Go to Portfolio",
    description: "Open wallets, inventory, and risk findings.",
    keywords: ["navigate", "wallet", "inventory", "risk"],
    hash: "#/portfolio",
  },
  {
    id: "navigate-move",
    label: "Go to Move",
    description: "Open plans, queue work, and maintenance.",
    keywords: ["navigate", "plan", "queue", "maintenance"],
    hash: "#/move",
  },
  {
    id: "navigate-vault",
    label: "Go to Vault",
    description: "Open protected values and workspace diagnostics.",
    keywords: ["navigate", "secret", "key", "diagnostics"],
    hash: "#/vault",
  },
] as const;

/**
 * The complete command allowlist. This factory is intentionally independent
 * from the broad legacy UI action map: only navigation and read/refresh work
 * may enter the palette.
 */
export function createCommandRegistry(
  actions: CommandRegistryActions,
): readonly CommandPaletteCommand[] {
  const commands: CommandPaletteCommand[] = NAVIGATION_COMMANDS.map((command) =>
    Object.freeze({
      id: command.id,
      label: command.label,
      description: command.description,
      keywords: Object.freeze([...command.keywords]),
      run: () => actions.navigate(command.hash),
    }),
  );
  commands.push(
    Object.freeze({
      id: "refresh-workspace",
      label: "Refresh workspace",
      description: "Refresh the current local workspace state.",
      keywords: Object.freeze(["reload", "sync", "status"]),
      run: () => actions.refreshWorkspace(),
    }),
    Object.freeze({
      id: "run-self-check",
      label: "Run self-check",
      description: "Probe local daemon health and operator readiness.",
      keywords: Object.freeze(["doctor", "diagnostics", "health", "verify"]),
      run: () => actions.runSelfCheck(),
    }),
  );
  return Object.freeze(commands);
}

/** Accept exactly one platform modifier plus K; reject every noisy variant. */
export function isCommandPaletteShortcut(event: CommandPaletteShortcut): boolean {
  const platformModifiers = Number(Boolean(event.metaKey)) + Number(Boolean(event.ctrlKey));
  return (
    event.key.toLowerCase() === "k" &&
    platformModifiers === 1 &&
    !event.altKey &&
    !event.shiftKey &&
    !event.repeat &&
    !event.isComposing
  );
}

let paletteSerial = 0;

function removeChildren(element: HTMLElement): void {
  for (const child of Array.from(element.children)) child.remove();
}

function normalizedIndex(index: number, length: number): number {
  return ((index % length) + length) % length;
}

/** Create the single application command palette. */
export function createCommandPalette(
  options: CommandPaletteOptions,
): CommandPaletteController {
  let lifecycle: ModalHandle | null = null;

  function isUnlocked(): boolean {
    try {
      return options.isUnlocked() === true;
    } catch (_) {
      return false;
    }
  }

  function clearLocalState(): void {
    lifecycle = null;
  }

  function close(): void {
    const active = lifecycle;
    clearLocalState();
    active?.close();
  }

  function reportError(error: unknown, command: CommandPaletteCommand): void {
    try {
      options.onError(error, command);
    } catch (_) {
      // Error reporting must never reactivate or strand the modal lifecycle.
    }
  }

  function execute(command: CommandPaletteCommand): void {
    if (!lifecycle) return;
    // Session state can change while the dialog is open. Never dispatch a
    // command against an auth state that is no longer visibly unlocked.
    if (!isUnlocked()) {
      close();
      return;
    }
    close();
    void Promise.resolve()
      .then(() => command.run())
      .catch((error) => reportError(error, command));
  }

  function open(): boolean {
    if (!isUnlocked() || lifecycle || hasActiveModal()) return false;

    paletteSerial += 1;
    const titleId = `commandPaletteTitle${paletteSerial}`;
    const listId = `commandPaletteList${paletteSerial}`;

    const overlay = document.createElement("div");
    overlay.className = "modal-overlay command-palette-overlay";

    const dialog = document.createElement("div");
    dialog.className = "modal-dialog command-palette-dialog";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.setAttribute("aria-labelledby", titleId);
    dialog.setAttribute("data-palette", "dialog");

    const title = document.createElement("h2");
    title.id = titleId;
    title.textContent = "Command palette";

    const shortcut = document.createElement("kbd");
    shortcut.className = "command-palette-shortcut";
    shortcut.textContent = "⌘/Ctrl K";

    const heading = document.createElement("div");
    heading.className = "command-palette-heading";
    heading.appendChild(title);
    heading.appendChild(shortcut);

    const input = document.createElement("input");
    input.type = "text";
    input.className = "command-palette-input";
    input.placeholder = "Type a command…";
    input.autocomplete = "off";
    input.spellcheck = false;
    input.setAttribute("autocapitalize", "off");
    input.setAttribute("role", "combobox");
    input.setAttribute("aria-label", "Filter commands");
    input.setAttribute("aria-autocomplete", "list");
    input.setAttribute("aria-expanded", "true");
    input.setAttribute("aria-controls", listId);
    input.setAttribute("data-palette", "input");

    const list = document.createElement("div");
    list.id = listId;
    list.className = "command-palette-list";
    list.setAttribute("role", "listbox");
    list.setAttribute("aria-label", "Available commands");
    list.setAttribute("data-palette", "list");

    const empty = document.createElement("p");
    empty.className = "command-palette-empty hidden";
    empty.textContent = "No matching commands.";
    empty.setAttribute("role", "status");
    empty.setAttribute("aria-live", "polite");
    empty.setAttribute("data-palette", "empty");

    const hint = document.createElement("p");
    hint.className = "command-palette-hint";
    hint.textContent = "Use ↑ and ↓ to choose, Enter to run, Escape to close.";

    dialog.appendChild(heading);
    dialog.appendChild(input);
    dialog.appendChild(list);
    dialog.appendChild(empty);
    dialog.appendChild(hint);
    overlay.appendChild(dialog);

    let visibleCommands: readonly CommandPaletteCommand[] = options.commands;
    let selectedIndex = visibleCommands.length ? 0 : -1;

    function select(index: number): void {
      if (!visibleCommands.length) {
        selectedIndex = -1;
        input.removeAttribute("aria-activedescendant");
        return;
      }
      selectedIndex = normalizedIndex(index, visibleCommands.length);
      Array.from(list.children).forEach((child, childIndex) => {
        const option = child as HTMLElement;
        const selected = childIndex === selectedIndex;
        option.setAttribute("aria-selected", String(selected));
        option.classList.toggle("is-selected", selected);
        if (selected) input.setAttribute("aria-activedescendant", option.id);
      });
    }

    function renderOptions(): void {
      removeChildren(list);
      visibleCommands.forEach((command) => {
        const option = document.createElement("div");
        option.id = `${listId}-${command.id}`;
        option.className = "command-palette-option";
        option.setAttribute("role", "option");
        option.setAttribute("tabindex", "-1");
        option.setAttribute("data-palette", "option");
        option.setAttribute("data-command-id", command.id);

        const label = document.createElement("span");
        label.className = "command-palette-option-label";
        label.textContent = command.label;
        const description = document.createElement("span");
        description.className = "command-palette-option-description";
        description.textContent = command.description;
        option.appendChild(label);
        option.appendChild(description);
        option.addEventListener("mousemove", () => {
          const nextIndex = visibleCommands.findIndex(
            (candidate) => candidate.id === command.id,
          );
          if (nextIndex >= 0) select(nextIndex);
        });
        option.addEventListener("click", () => execute(command));
        list.appendChild(option);
      });

      empty.classList.toggle("hidden", visibleCommands.length !== 0);
      select(selectedIndex);
    }

    function filter(): void {
      const query = input.value.trim().toLowerCase();
      visibleCommands = query
        ? options.commands.filter((command) =>
            [command.id, command.label, command.description, ...command.keywords]
              .join(" ")
              .toLowerCase()
              .includes(query),
          )
        : options.commands;
      selectedIndex = visibleCommands.length ? 0 : -1;
      renderOptions();
    }

    dialog.addEventListener("keydown", (event) => {
      if (event.isComposing || event.repeat) return;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        select(selectedIndex + (event.key === "ArrowDown" ? 1 : -1));
        return;
      }
      if (event.key === "Enter" && selectedIndex >= 0) {
        event.preventDefault();
        execute(visibleCommands[selectedIndex]);
      }
    });
    input.addEventListener("input", filter);
    renderOptions();

    lifecycle = openModal({
      overlay,
      dialog,
      initialFocus: input,
      onDismiss: clearLocalState,
    });
    return true;
  }

  function handleKeydown(event: KeyboardEvent): boolean {
    if (!isCommandPaletteShortcut(event)) return false;
    // An active modal owns keyboard interaction. Consume the browser chord so
    // it cannot open browser search, but never replace the operator's dialog.
    if (lifecycle || hasActiveModal()) {
      event.preventDefault();
      event.stopPropagation();
      return true;
    }
    // On the lock/setup screen this is not our shortcut; leave browser
    // behavior untouched instead of implying an authenticated affordance.
    if (!open()) return false;
    event.preventDefault();
    event.stopPropagation();
    return true;
  }

  return {
    open,
    handleKeydown,
    close,
    isOpen: () => lifecycle !== null,
  };
}
