import { equal, ok } from "node:assert/strict";
import { test } from "node:test";

import { createFido2Actions } from "../src/views/fido2";

type Listener = (event: any) => void;

class FakeEventTarget {
  private readonly listeners = new Map<string, Set<Listener>>();

  addEventListener(type: string, listener: Listener): void {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)?.add(listener);
  }

  removeEventListener(type: string, listener: Listener): void {
    this.listeners.get(type)?.delete(listener);
  }

  dispatch(type: string, event: any = {}): void {
    if (!("target" in event)) event.target = this;
    Array.from(this.listeners.get(type) || []).forEach((listener) => listener(event));
  }
}

class FakeElement extends FakeEventTarget {
  readonly children: FakeElement[] = [];
  readonly attributes = new Map<string, string>();
  readonly style = { cssText: "", overflow: "" };
  parent: FakeElement | null = null;
  id = "";
  value = "";
  disabled = false;
  isConnected = false;
  private html = "";

  constructor(
    readonly tagName: string,
    private readonly ownerDocument: FakeDocument,
  ) {
    super();
  }

  get innerHTML(): string {
    return this.html;
  }

  set innerHTML(value: string) {
    this.children.splice(0).forEach((child) => this.ownerDocument.unregisterTree(child));
    this.html = value;
    this.ownerDocument.parseRuntimeMarkup(this, value);
  }

  appendChild(child: FakeElement): FakeElement {
    child.parent = this;
    this.children.push(child);
    child.setConnected(this.isConnected);
    this.ownerDocument.registerTree(child);
    return child;
  }

  remove(): void {
    if (this.parent) {
      const index = this.parent.children.indexOf(this);
      if (index >= 0) this.parent.children.splice(index, 1);
    }
    this.ownerDocument.unregisterTree(this);
    this.setConnected(false);
    this.parent = null;
  }

  focus(): void {
    this.ownerDocument.activeElement = this;
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  removeAttribute(name: string): void {
    this.attributes.delete(name);
  }

  hasAttribute(name: string): boolean {
    return this.attributes.has(name);
  }

  setConnected(connected: boolean): void {
    this.isConnected = connected;
    this.children.forEach((child) => child.setConnected(connected));
  }
}

class FakeDocument extends FakeEventTarget {
  readonly body: FakeElement;
  activeElement: FakeElement | null = null;
  private readonly elements = new Map<string, FakeElement>();

  constructor() {
    super();
    this.body = new FakeElement("BODY", this);
    this.body.setConnected(true);
  }

  createElement(tagName: string): FakeElement {
    return new FakeElement(tagName.toUpperCase(), this);
  }

  getElementById(id: string): FakeElement | null {
    return this.elements.get(id) ?? null;
  }

  registerTree(element: FakeElement): void {
    if (element.id) this.elements.set(element.id, element);
    element.children.forEach((child) => this.registerTree(child));
  }

  unregisterTree(element: FakeElement): void {
    if (element.id && this.elements.get(element.id) === element) {
      this.elements.delete(element.id);
    }
    element.children.forEach((child) => this.unregisterTree(child));
  }

  parseRuntimeMarkup(parent: FakeElement, html: string): void {
    const openingTag = /<([a-z][a-z0-9]*)\b([^>]*)>/gi;
    let match: RegExpExecArray | null;
    while ((match = openingTag.exec(html)) != null) {
      const id = /\bid="([^"]+)"/.exec(match[2])?.[1];
      if (!id) continue;
      const element = this.createElement(match[1]);
      element.id = id;
      const attribute = /([a-zA-Z:-]+)="([^"]*)"/g;
      let attributeMatch: RegExpExecArray | null;
      while ((attributeMatch = attribute.exec(match[2])) != null) {
        element.setAttribute(attributeMatch[1], attributeMatch[2]);
      }
      parent.appendChild(element);
    }
  }
}

function keyboardEvent(key: string, shiftKey = false): any {
  return {
    key,
    shiftKey,
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

function installModalHarness() {
  const document = new FakeDocument();
  const shell = document.createElement("div");
  shell.id = "appShell";
  shell.setAttribute("aria-hidden", "false");
  document.body.appendChild(shell);
  const trigger = document.createElement("button");
  trigger.id = "removeKeyTrigger";
  shell.appendChild(trigger);
  trigger.focus();

  (globalThis as any).document = document;
  (globalThis as any).confirm = () => true;

  const requests: Array<{ method: string; path: string; body: any }> = [];
  const actions = createFido2Actions({
    api: async (method, path, body) => {
      requests.push({ method, path, body });
      return {};
    },
    toast: () => undefined,
    refresh: () => undefined,
    currentStatus: () => ({ locked: false }),
  });

  return { document, shell, trigger, requests, actions };
}

test("FIDO PIN prompt is modal, traps focus, and restores background state", async () => {
  const { document, shell, trigger, requests, actions } = installModalHarness();

  const removal = actions.fido2RemoveKey("backup");
  const overlay = document.body.children[document.body.children.length - 1];
  const input = document.getElementById("pinModalInput");
  const okButton = document.getElementById("pinModalOk");

  ok(overlay.innerHTML.includes('role="dialog"'));
  ok(overlay.innerHTML.includes('aria-modal="true"'));
  ok(overlay.innerHTML.includes('aria-labelledby="pinModalTitle"'));
  ok(document.getElementById("pinModalTitle"));
  equal(document.activeElement, input);
  equal(shell.hasAttribute("inert"), true);
  equal(shell.getAttribute("aria-hidden"), "true");
  equal(document.body.style.overflow, "hidden");

  okButton?.focus();
  const forwardTab = keyboardEvent("Tab");
  document.dispatch("keydown", forwardTab);
  equal(forwardTab.defaultPrevented, true);
  equal(document.activeElement, input);

  input?.focus();
  const reverseTab = keyboardEvent("Tab", true);
  document.dispatch("keydown", reverseTab);
  equal(reverseTab.defaultPrevented, true);
  equal(document.activeElement, okButton);

  const escape = keyboardEvent("Escape");
  document.dispatch("keydown", escape);
  await removal;

  equal(escape.defaultPrevented, true);
  equal(escape.propagationStopped, true);
  equal(document.getElementById("pinModalInput"), null);
  equal(shell.hasAttribute("inert"), false);
  equal(shell.getAttribute("aria-hidden"), "false");
  equal(document.body.style.overflow, "");
  equal(document.activeElement, trigger);
  equal(requests.length, 0);
});

test("FIDO PIN prompt keeps submit behavior and restores focus", async () => {
  const { document, trigger, requests, actions } = installModalHarness();

  const removal = actions.fido2RemoveKey("backup");
  const input = document.getElementById("pinModalInput");
  const okButton = document.getElementById("pinModalOk");
  if (input) input.value = "1234";
  okButton?.dispatch("click");
  await removal;

  equal(requests.length, 1);
  equal(requests[0].body.pin, "1234");
  equal(document.activeElement, trigger);
});

test("submitting a blank optional PIN still confirms key removal", async () => {
  const { document, requests, actions } = installModalHarness();

  const removal = actions.fido2RemoveKey("backup");
  document.getElementById("pinModalOk")?.dispatch("click");
  await removal;

  equal(requests.length, 1);
  equal(requests[0].path, "/api/fido2/remove");
  equal(requests[0].body.pin, undefined);
});

test("session reset dismisses a PIN prompt without resuming the old action", async () => {
  const { document, shell, trigger, requests, actions } = installModalHarness();

  const removal = actions.fido2RemoveKey("backup");
  ok(document.getElementById("pinModalInput"));

  actions.resetSession();
  await removal;

  equal(document.getElementById("pinModalInput"), null);
  equal(shell.hasAttribute("inert"), false);
  equal(shell.getAttribute("aria-hidden"), "false");
  equal(document.body.style.overflow, "");
  equal(document.activeElement, trigger);
  equal(requests.length, 0);
});
