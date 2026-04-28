import type { ActiveCompartment, StatusResponse } from "../contracts";
import { deriveUiMode } from "../state/status";

export interface SetupRequirement {
  id: "vault" | "unlock" | "compartment";
  complete: boolean;
}

export function setupRequirements(status: StatusResponse | null): SetupRequirement[] {
  return [
    {
      id: "vault",
      complete: deriveUiMode(status) !== "setup",
    },
    {
      id: "unlock",
      complete: deriveUiMode(status) === "unlocked",
    },
    {
      id: "compartment",
      complete: Boolean(status?.active_compartment),
    },
  ];
}

export function compartmentLabel(compartment: ActiveCompartment | null | undefined): string {
  return compartment?.label ?? "No active compartment";
}
