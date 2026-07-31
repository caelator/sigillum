import { FakeElement } from "./dom-fixture";
import type { HashSource } from "../src/core/router";
import type { EventSourceLike } from "../src/core/events";
import type { Operation, StatusResponse } from "../src/contracts";
import type { Route } from "../src/core/router";

export async function tick(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export class MemoryHashSource implements HashSource {
  hash = "";
  private readonly listeners: Array<() => void> = [];

  read(): string {
    return this.hash;
  }
  write(hash: string): void {
    this.hash = hash;
    for (const listener of this.listeners.slice()) listener();
  }
  replace(hash: string): void {
    this.hash = hash;
  }
  onChange(listener: () => void): () => void {
    this.listeners.push(listener);
    return () => {
      const index = this.listeners.indexOf(listener);
      if (index >= 0) this.listeners.splice(index, 1);
    };
  }
}

export class MockEventSource implements EventSourceLike {
  static instances: MockEventSource[] = [];

  readonly listeners = new Map<
    string,
    Array<(event: { data?: string }) => void>
  >();
  closed = false;

  constructor(readonly url: string) {
    MockEventSource.instances.push(this);
  }

  addEventListener(
    type: string,
    listener: (event: { data?: string }) => void,
  ): void {
    const registered = this.listeners.get(type) ?? [];
    registered.push(listener);
    this.listeners.set(type, registered);
  }

  close(): void {
    this.closed = true;
  }

  emit(type: string, data?: string): void {
    for (const listener of (this.listeners.get(type) ?? []).slice()) {
      listener({ data });
    }
  }
}

export function sampleOperation(id: string, state = "running"): Operation {
  return {
    id,
    kind: "inventory_scan_evm",
    state,
    progress: { processed: 1 },
    created_at_unix: 10,
    updated_at_unix: 12,
  } as Operation;
}

export function sampleStatus(locked = false): StatusResponse {
  return {
    initialized: true,
    locked,
    unlocked_compartments: locked
      ? []
      : [{ id: 0, label: "Main", threshold: 1 }],
  };
}

export const BOOT_ROUTE: Route = {
  destination: "overview",
  path: [],
  params: {},
  hash: "#/overview",
};

/** The legacy bridge test double: a sessionStorage-less section switcher. */
export function fakeBridge(initial = "overview") {
  return {
    section: initial,
    selected: [] as string[],
    readSection(): string {
      return this.section;
    },
    selectSection(id: string): void {
      this.selected.push(id);
      this.section = id;
    },
  };
}

export function mockFetchJson(handler: (path: string, init: unknown) => unknown): void {
  (globalThis as { fetch?: unknown }).fetch = async (
    path: string,
    init: unknown,
  ) => ({
    status: 200,
    json: async () => handler(path, init),
  });
}

export function asFakeElement(node: unknown): FakeElement {
  return node as FakeElement;
}
