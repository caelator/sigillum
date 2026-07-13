import { daemonHttpStatus } from "./session";
import {
  canonicalSessionToken,
  sessionEstablishmentToken,
} from "./sessionEstablishment";
import type { SessionRequestContext } from "./sessionCoordinator";

export interface SessionFinalizerDeps {
  readToken: () => string | null;
  writeToken: (token: string) => void;
  clearToken: () => void;
  privacyGeneration: () => number;
  boundaryGeneration: () => number;
  canAdoptSession: () => boolean;
  beforeSessionAdoption: () => void;
  isOwnerGenerationCurrent: (
    context: SessionRequestContext | null,
  ) => boolean;
  containAmbiguousOutcome: (
    path: string,
    capturedToken: string | null,
    stillCurrent: () => boolean,
  ) => Promise<never>;
}

export function createSessionFinalizers(deps: SessionFinalizerDeps) {
  async function finalizeSwitch(
    payload: any,
    body: any,
    context: SessionRequestContext | null,
    capturedToken: string | null,
  ): Promise<any> {
    const replacement = canonicalSessionToken(payload);
    const httpStatus = daemonHttpStatus(payload);
    const valid =
      httpStatus != null && httpStatus >= 200 && httpStatus < 300 &&
      !Object.prototype.hasOwnProperty.call(payload || {}, "error") &&
      payload?.status === "switched" &&
      Number.isInteger(payload?.compartment_id) &&
      payload.compartment_id === body?.id &&
      replacement != null && replacement !== capturedToken;
    if (
      valid && deps.canAdoptSession() &&
      deps.isOwnerGenerationCurrent(context) &&
      deps.readToken() === capturedToken
    ) {
      deps.writeToken(replacement);
      if (
        deps.canAdoptSession() && deps.readToken() === replacement &&
        deps.isOwnerGenerationCurrent(context)
      ) return payload;
      if (deps.readToken() === replacement) deps.clearToken();
    }
    return deps.containAmbiguousOutcome(
      "/api/compartment/switch", capturedToken,
      () => deps.isOwnerGenerationCurrent(context),
    );
  }

  async function finalizeLock(
    payload: any,
    context: SessionRequestContext | null,
    capturedToken: string | null,
  ): Promise<any> {
    const httpStatus = daemonHttpStatus(payload);
    const successfulHttp =
      httpStatus != null && httpStatus >= 200 && httpStatus < 300;
    if (
      deps.isOwnerGenerationCurrent(context) &&
      deps.readToken() === capturedToken &&
      (httpStatus === 423 ||
        (successfulHttp && !Object.prototype.hasOwnProperty.call(
          payload || {}, "error",
        ) && payload?.status === "locked"))
    ) {
      deps.clearToken();
      if (deps.readToken() == null && deps.isOwnerGenerationCurrent(context)) {
        return httpStatus === 423 ? { status: "locked" } : payload;
      }
    }
    return deps.containAmbiguousOutcome(
      "/api/lock", capturedToken,
      () => deps.isOwnerGenerationCurrent(context),
    );
  }

  async function finalizeEstablishment(
    method: string,
    path: string,
    body: any,
    payload: any,
    capturedToken: string | null,
    privacyGeneration: number,
    boundaryGeneration: number,
  ): Promise<any> {
    const generationCurrent = () =>
      deps.privacyGeneration() === privacyGeneration &&
      deps.boundaryGeneration() === boundaryGeneration;
    const predecessorCurrent = () =>
      generationCurrent() && deps.readToken() === capturedToken;
    const replacement = sessionEstablishmentToken(method, path, body, payload);
    let adoptionCurrent = predecessorCurrent() && deps.canAdoptSession();
    if (replacement && adoptionCurrent) {
      // A refresh started under the predecessor session must not keep the
      // newly authenticated UI joined to its single-flight. Cancel those
      // reads synchronously before T2 becomes observable.
      deps.beforeSessionAdoption();
      adoptionCurrent = predecessorCurrent() && deps.canAdoptSession();
      if (adoptionCurrent) {
        deps.writeToken(replacement);
        if (deps.canAdoptSession() && generationCurrent() &&
          deps.readToken() === replacement) return payload;
        if (deps.readToken() === replacement) deps.clearToken();
      }
    }
    return deps.containAmbiguousOutcome(
      path, capturedToken,
      adoptionCurrent ? generationCurrent : () => false,
    );
  }

  return { finalizeEstablishment, finalizeLock, finalizeSwitch };
}
