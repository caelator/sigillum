import type { FieldError } from "../contracts";

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

export function sessionAuthorizationHeader(): Record<string, string> {
  const token = readSessionToken();
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
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...sessionAuthorizationHeader(),
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
  if (response.status === 401) {
    clearSessionToken();
  }
  return payload as TPayload;
}
