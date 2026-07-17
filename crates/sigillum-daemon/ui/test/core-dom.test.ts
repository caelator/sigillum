import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import { clearList, el, renderList } from "../src/core/dom";
import { installDom, FakeElement } from "./dom-fixture";

test("el builds elements with safe textContent-only data flow", () => {
  installDom();
  let clicks = 0;
  const node = el(
    "div",
    {
      class: "attention-item",
      text: "<b>not markup</b>",
      dataset: { tier: "review" },
      attrs: { role: "alert", "aria-live": "polite" },
      on: { click: () => (clicks += 1) },
    },
    "tail",
    el("span", { text: "child" }),
    null,
    false,
  );

  equal(node.className, "attention-item");
  equal(node.textContent, "<b>not markup</b>"); // text, never parsed as HTML
  ok(node.innerHTML.includes("&lt;b&gt;")); // escaped by the fixture setter
  equal(node.dataset.tier, "review");
  equal(node.getAttribute("role"), "alert");
  equal(node.childNodes.length, 2); // text node + span; null/false skipped
  equal((node.childNodes[0] as { textContent: string }).textContent, "tail");

  node.click();
  equal(clicks, 1);

  const trusted = el("div", { html: "<b>trusted</b>" });
  equal(trusted.innerHTML, "<b>trusted</b>");
});

test("renderList creates, updates, moves, and removes rows by key", () => {
  installDom();
  const container = document.createElement("div");
  const renderItem = (
    item: { id: string; label: string },
    existing: HTMLElement | null,
  ): HTMLElement => {
    if (existing) {
      existing.textContent = item.label;
      return existing;
    }
    return el("div", { text: item.label });
  };

  renderList(
    container as unknown as Element,
    [
      { id: "a", label: "A" },
      { id: "b", label: "B" },
    ],
    (item) => item.id,
    renderItem,
  );
  equal(container.children.length, 2);
  equal(container.children[0].textContent, "A");
  const nodeA: unknown = container.children[0];
  const nodeB: unknown = container.children[1];

  // Update + reorder: node identity must be preserved (no re-creation).
  renderList(
    container as unknown as Element,
    [
      { id: "b", label: "B2" },
      { id: "a", label: "A2" },
      { id: "c", label: "C" },
    ],
    (item) => item.id,
    renderItem,
  );
  equal(container.children.length, 3);
  ok(container.children[0] === nodeB);
  ok(container.children[1] === nodeA);
  equal(container.children[0].textContent, "B2");
  equal(container.children[1].textContent, "A2");
  equal(container.children[2].textContent, "C");

  // Removal: vanished keys drop their nodes.
  renderList(
    container as unknown as Element,
    [{ id: "c", label: "C" }],
    (item) => item.id,
    renderItem,
  );
  equal(container.children.length, 1);
  equal(container.children[0].textContent, "C");
  equal((nodeA as FakeElement).isConnected, false);
  equal((nodeB as FakeElement).isConnected, false);
});

test("renderList drops the old row when renderItem returns a fresh node for a kept key", () => {
  installDom();
  const container = document.createElement("div");
  // A signature-style renderer: rebuilds the row whenever `label` changes,
  // returning a fresh node instead of patching `existing`.
  const signatures = new Map<string, string>();
  const renderItem = (
    item: { id: string; label: string },
    existing: HTMLElement | null,
  ): HTMLElement => {
    if (existing && signatures.get(item.id) === item.label) return existing;
    signatures.set(item.id, item.label);
    return el("div", { text: item.label });
  };

  renderList(container as unknown as Element, [{ id: "a", label: "A" }], (i) => i.id, renderItem);
  equal(container.children.length, 1);
  const first: unknown = container.children[0];

  renderList(container as unknown as Element, [{ id: "a", label: "A2" }], (i) => i.id, renderItem);
  equal(container.children.length, 1, "no zombie row next to the fresh node");
  equal(container.children[0].textContent, "A2");
  ok(container.children[0] !== first, "row was rebuilt");
  equal((first as FakeElement).isConnected, false, "old node detached");

  // Unchanged signature keeps the node (focus-preservation contract).
  const second: unknown = container.children[0];
  renderList(container as unknown as Element, [{ id: "a", label: "A2" }], (i) => i.id, renderItem);
  ok(container.children[0] === second, "unchanged row keeps its node");
});

test("renderList preserves focus and in-progress input across re-renders", () => {
  const dom = installDom();
  const container = document.createElement("div");
  let inputEl: FakeElement | null = null;

  const renderItem = (
    item: { id: string; label: string },
    existing: HTMLElement | null,
  ): HTMLElement => {
    if (existing) {
      // Patch only the label — the input node is untouched.
      existing.children[1].textContent = item.label;
      return existing;
    }
    const input = el("input");
    inputEl = input as unknown as FakeElement;
    return el("div", null, input, el("span", { text: item.label }));
  };

  renderList(
    container as unknown as Element,
    [{ id: "row", label: "before" }],
    (item) => item.id,
    renderItem,
  );
  ok(inputEl);
  const input = inputEl as unknown as FakeElement;
  input.value = "draft in progress";
  input.focus();
  ok(dom.document.activeElement === input);

  renderList(
    container as unknown as Element,
    [{ id: "row", label: "after" }],
    (item) => item.id,
    renderItem,
  );

  ok(dom.document.activeElement === input, "focus survives the patch");
  equal(input.value, "draft in progress");
  equal(container.children[0].children[1].textContent, "after");
});

test("renderList skips duplicate keys (first wins) and clearList resets", () => {
  installDom();
  const container = document.createElement("div");
  const renderItem = (
    item: { id: string; label: string },
    existing: HTMLElement | null,
  ): HTMLElement => existing ?? el("div", { text: item.label });

  renderList(
    container as unknown as Element,
    [
      { id: "x", label: "first" },
      { id: "x", label: "duplicate" },
    ],
    (item) => item.id,
    renderItem,
  );
  equal(container.children.length, 1);
  equal(container.children[0].textContent, "first");

  clearList(container as unknown as Element);
  equal(container.children.length, 0);
});
