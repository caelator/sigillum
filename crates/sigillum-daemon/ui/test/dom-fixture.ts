function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

class FakeClassList {
  private readonly values = new Set<string>();

  add(...names: string[]): void {
    names.forEach((name) => this.values.add(name));
  }

  remove(...names: string[]): void {
    names.forEach((name) => this.values.delete(name));
  }

  contains(name: string): boolean {
    return this.values.has(name);
  }

  toggle(name: string, force?: boolean): boolean {
    const next = force ?? !this.values.has(name);
    if (next) this.values.add(name);
    else this.values.delete(name);
    return next;
  }
}

export interface FakeEvent {
  type: string;
  target?: FakeElement | FakeDocument | null;
  key?: string;
  shiftKey?: boolean;
  preventDefault?: () => void;
  [key: string]: unknown;
}

type FakeListener = (event: FakeEvent) => void;

class FakeEventTarget {
  private readonly listeners = new Map<string, FakeListener[]>();

  addEventListener(type: string, listener: FakeListener): void {
    const registered = this.listeners.get(type) || [];
    registered.push(listener);
    this.listeners.set(type, registered);
  }

  removeEventListener(type: string, listener: FakeListener): void {
    const registered = this.listeners.get(type) || [];
    this.listeners.set(
      type,
      registered.filter((candidate) => candidate !== listener),
    );
  }

  dispatchEvent(event: FakeEvent): boolean {
    if (!event.target) event.target = this as unknown as FakeElement;
    (this.listeners.get(event.type) || []).slice().forEach((listener) => listener(event));
    return true;
  }
}

function matchesSelector(element: FakeElement, selector: string): boolean {
  const attribute = selector.match(/^\[([a-zA-Z0-9-]+)(?:="([^"]*)")?\]$/);
  if (!attribute) return false;
  const [, name, value] = attribute;
  if (!(name in element.attributes)) return false;
  return value === undefined || element.attributes[name] === value;
}

export class FakeElement extends FakeEventTarget {
  readonly dataset: Record<string, string> = {};
  readonly classList = new FakeClassList();
  readonly style: Record<string, string> = {};
  readonly children: FakeElement[] = [];
  readonly attributes: Record<string, string> = {};
  value = "";
  checked = false;
  disabled = false;
  selectedIndex = 0;
  isConnected = true;
  className = "";
  innerHTML = "";
  id = "";
  ownerDocument: FakeDocumentLike | null = null;

  constructor(readonly tagName = "DIV") {
    super();
  }

  private text = "";

  get textContent(): string {
    return this.text;
  }

  set textContent(value: string | null) {
    this.text = value ?? "";
    this.innerHTML = escapeHtml(this.text);
  }

  appendChild(child: FakeElement): FakeElement {
    this.children.push(child);
    return child;
  }

  remove(): void {
    this.isConnected = false;
  }

  focus(): void {
    if (this.ownerDocument) this.ownerDocument.activeElement = this;
  }

  click(): void {
    this.dispatchEvent({ type: "click", target: this });
  }

  scrollIntoView(): void {}

  setAttribute(name: string, value: string): void {
    this.attributes[name] = value;
  }

  getAttribute(name: string): string | null {
    return this.attributes[name] ?? null;
  }

  removeAttribute(name: string): void {
    delete this.attributes[name];
  }

  hasAttribute(name: string): boolean {
    return name in this.attributes;
  }

  closest(): FakeElement | null {
    return this;
  }

  querySelector(selector: string): FakeElement | null {
    for (const child of this.children) {
      if (matchesSelector(child, selector)) return child;
      const nested = child.querySelector(selector);
      if (nested) return nested;
    }
    return null;
  }
}

export interface FakeDocumentLike {
  activeElement: FakeElement | null;
}

class FakeStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

class FakeDocument extends FakeEventTarget implements FakeDocumentLike {
  readonly body = new FakeElement("BODY") as FakeElement & {
    dataset: Record<string, string>;
  };
  activeElement: FakeElement | null = null;
  private readonly elements = new Map<string, FakeElement>();

  constructor() {
    super();
    this.body.ownerDocument = this;
  }

  createElement(tagName: string): FakeElement {
    const element = new FakeElement(tagName.toUpperCase());
    element.ownerDocument = this;
    return element;
  }

  getElementById(id: string): FakeElement | null {
    return this.elements.get(id) ?? null;
  }

  querySelector(): FakeElement | null {
    return null;
  }

  querySelectorAll(): FakeElement[] {
    return [];
  }

  register(id: string, tagName = "DIV"): FakeElement {
    const element = this.createElement(tagName);
    element.id = id;
    this.elements.set(id, element);
    return element;
  }
}

export interface DomFixture {
  document: FakeDocument;
  sessionStorage: FakeStorage;
  el: (id: string, tagName?: string) => FakeElement;
}

export function installDom(ids: string[] = []): DomFixture {
  const document = new FakeDocument();
  const sessionStorage = new FakeStorage();
  const fixture: DomFixture = {
    document,
    sessionStorage,
    el: (id: string, tagName = "DIV") => {
      const existing = document.getElementById(id);
      if (existing) return existing;
      return document.register(id, tagName);
    },
  };
  ids.forEach((id) => fixture.el(id));
  (globalThis as any).document = document;
  (globalThis as any).window = {
    sessionStorage,
    isSecureContext: true,
    prompt: () => null,
  };
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { clipboard: { writeText: async () => undefined } },
  });
  return fixture;
}
