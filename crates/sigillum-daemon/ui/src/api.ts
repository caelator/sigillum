import type { ApiRequestOptions, ErrorResponse } from "./contracts";

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

export async function requestJson<TResponse, TBody = unknown>(
  options: ApiRequestOptions<TBody>,
): Promise<TResponse> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (options.sessionToken) {
    headers.Authorization = `Bearer ${options.sessionToken}`;
  }
  if (options.background) {
    headers["X-Sigillum-Background"] = "1";
  }

  const response = await fetch(options.path, {
    method: options.method,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });

  const payload = (await response.json().catch(() => ({}))) as
    | TResponse
    | ErrorResponse;
  if (!response.ok) {
    const maybeError = payload as Partial<ErrorResponse>;
    const message =
      typeof maybeError.error === "string"
        ? maybeError.error
        : `Request failed with ${response.status}`;
    throw new ApiError(response.status, message);
  }
  return payload as TResponse;
}
