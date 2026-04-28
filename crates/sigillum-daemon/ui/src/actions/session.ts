import { requestJson } from "../api";
import type {
  LockResponse,
  SessionRevokeResponse,
  UnlockResponse,
} from "../contracts";
import {
  clearSessionToken,
  readSessionToken,
  writeSessionToken,
} from "../api/session";

export interface PassphraseUnlockInput {
  compartment_id?: number | null;
  passphrase: string;
}

export async function unlockWithPassphrase(
  input: PassphraseUnlockInput,
): Promise<UnlockResponse> {
  const status = await requestJson<UnlockResponse, PassphraseUnlockInput>({
    method: "POST",
    path: "/api/unlock",
    body: input,
  });
  writeSessionToken(status.session_token);
  return status;
}

export async function lockAll(): Promise<LockResponse> {
  const status = await requestJson<LockResponse>({
    method: "POST",
    path: "/api/lock",
    sessionToken: readSessionToken(),
  });
  clearSessionToken();
  return status;
}

export function logoutLocalSession(): void {
  clearSessionToken();
}

export async function revokeSession(): Promise<SessionRevokeResponse> {
  const response = await requestJson<SessionRevokeResponse>({
    method: "POST",
    path: "/api/session/revoke",
    sessionToken: readSessionToken(),
  });
  clearSessionToken();
  return response;
}
