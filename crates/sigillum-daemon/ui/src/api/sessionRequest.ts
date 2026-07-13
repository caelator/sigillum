import { DaemonHttpError, SessionContextChangedError, daemonHttpStatus,
  requestWithSession } from "./session";
import { isExplicitSessionRejection,
  isSessionEstablishingPath } from "./sessionEstablishment";
import { createSessionFinalizers } from "./sessionFinalizers";
import { isSessionBoundaryPath,
  isSessionBoundarySuccess } from "./sessionBoundary";
import type { SessionRequestContext } from "./sessionCoordinator";

const API_READ_TIMEOUT_MS = 30_000, API_MUTATION_TIMEOUT_MS = 120_000;
export interface SessionRequesterDeps {
  readToken: () => string | null; writeToken: (token: string) => void;
  clearToken: () => void; canAdoptSession: () => boolean;
  privacyGeneration: () => number; boundaryGeneration: () => number;
  closeSessionUi: (forcePrivateReset?: boolean) => void; isUnlockedUi: () => boolean;
  markRefreshQueued: () => void; handleDaemonBoundary: () => void;
  settleBoundary: (path: string) => void;
  requestAllowed: (
    path: string,
    context: SessionRequestContext | null,
  ) => boolean;
  isContextCurrent: (context: SessionRequestContext | null) => boolean;
  isOwnerGenerationCurrent: (
    context: SessionRequestContext | null,
  ) => boolean;
  containAmbiguousOutcome: (
    path: string,
    capturedToken: string | null,
    stillCurrent: () => boolean,
  ) => Promise<never>;
  request?: (
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ) => Promise<any>;
}

function timeoutError(sessionClosed: boolean): Error {
  return new Error(sessionClosed
    ? "The daemon request timed out. The browser session was closed because the final state is unknown."
    : "The daemon request timed out.");
}

export function createSessionRequester(deps: SessionRequesterDeps) {
  const request = deps.request || requestWithSession;
  const inFlightMutations = new Set<Promise<any>>();
  const inFlightReads = new Set<AbortController>();
  const finalizers = createSessionFinalizers({ ...deps, beforeSessionAdoption: abortReads });

  async function waitForMutationDrain(): Promise<void> {
    while (inFlightMutations.size > 0) {
      await Promise.allSettled(Array.from(inFlightMutations));
    }
  }

  function abortReads(): void {
    inFlightReads.forEach((controller) => controller.abort());
  }

  async function api(
    method: string,
    path: string,
    body?: unknown,
    expectedContext: SessionRequestContext | null = null,
  ): Promise<any> {
    if (!deps.requestAllowed(path, expectedContext)) {
      throw new SessionContextChangedError();
    }
    if (expectedContext && !deps.isContextCurrent(expectedContext)) {
      throw new SessionContextChangedError();
    }

    const tokenBeforeRequest = deps.readToken();
    const privacyGenerationAtStart = deps.privacyGeneration();
    const boundaryGenerationAtStart = deps.boundaryGeneration();
    const sessionEstablishment = isSessionEstablishingPath(path);
    const ownedBoundary = isSessionBoundaryPath(path) &&
      expectedContext?.transitionOwnerId != null;
    const generationCurrent = () =>
      deps.privacyGeneration() === privacyGenerationAtStart &&
      deps.boundaryGeneration() === boundaryGenerationAtStart;
    const predecessorCurrent = () =>
      generationCurrent() && deps.readToken() === tokenBeforeRequest;
    const containAmbiguity = (): Promise<never> => {
      if (!ownedBoundary && !predecessorCurrent()) {
        deps.markRefreshQueued();
        return Promise.reject(new SessionContextChangedError());
      }
      return deps.containAmbiguousOutcome(
        path, tokenBeforeRequest,
        ownedBoundary
          ? () => deps.isOwnerGenerationCurrent(expectedContext)
          : generationCurrent,
      );
    };
    const rejectDaemonBoundary = (): never => {
      if (predecessorCurrent()) {
        deps.handleDaemonBoundary();
        deps.settleBoundary(path);
      } else deps.markRefreshQueued();
      throw new SessionContextChangedError();
    };
    const rejectMalformedUnauthorized = (): never => {
      if (predecessorCurrent()) {
        if (tokenBeforeRequest) deps.clearToken();
        if (deps.isUnlockedUi()) deps.closeSessionUi(true);
      }
      deps.markRefreshQueued();
      throw new SessionContextChangedError();
    };
    const controller = new AbortController();
    const requestTimeoutMs = Number(
      expectedContext?.requestTimeoutMs ??
        (method === "GET" ? API_READ_TIMEOUT_MS : API_MUTATION_TIMEOUT_MS),
    );
    let requestTimedOut = false;
    const requestTimeout = setTimeout(() => {
      requestTimedOut = true;
      controller.abort();
    }, requestTimeoutMs);
    let pendingRequest: Promise<any>;
    try {
      pendingRequest = request(method, path, body, controller.signal);
    } catch (error) {
      pendingRequest = Promise.reject(error);
    }
    const tracksMutation = method !== "GET";
    if (tracksMutation) inFlightMutations.add(pendingRequest);
    else inFlightReads.add(controller);

    let payload: any;
    try {
      payload = await pendingRequest;
    } catch (error) {
      if (
        error instanceof DaemonHttpError && error.status === 401 &&
        !ownedBoundary
      ) {
        return rejectMalformedUnauthorized();
      }
      if (
        error instanceof DaemonHttpError && error.status === 423 &&
        path !== "/api/lock" && path !== "/api/compartment/switch"
      ) return rejectDaemonBoundary();
      if (requestTimedOut) {
        deps.markRefreshQueued();
        if (ownedBoundary || sessionEstablishment) {
          return containAmbiguity();
        }
        const sessionClosed = Boolean(
          deps.isContextCurrent(expectedContext) &&
            generationCurrent(),
        );
        if (sessionClosed) {
          deps.clearToken();
          deps.closeSessionUi(true);
        }
        throw timeoutError(sessionClosed);
      }
      if (controller.signal.aborted) {
        deps.markRefreshQueued();
        throw new SessionContextChangedError();
      }
      if (ownedBoundary || sessionEstablishment) return containAmbiguity();
      throw error;
    } finally {
      clearTimeout(requestTimeout);
      if (tracksMutation) inFlightMutations.delete(pendingRequest);
      else inFlightReads.delete(controller);
    }

    if (path === "/api/compartment/switch") {
      const result = await finalizers.finalizeSwitch(
        payload, body, expectedContext, tokenBeforeRequest,
      );
      deps.settleBoundary(path);
      return result;
    }
    if (path === "/api/lock" && expectedContext?.transitionOwnerId != null) {
      const result = await finalizers.finalizeLock(
        payload, expectedContext, tokenBeforeRequest,
      );
      deps.settleBoundary(path);
      return result;
    }
    const httpStatus = daemonHttpStatus(payload);
    if (
      httpStatus === 423 && path !== "/api/lock" &&
      path !== "/api/compartment/switch"
    ) return rejectDaemonBoundary();
    if (sessionEstablishment) {
      if (isExplicitSessionRejection(payload)) {
        if (!predecessorCurrent()) {
          deps.markRefreshQueued();
          throw new SessionContextChangedError();
        }
        if (httpStatus === 401) {
          if (tokenBeforeRequest) deps.clearToken();
          if (deps.isUnlockedUi()) deps.closeSessionUi(true);
          deps.markRefreshQueued();
        }
        return payload;
      }
      return finalizers.finalizeEstablishment(
        method, path, body, payload, tokenBeforeRequest,
        privacyGenerationAtStart, boundaryGenerationAtStart,
      );
    }
    if (httpStatus === 401) {
      if (!predecessorCurrent()) {
        deps.markRefreshQueued();
        throw new SessionContextChangedError();
      }
      if (tokenBeforeRequest) deps.clearToken();
      if (deps.isUnlockedUi()) deps.closeSessionUi(true);
      deps.settleBoundary(path);
      deps.markRefreshQueued();
      throw new SessionContextChangedError();
    }
    if (!predecessorCurrent()) {
      deps.markRefreshQueued();
      throw new SessionContextChangedError();
    }
    const explicitRejection = httpStatus != null &&
      httpStatus >= 400 && httpStatus < 500;
    if (isSessionBoundarySuccess(path, payload)) {
      deps.clearToken();
      if (deps.readToken() != null) return containAmbiguity();
      deps.settleBoundary(path);
    } else if (explicitRejection) {
      deps.settleBoundary(path);
    } else if (ownedBoundary) {
      return containAmbiguity();
    }
    return payload;
  }

  return { abortReads, api, waitForMutationDrain };
}
