import { LockLatchStorage, LockLatchSubscriber, createLockUnconfirmedState } from "./lockUnconfirmed";
import { SessionContextChangedError, clearSessionToken, readSessionToken,
  requestFailClosedLockWithToken, writeSessionToken } from "./session";
import { SessionBoundaryChannel, createSessionBoundaryState,
  isSessionBoundaryPath } from "./sessionBoundary";
import { createSessionRequester } from "./sessionRequest";
const SESSION_TRANSITION_DRAIN_TIMEOUT_MS = 30_000, API_MUTATION_TIMEOUT_MS = 120_000;
export interface SessionRequestContext {
  token: string | null; privacyGeneration: number; boundaryGeneration: number;
  transitionOwnerId?: number; requestTimeoutMs?: number;
}

interface SessionTransitionOwner { id: number; path: string; }

interface SessionCoordinatorDeps {
  privacyGeneration: () => number;
  scrubPrivateWorkspace: () => void;
  resetPrivateState: () => void;
  setTransitionUi: (active: boolean) => void;
  renderTransitionState: (label: string) => void;
  closeSessionUi: (forcePrivateReset?: boolean) => void;
  showLockUnconfirmed: (canRetry: boolean, canAcknowledge: boolean) => void;
  clearLockUnconfirmed: () => void;
  isUnlockedUi: () => boolean;
  markRefreshQueued: () => void;
  refresh: () => Promise<unknown>;
  readToken?: () => string | null; writeToken?: (token: string) => void;
  clearToken?: () => void;
  failClosedLock?: (token: string) => Promise<boolean>;
  lockStorage?: LockLatchStorage | null; lockSubscribe?: LockLatchSubscriber;
  boundaryChannel?: SessionBoundaryChannel;
  request?: (method: string, path: string, body?: unknown,
    signal?: AbortSignal) => Promise<any>;
}

export function createSessionCoordinator(deps: SessionCoordinatorDeps) {
  const readToken = deps.readToken || readSessionToken;
  const writeToken = deps.writeToken || writeSessionToken;
  const clearToken = deps.clearToken || clearSessionToken;
  let transitionOwner: SessionTransitionOwner | null = null;
  let reconcilingOwnerId: number | null = null;
  let refreshPermitDepth = 0;
  let transitionSerial = 0;
  let lockListenerStarted = false;
  const lockState = createLockUnconfirmedState({
    lockWithToken: deps.failClosedLock || requestFailClosedLockWithToken,
    clearGeneralToken: clearToken,
    closeSessionUi: () => deps.closeSessionUi(true),
    clearWarning: deps.clearLockUnconfirmed,
    showWarning: deps.showLockUnconfirmed,
    storage: deps.lockStorage,
    subscribe: deps.lockSubscribe,
  });
  let requester: ReturnType<typeof createSessionRequester>;
  const boundaryState = createSessionBoundaryState({
    channel: deps.boundaryChannel,
    invalidateOwner: () => {
      reconcilingOwnerId = null;
      transitionOwner = null;
      transitionSerial += 1;
    },
    abortReads: () => requester.abortReads(),
    clearToken,
    scrubPrivateWorkspace: deps.scrubPrivateWorkspace,
    resetPrivateState: deps.resetPrivateState,
    closeSessionUi: deps.closeSessionUi,
    setTransitionUi: deps.setTransitionUi,
    markRefreshQueued: deps.markRefreshQueued,
  });
  requester = createSessionRequester({
    readToken,
    writeToken,
    clearToken,
    privacyGeneration: deps.privacyGeneration,
    boundaryGeneration: boundaryState.generation,
    canAdoptSession: () => !lockState.isLatched(),
    closeSessionUi: deps.closeSessionUi,
    isUnlockedUi: deps.isUnlockedUi,
    markRefreshQueued: deps.markRefreshQueued,
    handleDaemonBoundary: boundaryState.invalidateLocal,
    settleBoundary: boundaryState.settle,
    requestAllowed,
    isContextCurrent,
    isOwnerGenerationCurrent,
    containAmbiguousOutcome,
    request: deps.request,
  });
  const { api, waitForMutationDrain } = requester;
  function captureContext(): SessionRequestContext {
    return { token: readToken(), privacyGeneration: deps.privacyGeneration(),
      boundaryGeneration: boundaryState.generation() };
  }

  function isOwnerGenerationCurrent(
    context: SessionRequestContext | null | undefined,
  ): boolean {
    return Boolean(
      context &&
        context.privacyGeneration === deps.privacyGeneration() &&
        context.boundaryGeneration === boundaryState.generation() &&
        (context.transitionOwnerId == null ||
          transitionOwner?.id === context.transitionOwnerId),
    );
  }

  function isContextCurrent(context: SessionRequestContext | null | undefined): boolean {
    return Boolean(
      isOwnerGenerationCurrent(context) && context?.token === readToken(),
    );
  }

  function requestAllowed(
    path: string,
    context: SessionRequestContext | null,
  ): boolean {
    if (lockState.isLatched() ||
      (isSessionBoundaryPath(path) && context?.transitionOwnerId == null)) {
      return false;
    }
    const reconciliationRequest =
      reconcilingOwnerId != null &&
      transitionOwner?.id === reconcilingOwnerId &&
      refreshPermitDepth > 0;
    return !transitionOwner || reconciliationRequest ||
      (path === transitionOwner.path &&
        context?.transitionOwnerId === transitionOwner.id);
  }

  async function withTimeout(
    promise: Promise<unknown>, timeoutMs: number, message: string,
  ): Promise<void> {
    let timeout: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        promise,
        new Promise((_, reject) => {
          timeout = setTimeout(() => reject(new Error(message)), timeoutMs);
        }),
      ]);
    } finally {
      if (timeout) clearTimeout(timeout);
    }
  }

  async function containAmbiguousOutcome(
    path: string,
    capturedToken: string | null,
    stillCurrent: () => boolean,
  ): Promise<never> {
    const result = await lockState.contain(capturedToken, stillCurrent);
    if (result === "obsolete") throw new SessionContextChangedError();
    deps.markRefreshQueued();
    if (result === "unconfirmed") {
      reconcilingOwnerId = null;
      deps.setTransitionUi(true);
      throw new Error("LOCK NOT CONFIRMED. Stop the daemon before continuing.");
    }
    settleConfirmedLock(path);
    const operation = path === "/api/lock" ? "Lock" :
      path === "/api/compartment/switch" ? "Compartment switch" :
      isSessionBoundaryPath(path) ? "Session boundary" : "Session establishment";
    throw new Error(`${operation} outcome was ambiguous; fallback Lock confirmed.`);
  }

  function settleConfirmedLock(path?: string): void {
    const settledPath = path && isSessionBoundaryPath(path) ? path : "/api/lock";
    if (settledPath !== path) boundaryState.publish(settledPath);
    boundaryState.settle(settledPath);
  }

  function enterTransition(
    path: string,
    label: string,
    expectedContext: SessionRequestContext | null,
    supersede: boolean,
  ): SessionRequestContext {
    if (lockState.isLatched()) throw new SessionContextChangedError();
    if (transitionOwner && !supersede) throw new SessionContextChangedError();
    if (expectedContext && !isContextCurrent(expectedContext)) {
      throw new SessionContextChangedError();
    }
    // Cross-tab invalidation is published before any boundary operation can
    // reach the daemon. Browser storage events never fire back into this tab.
    if (!boundaryState.publish(path) && path !== "/api/lock") {
      throw new SessionContextChangedError();
    }
    const owner = { id: ++transitionSerial, path };
    reconcilingOwnerId = null;
    transitionOwner = owner;
    deps.setTransitionUi(true);
    deps.scrubPrivateWorkspace();
    deps.resetPrivateState();
    requester.abortReads();
    const transitionContext = {
      ...captureContext(),
      transitionOwnerId: owner.id,
      requestTimeoutMs: API_MUTATION_TIMEOUT_MS,
    };
    deps.renderTransitionState(label);
    return transitionContext;
  }

  async function beginTransition(
    path: string,
    label: string,
    expectedContext: SessionRequestContext | null = null,
  ): Promise<SessionRequestContext> {
    const context = enterTransition(path, label, expectedContext, false);
    try {
      await withTimeout(
        waitForMutationDrain(),
        SESSION_TRANSITION_DRAIN_TIMEOUT_MS,
        "Timed out waiting for the current operation to finish; transition cancelled.",
      );
      if (!isContextCurrent(context)) throw new SessionContextChangedError();
      return context;
    } catch (error) {
      // Settle only our still-current pre-send cancellation. An external
      // pending event may have superseded this owner while the drain waited.
      if (isContextCurrent(context)) boundaryState.settle(path);
      await endTransition(context);
      throw error;
    }
  }

  async function beginEmergencyLockTransition(
    path: string, label: string,
  ): Promise<SessionRequestContext> {
    return enterTransition(path, label, null, true);
  }

  async function endTransition(
    context: SessionRequestContext | null | undefined,
  ): Promise<void> {
    if (
      lockState.isLatched() ||
      !context ||
      transitionOwner?.id !== context.transitionOwnerId
    ) return;
    reconcilingOwnerId = context.transitionOwnerId ?? null;
    deps.markRefreshQueued();
    try {
      await deps.refresh();
    } finally {
      if (transitionOwner?.id === context.transitionOwnerId) {
        reconcilingOwnerId = null;
        transitionOwner = null;
        deps.setTransitionUi(false);
      }
    }
  }

  function runRefreshRequests<T>(operation: () => T): T {
    if (reconcilingOwnerId == null) return operation();
    refreshPermitDepth += 1;
    try {
      return operation();
    } finally {
      refreshPermitDepth -= 1;
    }
  }

  function enterPersistedUnconfirmedState(): boolean {
    if (!lockState.isLatched()) return false;
    reconcilingOwnerId = null;
    transitionOwner = { id: ++transitionSerial, path: "/api/lock-unconfirmed" };
    deps.setTransitionUi(true);
    deps.resetPrivateState();
    return lockState.restore();
  }

  async function releaseExternallyConfirmedLock(): Promise<void> {
    deps.clearLockUnconfirmed();
    const owner = transitionOwner;
    settleConfirmedLock(owner?.path);
    if (owner) {
      await endTransition({
        token: null,
        privacyGeneration: deps.privacyGeneration(),
        boundaryGeneration: boundaryState.generation(),
        transitionOwnerId: owner.id,
      });
    }
  }

  function initializeLockUnconfirmedState(): boolean {
    boundaryState.start();
    if (!lockListenerStarted) {
      lockListenerStarted = true;
      lockState.listen((latched) => {
        if (latched) enterPersistedUnconfirmedState();
        else void releaseExternallyConfirmedLock();
      });
    }
    return enterPersistedUnconfirmedState();
  }

  async function retryUnconfirmedLock(): Promise<boolean> {
    if (!lockState.isLatched() || !(await lockState.retry())) return false;
    const owner = transitionOwner;
    settleConfirmedLock(owner?.path);
    if (owner) {
      await endTransition({
        token: null,
        privacyGeneration: deps.privacyGeneration(),
        boundaryGeneration: boundaryState.generation(),
        transitionOwnerId: owner.id,
      });
    }
    return true;
  }

  async function acknowledgeDaemonRestart(confirmation: string): Promise<void> {
    if (!lockState.acknowledgeRestart(confirmation)) return;
    await releaseExternallyConfirmedLock();
  }

  return {
    acknowledgeDaemonRestart,
    api,
    beginEmergencyLockTransition,
    beginTransition,
    captureContext,
    endTransition,
    initializeLockUnconfirmedState,
    isContextCurrent,
    retryUnconfirmedLock,
    runRefreshRequests,
  };
}
