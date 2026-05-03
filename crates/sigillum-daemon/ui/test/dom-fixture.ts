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

export class FakeElement {
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

  constructor(readonly tagName = "DIV") {}

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

  focus(): void {}

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

  addEventListener(): void {}

  closest(): FakeElement | null {
    return this;
  }

  querySelector(): FakeElement | null {
    return null;
  }
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

class FakeDocument {
  readonly body = new FakeElement("BODY") as FakeElement & {
    dataset: Record<string, string>;
  };
  private readonly elements = new Map<string, FakeElement>();

  createElement(tagName: string): FakeElement {
    return new FakeElement(tagName.toUpperCase());
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
    const element = new FakeElement(tagName.toUpperCase());
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
