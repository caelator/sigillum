import { equal } from "node:assert/strict";
import { test } from "node:test";

import { enhanceHelpTips } from "../src/views/helpTips";

type Listener = (event: any) => void;

class FakeEventTarget {
  private readonly listeners = new Map<string, Set<Listener>>();

  addEventListener(type: string, listener: Listener): void {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)?.add(listener);
  }

  dispatch(type: string, event: any = {}): any {
    if (!("target" in event)) event.target = this;
    Array.from(this.listeners.get(type) || []).forEach((listener) => listener(event));
    return event;
  }
}

class FakeElement extends FakeEventTarget {
  readonly attributes = new Map<string, string>();
  readonly children: FakeElement[] = [];
  readonly dataset: Record<string, string> = {};
  className = "";
  id = "";
  parentNode: FakeElement | null = null;
  textContent = "";

  constructor(readonly ownerDocument: FakeDocument) {
    super();
  }

  appendChild(child: FakeElement): FakeElement {
    child.parentNode?.removeChild(child);
    child.parentNode = this;
    this.children.push(child);
    return child;
  }

  insertBefore(child: FakeElement, reference: FakeElement): FakeElement {
    child.parentNode?.removeChild(child);
    const index = this.children.indexOf(reference);
    child.parentNode = this;
    this.children.splice(index < 0 ? this.children.length : index, 0, child);
    return child;
  }

  removeChild(child: FakeElement): void {
    const index = this.children.indexOf(child);
    if (index >= 0) this.children.splice(index, 1);
    child.parentNode = null;
  }

  focus(): void {
    this.dispatch("focus");
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
    if (name.startsWith("data-")) {
      const key = name
        .slice(5)
        .replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase());
      this.dataset[key] = value;
    }
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.attributes.has(name);
  }
}

class FakeDocument extends FakeEventTarget {
  readonly elements: FakeElement[] = [];
  readonly triggers: FakeElement[] = [];

  createElement(): FakeElement {
    const element = new FakeElement(this);
    this.elements.push(element);
    return element;
  }

  createHelpTip(content: string): FakeElement {
    const container = this.createElement();
    const trigger = this.createElement();
    trigger.className = "help-tip";
    trigger.dataset.tip = content;
    trigger.setAttribute("data-tip", content);
    container.appendChild(trigger);
    this.triggers.push(trigger);
    return trigger;
  }

  getElementById(id: string): FakeElement | null {
    return this.elements.find((element) => element.id === id) ?? null;
  }

  querySelectorAll(selector: string): FakeElement[] {
    return selector === ".help-tip[data-tip]" ? this.triggers : [];
  }
}

function keyboardEvent(key: string): any {
  return {
    key,
    defaultPrevented: false,
    propagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
  };
}

test("help tips gain a real tooltip description and button semantics once", () => {
  const document = new FakeDocument();
  const trigger = document.createHelpTip("An isolated vault space.");

  enhanceHelpTips(document as unknown as ParentNode);

  equal(trigger.getAttribute("role"), "button");
  equal(trigger.getAttribute("tabindex"), "0");
  equal(trigger.getAttribute("aria-label"), "More information");
  equal(trigger.getAttribute("aria-expanded"), "false");
  equal(trigger.getAttribute("data-help-tip-enhanced"), "true");
  const tooltip = document.getElementById(trigger.getAttribute("aria-describedby") || "");
  equal(tooltip?.getAttribute("role"), "tooltip");
  equal(tooltip?.textContent, "An isolated vault space.");
  equal(tooltip?.parentNode, trigger.parentNode);
  equal(trigger.children.length, 0);

  enhanceHelpTips(document as unknown as ParentNode);
  equal(trigger.parentNode?.children.length, 2);
});

test("help tips stay open for hover or focus and Escape dismisses globally", () => {
  const document = new FakeDocument();
  const first = document.createHelpTip("First explanation");
  const second = document.createHelpTip("Second explanation");
  enhanceHelpTips(document as unknown as ParentNode);

  first.parentNode?.dispatch("mouseenter");
  equal(first.getAttribute("data-help-tip-open"), "true");
  equal(first.getAttribute("aria-expanded"), "true");

  const escape = keyboardEvent("Escape");
  document.dispatch("keydown", escape);
  equal(escape.defaultPrevented, true);
  equal(escape.propagationStopped, true);
  equal(first.getAttribute("data-help-tip-open"), "false");

  first.parentNode?.dispatch("mouseleave");
  first.dispatch("focus");
  equal(first.getAttribute("data-help-tip-open"), "true");

  second.parentNode?.dispatch("mouseenter");
  equal(first.getAttribute("data-help-tip-open"), "false");
  equal(second.getAttribute("data-help-tip-open"), "true");

  second.parentNode?.dispatch("mouseleave");
  equal(second.getAttribute("data-help-tip-open"), "false");
});

test("Enter and Space reopen a dismissed focused help tip", () => {
  const document = new FakeDocument();
  const trigger = document.createHelpTip("Keyboard explanation");
  enhanceHelpTips(document as unknown as ParentNode);
  trigger.dispatch("focus");

  document.dispatch("keydown", keyboardEvent("Escape"));
  equal(trigger.getAttribute("data-help-tip-open"), "false");

  const enter = keyboardEvent("Enter");
  trigger.dispatch("keydown", enter);
  equal(enter.defaultPrevented, true);
  equal(trigger.getAttribute("data-help-tip-open"), "true");

  document.dispatch("keydown", keyboardEvent("Escape"));
  const space = keyboardEvent(" ");
  trigger.dispatch("keydown", space);
  equal(space.defaultPrevented, true);
  equal(trigger.getAttribute("data-help-tip-open"), "true");
});
