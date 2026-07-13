export const SESSION_TOKEN_KEY = "sigillumSessionToken";

let volatileSessionToken: string | null = null;

function sessionStorageOrNull(): Storage | null {
  try {
    return window.sessionStorage;
  } catch (_) {
    return null;
  }
}

export function readSessionToken(): string | null {
  return (
    sessionStorageOrNull()?.getItem(SESSION_TOKEN_KEY) ?? volatileSessionToken
  );
}

export function writeSessionToken(token: string | null | undefined): void {
  if (!token) {
    return;
  }
  volatileSessionToken = token;
  try {
    sessionStorageOrNull()?.setItem(SESSION_TOKEN_KEY, token);
  } catch (_) {}
}

export function clearSessionToken(): void {
  volatileSessionToken = null;
  try {
    sessionStorageOrNull()?.removeItem(SESSION_TOKEN_KEY);
  } catch (_) {}
}

export type DaemonMethod = "GET" | "POST" | "DELETE";

export const FAIL_CLOSED_LOCK_TIMEOUT_MS = 10_000;

export class SessionContextChangedError extends Error {
  constructor() {
    super("The browser session changed while this request was in flight.");
    this.name = "SessionContextChangedError";
  }
}

export class DaemonHttpError extends Error {
  constructor(readonly status: number, message?: string) {
    super(message || `Daemon returned HTTP ${status} without a structured error.`);
    this.name = "DaemonHttpError";
  }
}

export function isSessionContextChangedError(error: unknown): boolean {
  return (
    error instanceof SessionContextChangedError ||
    (typeof error === "object" &&
      error !== null &&
      "name" in error &&
      (error as { name?: unknown }).name === "SessionContextChangedError")
  );
}

export interface DaemonPayload {
  error?: string;
  session_token?: string;
  [key: string]: unknown;
}

const DAEMON_HTTP_STATUS = Symbol("sigillumDaemonHttpStatus");

export function daemonHttpStatus(payload: unknown): number | null {
  if (!payload || typeof payload !== "object") return null;
  const status = (payload as Record<symbol, unknown>)[DAEMON_HTTP_STATUS];
  return typeof status === "number" ? status : null;
}

export function withDaemonHttpStatus<T extends DaemonPayload>(
  payload: T,
  status: number,
): T {
  Object.defineProperty(payload, DAEMON_HTTP_STATUS, { value: status });
  return payload;
}

export async function requestFailClosedLockWithToken(
  token: string,
  timeoutMs = FAIL_CLOSED_LOCK_TIMEOUT_MS,
): Promise<boolean> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch("/api/lock", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      signal: controller.signal,
    });
    // 423 means another Lock already latched, which is also fail-closed.
    if (response.status === 423) return true;
    if (response.status < 200 || response.status >= 300) return false;
    try {
      const payload = await response.json() as DaemonPayload;
      return !payload.error && payload.status === "locked";
    } catch (_) {
      return false;
    }
  } catch (_) {
    return false;
  } finally {
    clearTimeout(timeout);
  }
}

export async function requestWithSession<TPayload extends DaemonPayload = DaemonPayload>(
  method: DaemonMethod,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<TPayload> {
  const requestToken = readSessionToken();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(requestToken ? { Authorization: `Bearer ${requestToken}` } : {}),
  };
  const response = await fetch(path, {
    method,
    headers,
    signal,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  let payload: DaemonPayload = {};
  let parsedObject = false;
  try {
    const parsed = await response.json();
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      payload = parsed as DaemonPayload;
      parsedObject = true;
    }
  } catch (_) {}
  withDaemonHttpStatus(payload, response.status);
  const successfulHttp = response.status >= 200 && response.status < 300;
  if (successfulHttp && !parsedObject) {
    throw new DaemonHttpError(
      response.status,
      `Daemon returned HTTP ${response.status} without an object response.`,
    );
  }
  if (
    !successfulHttp &&
    (typeof payload.error !== "string" || payload.error.trim().length === 0)
  ) {
    throw new DaemonHttpError(response.status);
  }
  return payload as TPayload;
}
