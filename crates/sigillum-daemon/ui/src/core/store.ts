/**
 * core/store.ts — tiny observable store for the strict-typed console core
 * (plan task 4.1; decision D-B option A: zero runtime dependencies).
 *
 * State is organized as named per-resource slices (`status`, `operations`,
 * `route`, …). A slice is an immutable-by-convention value: writers always
 * REPLACE it via {@link Store.set}/{@link Store.update} and never mutate it
 * in place. Change detection is reference equality per slice — reusing the
 * previous reference is the structural-sharing fast path and fires no
 * notification. Listeners subscribe to ONE slice and are only called when
 * that slice's reference actually changed.
 *
 * Notifications are batched on a microtask: N `set()` calls in one turn
 * produce at most one listener invocation per slice, with `next` = latest
 * value and `prev` = the value at the last notification. Renderers should
 * therefore always read the freshest state from the arguments (or `get()`),
 * never from a closure.
 */

export type Unsubscribe = () => void;
export type SliceListener<T> = (next: T, prev: T) => void;

export interface Store<Slices extends object> {
  /** Current value of a slice. */
  get<K extends keyof Slices & string>(key: K): Slices[K];
  /**
   * Replace a slice. No-op (no notification) when `next` is the same
   * reference as the current value.
   */
  set<K extends keyof Slices & string>(key: K, next: Slices[K]): void;
  /**
   * Replace a slice via a pure updater. Returning `prev` unchanged is the
   * idiomatic "nothing changed" fast path.
   */
  update<K extends keyof Slices & string>(
    key: K,
    fn: (prev: Slices[K]) => Slices[K],
  ): void;
  /**
   * Subscribe to one slice. The listener fires (batched, microtask) only
   * when the slice's reference changed since the last notification.
   */
  subscribe<K extends keyof Slices & string>(
    key: K,
    listener: SliceListener<Slices[K]>,
  ): Unsubscribe;
}

export function createStore<Slices extends object>(
  initial: Slices,
): Store<Slices> {
  type Key = keyof Slices & string;

  const slices: Slices = { ...initial };
  const listeners = new Map<Key, Set<SliceListener<Slices[Key]>>>();
  // Value each slice held at its last notification — the `prev` baseline
  // for the next batched flush.
  const notified = new Map<Key, Slices[Key]>();
  const dirty = new Set<Key>();
  let flushScheduled = false;

  for (const key of Object.keys(initial) as Key[]) {
    notified.set(key, initial[key]);
  }

  function flush(): void {
    flushScheduled = false;
    const keys = Array.from(dirty);
    dirty.clear();
    for (const key of keys) {
      const next = slices[key];
      const prev = notified.get(key) as Slices[typeof key];
      if (Object.is(next, prev)) continue;
      notified.set(key, next);
      const subs = listeners.get(key);
      if (!subs) continue;
      for (const listener of Array.from(subs)) {
        listener(next, prev as Slices[Key]);
      }
    }
  }

  function scheduleFlush(): void {
    if (flushScheduled) return;
    flushScheduled = true;
    queueMicrotask(flush);
  }

  const api: Store<Slices> = {
    get(key) {
      return slices[key];
    },
    set(key, next) {
      if (Object.is(slices[key], next)) return;
      slices[key] = next;
      dirty.add(key);
      scheduleFlush();
    },
    update(key, fn) {
      api.set(key, fn(slices[key]));
    },
    subscribe(key, listener) {
      const subs =
        listeners.get(key) ??
        (() => {
          const created = new Set<SliceListener<Slices[Key]>>();
          listeners.set(key, created);
          return created;
        })();
      subs.add(listener as SliceListener<Slices[Key]>);
      return () => {
        subs.delete(listener as SliceListener<Slices[Key]>);
      };
    },
  };

  return api;
}
