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
