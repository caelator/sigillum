import { equal } from "node:assert/strict";
import { test } from "node:test";

import {
  handleLegacyEnter,
  legacyEnterActionForId,
  type LegacyEnterActionName,
  type LegacyEnterActions,
} from "../src/core/keyboard";
import { confirmDangerDialog } from "../src/render/confirm";
import { installDom } from "./dom-fixture";

const ACTION_NAMES: LegacyEnterActionName[] = [
  "unlock",
  "fido2Unlock",
  "wizInitPassphrase",
  "wizRegisterKey",
  "wizRegisterAdditionalKey",
  "wizSetNewPin",
  "wizSetAdditionalKeyPin",
  "wizAddCustomComp",
];

function actionHarness(): {
  actions: LegacyEnterActions;
  counts: Record<LegacyEnterActionName, number>;
  reset: () => void;
} {
  const counts = Object.fromEntries(
    ACTION_NAMES.map((name) => [name, 0]),
  ) as Record<LegacyEnterActionName, number>;
  const invoke = (name: LegacyEnterActionName) => () => {
    counts[name] += 1;
  };
  const actions: LegacyEnterActions = {
    unlock: invoke("unlock"),
    fido2Unlock: invoke("fido2Unlock"),
    wizInitPassphrase: invoke("wizInitPassphrase"),
    wizRegisterKey: invoke("wizRegisterKey"),
    wizRegisterAdditionalKey: invoke("wizRegisterAdditionalKey"),
    wizSetNewPin: invoke("wizSetNewPin"),
    wizSetAdditionalKeyPin: invoke("wizSetAdditionalKeyPin"),
    wizAddCustomComp: invoke("wizAddCustomComp"),
  };
  return {
    actions,
    counts,
    reset: () => ACTION_NAMES.forEach((name) => (counts[name] = 0)),
  };
}

function keyboardEvent(
  key: string,
  target: unknown,
  preventDefault: () => void,
): KeyboardEvent {
  return { key, target, preventDefault } as unknown as KeyboardEvent;
}

test("legacy Enter targets dispatch their exact action and prevent native handling", () => {
  const dom = installDom();
  const harness = actionHarness();
  const mappings: Array<[string, LegacyEnterActionName]> = [
    ["passphrase", "unlock"],
    ["fido2Pin", "fido2Unlock"],
    ["fido2TapCount", "fido2Unlock"],
    ["wizPassphraseConfirm", "wizInitPassphrase"],
    ["wizFido2Label", "wizRegisterKey"],
    ["wizAdditionalKeyLabel", "wizRegisterAdditionalKey"],
    ["wizNewFido2PinConfirm", "wizSetNewPin"],
    ["wizAdditionalNewPinConfirm", "wizSetAdditionalKeyPin"],
    ["wizCustomThreshold", "wizAddCustomComp"],
  ];

  for (const [id, expectedAction] of mappings) {
    harness.reset();
    const target = dom.el(id, "INPUT");
    let prevented = 0;
    equal(legacyEnterActionForId(id), expectedAction);
    equal(
      handleLegacyEnter(
        keyboardEvent("Enter", target, () => {
          prevented += 1;
        }),
        harness.actions,
      ),
      true,
    );
    equal(prevented, 1);
    equal(harness.counts[expectedAction], 1);
    equal(
      ACTION_NAMES.reduce((total, name) => total + harness.counts[name], 0),
      1,
    );
  }
});

test("legacy Enter yields to unknown targets, native forms, and active modals", async () => {
  const dom = installDom();
  const harness = actionHarness();
  const unknown = dom.el("unknown", "INPUT");
  let prevented = 0;

  equal(
    handleLegacyEnter(
      keyboardEvent("Enter", unknown, () => {
        prevented += 1;
      }),
      harness.actions,
    ),
    false,
  );
  equal(
    handleLegacyEnter(
      keyboardEvent("Escape", dom.el("passphrase", "INPUT"), () => {
        prevented += 1;
      }),
      harness.actions,
    ),
    false,
  );

  const repeatedEnter = keyboardEvent(
    "Enter",
    dom.el("passphrase", "INPUT"),
    () => {
      prevented += 1;
    },
  );
  Object.assign(repeatedEnter, { repeat: true });
  equal(handleLegacyEnter(repeatedEnter, harness.actions), false);

  const composingEnter = keyboardEvent(
    "Enter",
    dom.el("passphrase", "INPUT"),
    () => {
      prevented += 1;
    },
  );
  Object.assign(composingEnter, { isComposing: true });
  equal(handleLegacyEnter(composingEnter, harness.actions), false);

  const formTarget = dom.el("fido2Pin", "INPUT");
  (formTarget as any).form = dom.document.createElement("form");
  equal(
    handleLegacyEnter(
      keyboardEvent("Enter", formTarget, () => {
        prevented += 1;
      }),
      harness.actions,
    ),
    false,
  );

  const pending = confirmDangerDialog({
    title: "Active modal",
    body: "The modal owns Enter while it is open.",
  });
  equal(
    handleLegacyEnter(
      keyboardEvent("Enter", dom.el("wizFido2Label", "INPUT"), () => {
        prevented += 1;
      }),
      harness.actions,
    ),
    false,
  );
  equal(prevented, 0);
  equal(
    ACTION_NAMES.reduce((total, name) => total + harness.counts[name], 0),
    0,
  );

  (document.body.querySelector("[data-confirm-cancel]") as HTMLButtonElement).click();
  equal(await pending, false);
});
