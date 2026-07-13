import { equal } from "node:assert/strict";
import { test } from "node:test";

import { createSessionPrivacyGuard } from "../src/views/sessionPrivacy";
import { installDom } from "./dom-fixture";

test("session privacy restores static cards and scrubs sensitive controls", () => {
  const dom = installDom([
    "operatorCard",
    "passwordField",
    "fileField",
    "walletImportSeedMnemonic",
    "seedMnemonic",
    "secretReveal",
    "toastStack",
    "compSwitcher",
    "compartmentBadge",
  ]);
  const card = dom.el("operatorCard");
  card.innerHTML = "<p>safe static shell</p>";
  const password = dom.el("passwordField");
  password.value = "private-password";
  const file = dom.el("fileField");
  file.value = "/tmp/private-snapshot";
  const importMnemonic = dom.el("walletImportSeedMnemonic");
  importMnemonic.value = "twelve private words";
  const seedMnemonic = dom.el("seedMnemonic");
  seedMnemonic.value = "another private phrase";
  const reveal = dom.el("secretReveal");
  dom.el("toastStack").innerHTML = "private toast";
  dom.el("compSwitcher").innerHTML = "private compartments";
  dom.el("compartmentBadge").textContent = "private compartment";

  (dom.document as any).querySelectorAll = (selector: string) => {
    if (selector.includes('input[type="password"]')) return [password, file];
    if (selector === ".secret-value") return [reveal];
    return [];
  };

  let resets = 0;
  let enhancements = 0;
  const guard = createSessionPrivacyGuard({
    cardIds: ["operatorCard"],
    resetters: [() => { resets += 1; }],
    enhanceRestoredUi: () => { enhancements += 1; },
    document: dom.document as unknown as Document,
  });

  card.innerHTML = "<p>rendered private state</p>";
  guard.scrub();

  equal(card.innerHTML, "<p>safe static shell</p>");
  equal(password.value, "");
  equal(file.value, "");
  equal(importMnemonic.value, "");
  equal(seedMnemonic.value, "");
  equal(reveal.isConnected, false);
  equal(dom.el("toastStack").innerHTML, "");
  equal(dom.el("compSwitcher").innerHTML, "");
  equal(dom.el("compartmentBadge").textContent, "");
  equal(resets, 1);
  equal(enhancements, 1);
  equal(guard.generation(), 1);
});

test("one broken resetter cannot abort the private DOM scrub", () => {
  const dom = installDom(["operatorCard"]);
  const card = dom.el("operatorCard");
  card.innerHTML = "safe shell";
  let laterResetRan = false;
  const reported: unknown[] = [];
  const guard = createSessionPrivacyGuard({
    cardIds: ["operatorCard"],
    resetters: [
      () => { throw new Error("broken resetter"); },
      () => { laterResetRan = true; },
    ],
    enhanceRestoredUi: () => undefined,
    reportResetError: (error) => reported.push(error),
    document: dom.document as unknown as Document,
  });

  card.innerHTML = "private rendered state";
  guard.scrub();

  equal(card.innerHTML, "safe shell");
  equal(laterResetRan, true);
  equal(reported.length, 1);
  equal(guard.generation(), 1);
});
