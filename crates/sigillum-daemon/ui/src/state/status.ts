import type { StatusResponse } from "../contracts";

export type UiMode = "loading" | "setup" | "locked" | "unlocked";

export interface StatusSnapshot {
  mode: UiMode;
  initialized: boolean;
  locked: boolean;
  unlockedCompartmentCount: number;
  activeCompartmentLabel: string | null;
}

export function deriveUiMode(status: StatusResponse | null): UiMode {
  if (!status) {
    return "loading";
  }
  if (!status.initialized) {
    return "setup";
  }
  return status.locked ? "locked" : "unlocked";
}

export function snapshotStatus(status: StatusResponse | null): StatusSnapshot {
  return {
    mode: deriveUiMode(status),
    initialized: status?.initialized ?? false,
    locked: status?.locked ?? true,
    unlockedCompartmentCount: status?.unlocked_compartments.length ?? 0,
    activeCompartmentLabel: status?.active_compartment?.compartment_label ?? null,
  };
}
