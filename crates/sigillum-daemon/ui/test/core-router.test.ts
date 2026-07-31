import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import {
  createLegacySectionAdapter,
  createRouter,
  formatHash,
  parseHash,
  type Destination,
  type DestinationController,
  type Route,
} from "../src/core/router";
import { fakeBridge, MemoryHashSource } from "./core-helpers";

test("parseHash accepts destination hashes and rejects junk", () => {
  deepEqual(parseHash(""), null);
  deepEqual(parseHash("#"), null);
  deepEqual(parseHash("#/"), null);
  deepEqual(parseHash("#/bogus"), null);
  deepEqual(parseHash("move"), null);
  deepEqual(parseHash("#/overview"), { destination: "overview", path: [] });
  deepEqual(parseHash("#/move/plan/abc-123"), {
    destination: "move",
    path: ["plan", "abc-123"],
  });
  equal(formatHash("move", "plan", "abc 123"), "#/move/plan/abc%20123");
  deepEqual(parseHash("#/move/plan/abc%20123"), {
    destination: "move",
    path: ["plan", "abc 123"],
  });
});

test("router resolves registered patterns into params and publishes changes", () => {
  const source = new MemoryHashSource();
  const routes: Route[] = [];
  const router = createRouter({
    source,
    onRoute: (route) => routes.push(route),
  });
  router.register("move", "plan/:id");

  router.start(); // empty hash → replaced with the fallback
  equal(source.hash, "#/overview");
  deepEqual(
    routes.map((route) => route.destination),
    ["overview"],
  );

  source.write("#/move/plan/abc-123"); // external change (back/forward)
  const route = routes[routes.length - 1];
  equal(route.destination, "move");
  deepEqual(route.path, ["plan", "abc-123"]);
  deepEqual(route.params, { id: "abc-123" });
  equal(route.hash, "#/move/plan/abc-123");

  // Unknown sub-path still parses (inert deep link) with empty params.
  source.write("#/move/unknown/thing");
  deepEqual(routes[routes.length - 1].params, {});

  // navigate() is a no-op when the hash already matches.
  const published = routes.length;
  router.navigate("#/move/unknown/thing");
  equal(routes.length, published);
});

test("adapter rule 5: boot replaces an invalid hash with the legacy section", () => {
  const source = new MemoryHashSource();
  const bridge = fakeBridge("receive");
  const routes: Route[] = [];
  const router = createRouter({
    source,
    fallback: () => (bridge.section as Destination) || "overview",
    onRoute: (route) => routes.push(route),
  });
  const adapter = createLegacySectionAdapter({
    router,
    bridge,
    onRoute: () => {},
  });
  void adapter;
  router.start();

  equal(source.hash, "#/receive");
  equal(routes[0].destination, "receive");
  deepEqual(bridge.selected, []); // already in sync — no legacy call
});

test("adapter syncs both directions without loops (rules 2+3+6)", () => {
  const source = new MemoryHashSource();
  const bridge = fakeBridge("overview");
  const routes: Route[] = [];
  let handler: (route: Route) => void = () => {};
  const router = createRouter({
    source,
    fallback: "overview",
    onRoute: (route) => handler(route),
  });
  const adapter = createLegacySectionAdapter({
    router,
    bridge,
    onRoute: (route) => routes.push(route),
  });
  handler = adapter.handleRoute;
  adapter.start();
  equal(source.hash, "#/overview");

  // Deep link arrives (user pastes a URL / back button): legacy follows.
  source.write("#/vault");
  deepEqual(bridge.selected, ["vault"]);
  equal(bridge.section, "vault");
  equal(routes[routes.length - 1].destination, "vault");

  // Legacy switcher reports its own change: hash follows, no echo back.
  bridge.selected = [];
  bridge.section = "portfolio"; // legacy updates itself first…
  adapter.notifyLegacySection("portfolio"); // …then reports (app.ts wiring)
  equal(source.hash, "#/portfolio");
  deepEqual(bridge.selected, []); // no selectSection echo (loop safety)
});

test("adapter preserves controller-owned sub-paths and mounts migrated destinations", () => {
  const source = new MemoryHashSource();
  source.hash = "#/move/plan/abc-123";
  const bridge = fakeBridge("move");
  const mounted: Route[] = [];
  let unmounted = 0;
  const moveController: DestinationController = {
    id: "move",
    migrated: true,
    mount: (route) => mounted.push(route),
    unmount: () => {
      unmounted += 1;
    },
  };
  let handler: (route: Route) => void = () => {};
  const router = createRouter({
    source,
    fallback: "overview",
    onRoute: (route) => handler(route),
  });
  router.register("move", "plan/:id");
  const adapter = createLegacySectionAdapter({
    router,
    bridge,
    destinations: [moveController],
    onRoute: () => {},
  });
  handler = adapter.handleRoute;
  adapter.start();

  // Boot deep link mounts the migrated controller with parsed params.
  equal(mounted.length, 1);
  deepEqual(mounted[0].params, { id: "abc-123" });

  // Rule 1/3: legacy reports the SAME destination — sub-path untouched.
  adapter.notifyLegacySection("move");
  equal(source.hash, "#/move/plan/abc-123");

  // Leaving the destination unmounts it; legacy chrome still follows.
  source.write("#/vault");
  equal(unmounted, 1);
  deepEqual(bridge.selected, ["vault"]);
});
