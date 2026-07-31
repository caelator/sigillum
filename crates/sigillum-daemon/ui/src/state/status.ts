import type { StatusResponse } from "../contracts";

export type UiMode = "loading" | "setup" | "locked" | "unlocked";

export function deriveUiMode(status: StatusResponse | null): UiMode {
  if (!status) {
    return "loading";
  }
  if (!status.initialized) {
    return "setup";
  }
  return status.locked ? "locked" : "unlocked";
}
