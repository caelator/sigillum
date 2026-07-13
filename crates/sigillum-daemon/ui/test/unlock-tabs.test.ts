import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import { applyUnlockTabKeydown } from "../src/views/fido2";

function createTab(mode: string, focused: string[]) {
  return {
    dataset: { arg0: mode },
    focus: () => focused.push(mode),
  } as unknown as HTMLElement;
}

function keyEvent(key: string, target: HTMLElement) {
  return {
    key,
    target,
    defaultPrevented: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
  };
}

test("unlock tabs activate and focus with ArrowLeft and ArrowRight", () => {
  const focused: string[] = [];
  const activated: string[] = [];
  const tabs = [createTab("passphrase", focused), createTab("fido2", focused)];

  const right = keyEvent("ArrowRight", tabs[0]);
  equal(applyUnlockTabKeydown(right, tabs, (tab) => activated.push(tab)), true);
  equal(right.defaultPrevented, true);

  const wrapRight = keyEvent("ArrowRight", tabs[1]);
  equal(applyUnlockTabKeydown(wrapRight, tabs, (tab) => activated.push(tab)), true);

  const wrapLeft = keyEvent("ArrowLeft", tabs[0]);
  equal(applyUnlockTabKeydown(wrapLeft, tabs, (tab) => activated.push(tab)), true);

  deepEqual(activated, ["fido2", "passphrase", "fido2"]);
  deepEqual(focused, ["fido2", "passphrase", "fido2"]);
});

test("unlock tabs support Home and End without consuming unrelated keys", () => {
  const focused: string[] = [];
  const activated: string[] = [];
  const tabs = [createTab("passphrase", focused), createTab("fido2", focused)];

  const end = keyEvent("End", tabs[0]);
  equal(applyUnlockTabKeydown(end, tabs, (tab) => activated.push(tab)), true);
  equal(end.defaultPrevented, true);

  const home = keyEvent("Home", tabs[1]);
  equal(applyUnlockTabKeydown(home, tabs, (tab) => activated.push(tab)), true);

  const tab = keyEvent("Tab", tabs[0]);
  equal(applyUnlockTabKeydown(tab, tabs, (mode) => activated.push(mode)), false);
  equal(tab.defaultPrevented, false);

  const outside = createTab("outside", focused);
  const outsideArrow = keyEvent("ArrowRight", outside);
  equal(
    applyUnlockTabKeydown(outsideArrow, tabs, (mode) => activated.push(mode)),
    false,
  );

  deepEqual(activated, ["fido2", "passphrase"]);
  deepEqual(focused, ["fido2", "passphrase"]);
});
