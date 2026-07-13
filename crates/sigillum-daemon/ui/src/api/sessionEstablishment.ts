import { daemonHttpStatus } from "./session";

const SESSION_ESTABLISHING_PATHS = new Set([
  "/api/unlock",
  "/api/fido2/unlock",
  "/api/biometric/unlock",
  "/api/compartment/init",
  "/api/fido2/setup",
]);

export function isSessionEstablishingPath(path: string): boolean {
  return SESSION_ESTABLISHING_PATHS.has(path);
}

export function isExplicitSessionRejection(payload: unknown): boolean {
  const status = daemonHttpStatus(payload);
  return status != null && status >= 400 && status < 500;
}

export function canonicalSessionToken(payload: any): string | null {
  const token = payload?.session_token;
  return typeof token === "string" && /^[0-9a-f]{64}$/.test(token)
    ? token
    : null;
}

function validUnlockedResponse(payload: any, expectedMethod: string): boolean {
  const unlocked = payload?.unlocked_compartments;
  const activeId = payload?.active_compartment_id;
  return payload?.status === "unlocked" &&
    payload?.method === expectedMethod &&
    Array.isArray(unlocked) && unlocked.length > 0 &&
    unlocked.every((item: any) => Number.isInteger(item?.id)) &&
    Number.isInteger(activeId) &&
    unlocked.some((item: any) => item.id === activeId);
}

export function sessionEstablishmentToken(
  method: string,
  path: string,
  body: any,
  payload: any,
): string | null {
  const httpStatus = daemonHttpStatus(payload);
  if (
    method !== "POST" || httpStatus == null || httpStatus < 200 ||
    httpStatus >= 300 || !payload || typeof payload !== "object" ||
    Object.prototype.hasOwnProperty.call(payload, "error")
  ) return null;
  const token = canonicalSessionToken(payload);
  if (!token) return null;

  if (path === "/api/unlock") {
    return validUnlockedResponse(payload, "passphrase") ? token : null;
  }
  if (path === "/api/fido2/unlock") {
    return validUnlockedResponse(payload, "fido2") ? token : null;
  }
  if (path === "/api/biometric/unlock") {
    return validUnlockedResponse(payload, "biometric") ? token : null;
  }
  if (path === "/api/compartment/init") {
    return payload.status === "initialized" && Number.isInteger(body?.id) &&
      payload.compartment_id === body.id &&
      typeof payload.compartment_label === "string" &&
      payload.compartment_label.length > 0 ? token : null;
  }
  if (path === "/api/fido2/setup") {
    return payload.status === "setup_complete" && payload.unlocked === true &&
      Number.isInteger(payload.total_keys) && payload.total_keys > 0 &&
      Array.isArray(body?.compartments) && body.compartments.length > 0 &&
      payload.compartments === body.compartments.length
      ? token : null;
  }
  return null;
}
