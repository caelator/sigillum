import type { FieldError } from "../contracts";

export const SESSION_TOKEN_KEY = "sigillumSessionToken";

let volatileSessionToken: string | null = null;
const sessionTokenListeners = new Set<(token: string | null) => void>();

function notifySessionTokenListeners(token: string | null): void {
  for (const listener of Array.from(sessionTokenListeners)) {
    try {
      listener(token);
    } catch (_) {
      // Token persistence and revocation must not depend on an observer.
    }
  }
}

/** Observe same-tab token changes (the browser storage event does not). */
export function subscribeSessionToken(
  listener: (token: string | null) => void,
): () => void {
  sessionTokenListeners.add(listener);
  return () => sessionTokenListeners.delete(listener);
}

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
  const previous = readSessionToken();
  volatileSessionToken = token;
  try {
    sessionStorageOrNull()?.setItem(SESSION_TOKEN_KEY, token);
  } catch (_) {}
  if (previous !== token) notifySessionTokenListeners(token);
}

export function clearSessionToken(): void {
  const previous = readSessionToken();
  volatileSessionToken = null;
  try {
    sessionStorageOrNull()?.removeItem(SESSION_TOKEN_KEY);
  } catch (_) {}
  if (previous !== null) notifySessionTokenListeners(null);
}

export function sessionAuthorizationHeader(
  token: string | null = readSessionToken(),
): Record<string, string> {
  return token ? { Authorization: `Bearer ${token}` } : {};
}

export type DaemonMethod = "GET" | "POST" | "DELETE";

export interface DaemonPayload {
  error?: string;
  code?: string;
  fields?: FieldError[];
  session_token?: string;
  [key: string]: unknown;
}

export async function requestWithSession<TPayload extends DaemonPayload = DaemonPayload>(
  method: DaemonMethod,
  path: string,
  body?: unknown,
): Promise<TPayload> {
  const requestSessionToken = readSessionToken();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...sessionAuthorizationHeader(requestSessionToken),
  };
  const response = await fetch(path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  let payload: DaemonPayload = {};
  try {
    payload = (await response.json()) as DaemonPayload;
  } catch (_) {}

  if (payload.session_token) {
    writeSessionToken(payload.session_token);
  }
  // A response can arrive after a reauthentication rotated the tab to a new
  // token. Never let a stale 401 revoke that newer session.
  if (
    response.status === 401 &&
    readSessionToken() === requestSessionToken
  ) {
    clearSessionToken();
  }
  return payload as TPayload;
}
