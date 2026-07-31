import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import {
  createCommandPalette,
  createCommandRegistry,
  isCommandPaletteShortcut,
  type CommandPaletteCommand,
  type CommandPaletteController,
} from "../src/core/palette";
import { confirmDangerDialog } from "../src/render/confirm";
import { hasActiveModal } from "../src/render/modal";
import { installDom, type FakeElement } from "./dom-fixture";

const COMMAND_IDS = [
  "navigate-overview",
  "navigate-receive",
  "navigate-portfolio",
  "navigate-move",
  "navigate-vault",
  "refresh-workspace",
  "run-self-check",
] as const;

interface ActionHarness {
  navigated: string[];
  refreshes: number;
  selfChecks: number;
  commands: readonly CommandPaletteCommand[];
}

function actionHarness(overrides?: {
  refreshWorkspace?: () => unknown;
  runSelfCheck?: () => unknown;
}): ActionHarness {
  const harness: ActionHarness = {
    navigated: [],
    refreshes: 0,
    selfChecks: 0,
    commands: [],
  };
  harness.commands = createCommandRegistry({
    navigate: (hash) => harness.navigated.push(hash),
    refreshWorkspace: () => {
      harness.refreshes += 1;
      return overrides?.refreshWorkspace?.();
    },
    runSelfCheck: () => {
      harness.selfChecks += 1;
      return overrides?.runSelfCheck?.();
    },
  });
  return harness;
}

function paletteElement(name: string): FakeElement {
  const found = (document.body as unknown as FakeElement).querySelector(
    `[data-palette="${name}"]`,
  );
  if (!found) throw new Error(`missing palette element: ${name}`);
  return found;
}

function shortcutEvent(
  overrides: Partial<KeyboardEvent> = {},
  counters = { prevented: 0, stopped: 0 },
): KeyboardEvent {
  return {
    key: "k",
    metaKey: true,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    isComposing: false,
    preventDefault: () => {
      counters.prevented += 1;
    },
    stopPropagation: () => {
      counters.stopped += 1;
    },
    ...overrides,
  } as unknown as KeyboardEvent;
}

function keyEvent(
  key: string,
  target: FakeElement,
  counters = { prevented: 0, stopped: 0 },
): KeyboardEvent {
  return {
    type: "keydown",
    key,
    target,
    repeat: false,
    isComposing: false,
    preventDefault: () => {
      counters.prevented += 1;
    },
    stopPropagation: () => {
      counters.stopped += 1;
    },
  } as unknown as KeyboardEvent;
}

function createHarnessPalette(args?: {
  unlocked?: () => boolean;
  actions?: ActionHarness;
  onError?: (error: unknown, command: CommandPaletteCommand) => void;
}): { palette: CommandPaletteController; actions: ActionHarness } {
  const actions = args?.actions ?? actionHarness();
  return {
    actions,
    palette: createCommandPalette({
      isUnlocked: args?.unlocked ?? (() => true),
      commands: actions.commands,
      onError: args?.onError ?? (() => {}),
    }),
  };
}

test("registry is the exact frozen safe allowlist", () => {
  installDom();
  const harness = actionHarness();

  deepEqual(
    harness.commands.map((command) => command.id),
    [...COMMAND_IDS],
  );
  equal(Object.isFrozen(harness.commands), true);
  equal(harness.commands.every((command) => Object.isFrozen(command)), true);
  harness.commands.forEach((command) => {
    equal(
      /(?:sign|broadcast|delete|lock|sweep|enqueue|send|transfer)/i.test(
        `${command.id} ${command.label}`,
      ),
      false,
    );
  });

  harness.commands[0].run();
  harness.commands[1].run();
  harness.commands[2].run();
  harness.commands[3].run();
  harness.commands[4].run();
  harness.commands[5].run();
  harness.commands[6].run();
  deepEqual(harness.navigated, [
    "#/overview",
    "#/receive",
    "#/portfolio",
    "#/move",
    "#/vault",
  ]);
  equal(harness.refreshes, 1);
  equal(harness.selfChecks, 1);
});

test("shortcut accepts exact Cmd/Ctrl-K and rejects modified or noisy variants", () => {
  equal(isCommandPaletteShortcut(shortcutEvent()), true);
  equal(
    isCommandPaletteShortcut(shortcutEvent({ metaKey: false, ctrlKey: true })),
    true,
  );
  equal(
    isCommandPaletteShortcut(shortcutEvent({ metaKey: true, ctrlKey: true })),
    false,
  );
  equal(
    isCommandPaletteShortcut(shortcutEvent({ metaKey: false, ctrlKey: false })),
    false,
  );
  equal(isCommandPaletteShortcut(shortcutEvent({ key: "p" })), false);
  equal(isCommandPaletteShortcut(shortcutEvent({ altKey: true })), false);
  equal(isCommandPaletteShortcut(shortcutEvent({ shiftKey: true })), false);
  equal(isCommandPaletteShortcut(shortcutEvent({ repeat: true })), false);
  equal(isCommandPaletteShortcut(shortcutEvent({ isComposing: true })), false);
});

test("locked state refuses without consuming; an active modal consumes without replacement", async () => {
  installDom();
  let unlocked = false;
  const { palette } = createHarnessPalette({ unlocked: () => unlocked });
  const lockedCounters = { prevented: 0, stopped: 0 };

  equal(palette.handleKeydown(shortcutEvent({}, lockedCounters)), false);
  equal(palette.isOpen(), false);
  equal(lockedCounters.prevented, 0);
  equal(lockedCounters.stopped, 0);

  unlocked = true;
  const confirmation = confirmDangerDialog({
    title: "Existing modal",
    body: "The palette must not replace this decision.",
  });
  equal(hasActiveModal(), true);
  const modalCounters = { prevented: 0, stopped: 0 };
  equal(palette.handleKeydown(shortcutEvent({}, modalCounters)), true);
  equal(palette.isOpen(), false);
  equal(
    (document.body as unknown as FakeElement).querySelector(
      '[data-confirm-overlay="confirm"]',
    ) != null,
    true,
  );
  equal(modalCounters.prevented, 1);
  equal(modalCounters.stopped, 1);

  (
    (document.body as unknown as FakeElement).querySelector(
      '[data-confirm-cancel=""]',
    ) as FakeElement
  ).click();
  equal(await confirmation, false);
  equal(hasActiveModal(), false);
});

test("a second palette shortcut is consumed without replacing the open palette", () => {
  installDom();
  const { palette } = createHarnessPalette();
  equal(palette.open(), true);
  const originalDialog = paletteElement("dialog");
  const counters = { prevented: 0, stopped: 0 };

  equal(palette.handleKeydown(shortcutEvent({}, counters)), true);
  equal(palette.isOpen(), true);
  equal(paletteElement("dialog"), originalDialog);
  equal(counters.prevented, 1);
  equal(counters.stopped, 1);
  palette.close();
  equal(hasActiveModal(), false);
});

test("palette focuses its combobox, filters, wraps, and executes the selected command", async () => {
  const dom = installDom();
  const invoker = dom.document.createElement("button");
  dom.document.body.appendChild(invoker);
  invoker.focus();
  const { palette, actions } = createHarnessPalette();
  const shortcutCounters = { prevented: 0, stopped: 0 };

  equal(palette.handleKeydown(shortcutEvent({}, shortcutCounters)), true);
  equal(shortcutCounters.prevented, 1);
  equal(shortcutCounters.stopped, 1);
  equal(palette.isOpen(), true);
  equal(hasActiveModal(), true);

  const dialog = paletteElement("dialog");
  const input = paletteElement("input");
  const list = paletteElement("list");
  equal(dom.document.activeElement, input);
  equal(dialog.getAttribute("role"), "dialog");
  equal(dialog.getAttribute("aria-modal"), "true");
  equal(input.getAttribute("role"), "combobox");
  equal(input.getAttribute("aria-expanded"), "true");
  equal(list.getAttribute("role"), "listbox");
  equal(list.children.length, 7);
  equal(list.children[0].getAttribute("aria-selected"), "true");

  const upCounters = { prevented: 0, stopped: 0 };
  dialog.dispatchEvent(keyEvent("ArrowUp", input, upCounters) as any);
  equal(upCounters.prevented, 1);
  equal(
    input.getAttribute("aria-activedescendant"),
    list.children[6].id,
  );
  dialog.dispatchEvent(keyEvent("ArrowDown", input) as any);
  equal(
    input.getAttribute("aria-activedescendant"),
    list.children[0].id,
  );

  input.value = "health";
  input.dispatchEvent({ type: "input", target: input });
  equal(list.children.length, 1);
  equal(list.children[0].getAttribute("data-command-id"), "run-self-check");
  equal(list.children[0].getAttribute("aria-selected"), "true");

  const enterCounters = { prevented: 0, stopped: 0 };
  dialog.dispatchEvent(keyEvent("Enter", input, enterCounters) as any);
  equal(enterCounters.prevented, 1);
  equal(palette.isOpen(), false);
  equal(hasActiveModal(), false);
  equal(dom.document.activeElement, invoker);
  await Promise.resolve();
  equal(actions.selfChecks, 1);
});

test("Escape and backdrop dismiss safely and restore the connected invoker", () => {
  const dom = installDom();
  const invoker = dom.document.createElement("button");
  dom.document.body.appendChild(invoker);
  const { palette } = createHarnessPalette();

  invoker.focus();
  equal(palette.open(), true);
  const input = paletteElement("input");
  const tabCounters = { prevented: 0, stopped: 0 };
  dom.document.dispatchEvent(keyEvent("Tab", input, tabCounters) as any);
  equal(tabCounters.prevented, 1);
  equal(dom.document.activeElement, input);

  const escapeCounters = { prevented: 0, stopped: 0 };
  dom.document.dispatchEvent(keyEvent("Escape", input, escapeCounters) as any);
  equal(escapeCounters.prevented, 1);
  equal(escapeCounters.stopped, 1);
  equal(palette.isOpen(), false);
  equal(dom.document.activeElement, invoker);

  equal(palette.open(), true);
  const dialog = paletteElement("dialog");
  const overlay = dialog.parentNode as FakeElement;
  overlay.dispatchEvent({ type: "click", target: overlay });
  equal(palette.isOpen(), false);
  equal(hasActiveModal(), false);
  equal(dom.document.activeElement, invoker);
});

test("no-results state leaves Enter inert and keeps the palette dismissible", () => {
  installDom();
  const { palette, actions } = createHarnessPalette();
  equal(palette.open(), true);
  const dialog = paletteElement("dialog");
  const input = paletteElement("input");
  const list = paletteElement("list");
  const empty = paletteElement("empty");

  input.value = "definitely-not-a-command";
  input.dispatchEvent({ type: "input", target: input });
  equal(list.children.length, 0);
  equal(input.getAttribute("aria-activedescendant"), null);
  equal(empty.classList.contains("hidden"), false);

  const counters = { prevented: 0, stopped: 0 };
  dialog.dispatchEvent(keyEvent("Enter", input, counters) as any);
  equal(counters.prevented, 0);
  equal(palette.isOpen(), true);
  equal(actions.navigated.length, 0);
  equal(actions.refreshes, 0);
  equal(actions.selfChecks, 0);
  palette.close();
  equal(hasActiveModal(), false);
});

test("async command failures report after closing and never strand modal state", async () => {
  installDom();
  const failure = new Error("probe unavailable");
  const actions = actionHarness({ runSelfCheck: () => Promise.reject(failure) });
  const errors: Array<{ error: unknown; id: string }> = [];
  const { palette } = createHarnessPalette({
    actions,
    onError: (error, command) => errors.push({ error, id: command.id }),
  });

  equal(palette.open(), true);
  const dialog = paletteElement("dialog");
  const input = paletteElement("input");
  input.value = "health";
  input.dispatchEvent({ type: "input", target: input });
  dialog.dispatchEvent(keyEvent("Enter", input) as any);
  equal(palette.isOpen(), false);
  equal(hasActiveModal(), false);

  await new Promise((resolve) => setTimeout(resolve, 0));
  deepEqual(errors, [{ error: failure, id: "run-self-check" }]);
  equal(palette.open(), true);
  palette.close();
  equal(hasActiveModal(), false);
});

test("execution fails closed if the workspace locks while the palette is open", async () => {
  installDom();
  let unlocked = true;
  const actions = actionHarness();
  const { palette } = createHarnessPalette({
    actions,
    unlocked: () => unlocked,
  });

  equal(palette.open(), true);
  const dialog = paletteElement("dialog");
  const input = paletteElement("input");
  unlocked = false;
  dialog.dispatchEvent(keyEvent("Enter", input) as any);
  equal(palette.isOpen(), false);
  equal(hasActiveModal(), false);
  await Promise.resolve();
  equal(actions.navigated.length, 0);
  equal(actions.refreshes, 0);
  equal(actions.selfChecks, 0);
});
