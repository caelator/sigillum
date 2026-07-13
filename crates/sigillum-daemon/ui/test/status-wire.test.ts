import { deepEqual, equal } from "node:assert/strict";
import { test } from "node:test";

import type { StatusResponse } from "../src/contracts";
import {
  clearStaleTokenForLockedMode,
  deriveUiMode,
  snapshotStatus,
} from "../src/state/status";

const unlockedStatusWire = {
  locked: false,
  initialized: true,
  active_compartment: {
    compartment_id: 7,
    compartment_label: "ops-treasury",
    api_key_count: 3,
    secret_count: 14,
  },
  unlocked_compartments: [
    {
      id: 7,
      label: "ops-treasury",
      threshold: 2,
      passphrase_mode: "fixed",
    },
  ],
  fido2: {
    enabled: true,
    key_count: 2,
  },
} satisfies StatusResponse;

test("status snapshot follows the daemon JSON wire shape", () => {
  const status = JSON.parse(JSON.stringify(unlockedStatusWire)) as StatusResponse;

  deepEqual(snapshotStatus(status), {
    mode: "unlocked",
    initialized: true,
    locked: false,
    unlockedCompartmentCount: 1,
    activeCompartmentLabel: "ops-treasury",
  });

  equal(status.active_compartment?.compartment_id, 7);
  equal(status.unlocked_compartments[0]?.id, 7);
  equal(status.unlocked_compartments[0]?.label, "ops-treasury");
});

test("UI mode derivation covers lifecycle status responses", () => {
  const setupStatus = {
    initialized: false,
    locked: true,
    unlocked_compartments: [],
  } satisfies StatusResponse;
  const lockedStatus = {
    initialized: true,
    locked: true,
    unlocked_compartments: [],
  } satisfies StatusResponse;

  equal(deriveUiMode(null), "loading");
  equal(deriveUiMode(setupStatus), "setup");
  equal(deriveUiMode(lockedStatus), "locked");
  equal(deriveUiMode(unlockedStatusWire), "unlocked");
  equal(snapshotStatus(lockedStatus).activeCompartmentLabel, null);
});

test("locked status clears a stale browser token fail-closed", () => {
  let clearCount = 0;
  const clear = () => {
    clearCount += 1;
  };

  equal(clearStaleTokenForLockedMode("locked", "stale-token", clear), true);
  equal(clearCount, 1);
  equal(clearStaleTokenForLockedMode("locked", null, clear), false);
  equal(clearStaleTokenForLockedMode("unlocked", "current-token", clear), false);
  equal(clearCount, 1);
});
