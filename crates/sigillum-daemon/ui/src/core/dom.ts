/**
 * core/dom.ts — element builder and keyed list rendering for the strict-typed
 * console core (plan task 4.1).
 *
 * Two primitives, no framework, no proxies:
 *
 * - {@link el} builds DOM with `textContent`-only data flow. Untrusted data
 *   NEVER goes through `innerHTML`; the `html` prop exists solely for markup
 *   that is already sanitized by the codebase's `esc()` helpers (the same
 *   rule the legacy renderer follows).
 *
 * - {@link renderList} patches a container's children by key — create, move,
 *   update, remove — instead of wholesale `innerHTML` replacement. Because
 *   unchanged rows keep their nodes, re-rendering a live list does not lose
 *   focus, selection, or in-progress input values (the two defects that
 *   motivated D-B).
 */

export type ElChild = Node | string | number | null | undefined | false;

export interface ElProps {
  /** Class name(s); maps to `className`. */
  class?: string;
  /** Text content (safe for untrusted data — uses `textContent`). */
  text?: string | number;
  /**
   * Trusted markup ONLY: every interpolated value must already be escaped
   * with `esc()`/`escAttr()`. Never pass raw API/user data.
   */
  html?: string;
  /** `data-*` attributes. */
  dataset?: Record<string, string>;
  /** Other attributes (`role`, `aria-*`, `type`, `href`, …). */
  attrs?: Record<string, string>;
  /** Event listeners, bound with `addEventListener`. */
  on?: {
    [E in keyof HTMLElementEventMap]?: (event: HTMLElementEventMap[E]) => void;
  };
}

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: ElProps | null,
  ...children: ElChild[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (props) {
    if (props.class !== undefined) node.className = props.class;
    if (props.text !== undefined) node.textContent = String(props.text);
    if (props.html !== undefined) node.innerHTML = props.html;
    if (props.dataset) {
      for (const [name, value] of Object.entries(props.dataset)) {
        node.dataset[name] = value;
      }
    }
    if (props.attrs) {
      for (const [name, value] of Object.entries(props.attrs)) {
        node.setAttribute(name, value);
      }
    }
    if (props.on) {
      for (const [type, listener] of Object.entries(props.on)) {
        if (listener) {
          node.addEventListener(type, listener as (event: Event) => void);
        }
      }
    }
  }
  appendChildren(node, children);
  return node;
}

function appendChildren(node: HTMLElement, children: ElChild[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (typeof child === "string" || typeof child === "number") {
      node.appendChild(document.createTextNode(String(child)));
    } else {
      node.appendChild(child);
    }
  }
}

/** Compute the stable identity of a list item. Keys must be unique per list. */
export type ListKeyFn<T> = (item: T) => string;

/**
 * Render (or patch) one row.
 *
 * `existing` is the node this same key produced on the previous render, or
 * `null` for a new key. Return `existing` after patching it in place — that
 * is what preserves focus and in-progress input state. Return a fresh node
 * only when the row's shape genuinely changed; it replaces the old one.
 */
export type ListRenderItem<T> = (item: T, existing: HTMLElement | null) => HTMLElement;

interface ListState {
  byKey: Map<string, HTMLElement>;
}

const listStates = new WeakMap<Element, ListState>();

/**
 * Patch `container`'s children so they match `items`, keyed by `keyFn`.
 *
 * Semantics:
 * - new key → `renderItem(item, null)`, node inserted at its final position
 * - kept key → `renderItem(item, existing)`, node moved if its index changed
 * - vanished key → node removed
 * - duplicate keys → the FIRST occurrence wins; later duplicates are skipped
 *   (list keys must be unique — a duplicate is a caller bug)
 *
 * Any non-list children the caller appended to the container itself are not
 * tracked: keep a dedicated container per list.
 */
export function renderList<T>(
  container: Element,
  items: readonly T[],
  keyFn: ListKeyFn<T>,
  renderItem: ListRenderItem<T>,
): void {
  const state: ListState = listStates.get(container) ?? {
    byKey: new Map<string, HTMLElement>(),
  };
  const nextByKey = new Map<string, HTMLElement>();

  for (let index = 0; index < items.length; index++) {
    const item = items[index];
    const key = keyFn(item);
    if (nextByKey.has(key)) continue; // duplicate key: first occurrence wins
    const existing = state.byKey.get(key) ?? null;
    const node = renderItem(item, existing);
    // A renderItem may return a fresh node for a kept key instead of patching
    // `existing` in place: the old row must not linger next to the new one.
    if (existing && existing !== node && existing.parentNode === container) {
      existing.remove();
    }
    nextByKey.set(key, node);
    // `children` counts only element nodes, so this index is stable across
    // moves within this container.
    const ref = container.children[index] ?? null;
    if (ref !== node) {
      container.insertBefore(node, ref);
    }
  }

  for (const [key, node] of Array.from(state.byKey)) {
    if (!nextByKey.has(key)) {
      node.remove();
    }
  }

  state.byKey = nextByKey;
  listStates.set(container, state);
}

/** Drop all keyed-list bookkeeping for a container (e.g. before a full reset). */
export function clearList(container: Element): void {
  const state = listStates.get(container);
  if (state) {
    for (const node of state.byKey.values()) {
      node.remove();
    }
  }
  listStates.delete(container);
}
