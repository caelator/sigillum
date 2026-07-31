/**
 * core/api.ts — typed daemon API client for the strict-typed console core
 * (plan task 4.1).
 *
 * One client, one error shape. Every method wraps `requestWithSession`
 * (session token from `api/session.ts` storage, 401 token clearing included)
 * and returns a typed DTO from `contracts.ts`. Failures throw {@link ApiError}
 * carrying an {@link ApiFailure} — a discriminated union on the daemon's
 * structured error `code` (plan task 1.4) so views branch deterministically:
 *
 * ```ts
 * try {
 *   await api.getTreasuryOverview();
 * } catch (e) {
 *   const failure = apiFailure(e);
 *   if (failure?.code === "vault_locked") router.navigate("#/vault"); // unlock
 * }
 * ```
 *
 * Query-bearing methods take typed option bags mirroring the plan-task-1.5
 * pagination/filter/sort parameters; a parameterless call keeps the legacy
 * (unpaginated) response.
 */

import type {
  AuditListQuery,
  AuditResponse,
  ConsolidationPlanListQuery,
  ConsolidationPlanListResponse,
  DiagnosticsResponse,
  EthStealthDepositListQuery,
  EthStealthDepositListResponse,
  FieldError,
  OperationListResponse,
  OperationMutationResponse,
  OperationResponse,
  QueueExecutionPauseResponse,
  QueueJobListQuery,
  QueueJobListResponse,
  QueueProcessRequest,
  QueueProcessResponse,
  ReceivingOverviewResponse,
  SelfCheckRunResponse,
  StatusResponse,
  TreasuryOverviewResponse,
  TreasuryPolicyMutationResponse,
  TreasuryPolicyResponse,
  TreasuryPolicyUpdateRequest,
  WalletInventoryListQuery,
  WalletInventoryListResponse,
} from "../contracts";
import { requestWithSession } from "../api/session";

// ── Error model ───────────────────────────────────────────────────────

/** The daemon's structured error codes (sigillum-api error_codes.rs). */
export type KnownApiErrorCode =
  | "validation_failed"
  | "bad_request"
  | "typed_confirmation_mismatch"
  | "unauthorized"
  | "forbidden"
  | "vault_locked"
  | "execution_gate_denied"
  | "capability_scope_denied"
  | "policy_violation"
  | "not_found"
  | "not_initialized"
  | "conflict"
  | "locked_in_progress"
  | "rate_limited"
  | "unlock_throttled"
  | "internal"
  | "unavailable"
  | "unknown";

interface FailureWithCode<C extends string> {
  code: C;
  /** Human-readable message from the daemon. */
  error: string;
  /** Optional recovery hint (e.g. "unlock", "retry"). */
  action?: string;
  /** Field-level validation details (validation_failed). */
  fields?: FieldError[];
}

/**
 * The error every client method can throw, discriminated by `code`. Known
 * codes get their own members so `failure.code === "vault_locked"` narrows;
 * the trailing `(string & {})` member keeps codes added by newer daemons
 * representable (per the 1.x compatibility rule).
 */
export type ApiFailure =
  | FailureWithCode<"validation_failed">
  | FailureWithCode<"bad_request">
  | FailureWithCode<"typed_confirmation_mismatch">
  | FailureWithCode<"unauthorized">
  | FailureWithCode<"forbidden">
  | FailureWithCode<"vault_locked">
  | FailureWithCode<"execution_gate_denied">
  | FailureWithCode<"capability_scope_denied">
  | FailureWithCode<"policy_violation">
  | FailureWithCode<"not_found">
  | FailureWithCode<"not_initialized">
  | FailureWithCode<"conflict">
  | FailureWithCode<"locked_in_progress">
  | FailureWithCode<"rate_limited">
  | FailureWithCode<"unlock_throttled">
  | FailureWithCode<"internal">
  | FailureWithCode<"unavailable">
  | FailureWithCode<"unknown">
  | FailureWithCode<string & {}>;

export class ApiError extends Error {
  readonly failure: ApiFailure;

  constructor(failure: ApiFailure) {
    super(failure.error);
    this.name = "ApiError";
    this.failure = failure;
  }

  get code(): ApiFailure["code"] {
    return this.failure.code;
  }

  get fields(): FieldError[] | undefined {
    return this.failure.fields;
  }
}

/** Unwrap an {@link ApiError} (or a raw failure-shaped value) for branching. */
export function apiFailure(error: unknown): ApiFailure | null {
  if (error instanceof ApiError) return error.failure;
  if (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "error" in error
  ) {
    return error as ApiFailure;
  }
  return null;
}

/** Type guard for one specific failure code, e.g. `isApiFailure(e, "vault_locked")`. */
export function isApiFailure<C extends ApiFailure["code"]>(
  error: unknown,
  code: C,
): boolean {
  return apiFailure(error)?.code === code;
}

// ── Query strings ─────────────────────────────────────────────────────

type QueryValue = string | number | boolean | null | undefined;

function buildQuery(params: Record<string, QueryValue>): string {
  const parts: string[] = [];
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue;
    parts.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
  }
  return parts.length ? `?${parts.join("&")}` : "";
}

// ── Client ────────────────────────────────────────────────────────────

async function request<T>(
  method: "GET" | "POST" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  let payload: unknown;
  try {
    payload = await requestWithSession(method, path, body);
  } catch (error) {
    // Network-level failure (daemon down, fetch rejected): not a daemon
    // error envelope, so synthesize the `unavailable` failure.
    throw new ApiError({
      code: "unavailable",
      error: error instanceof Error ? error.message : String(error),
    });
  }
  const envelope = payload as {
    code?: string;
    error?: string;
    action?: string;
    fields?: FieldError[];
  } | null;
  if (envelope && envelope.error != null) {
    throw new ApiError({
      code: envelope.code ?? "unknown",
      error: envelope.error,
      action: envelope.action,
      fields: envelope.fields,
    });
  }
  return payload as T;
}

export interface DaemonApi {
  // Status & lifecycle (passive: does not extend idle lock)
  getStatus(): Promise<StatusResponse>;
  // Operations registry (plan task 1.1)
  listOperations(): Promise<OperationListResponse>;
  getOperation(id: string): Promise<OperationResponse>;
  cancelOperation(id: string): Promise<OperationMutationResponse>;
  // Queue (plan task 1.5 pagination on list)
  listQueueJobs(query?: QueueJobListQuery): Promise<QueueJobListResponse>;
  pauseQueue(): Promise<QueueExecutionPauseResponse>;
  resumeQueue(): Promise<QueueExecutionPauseResponse>;
  processQueue(body?: QueueProcessRequest): Promise<QueueProcessResponse>;
  // Consolidation plans (Move)
  listPlans(
    query?: ConsolidationPlanListQuery,
  ): Promise<ConsolidationPlanListResponse>;
  // Treasury (Move/Overview)
  getTreasuryOverview(): Promise<TreasuryOverviewResponse>;
  getTreasuryPolicy(): Promise<TreasuryPolicyResponse>;
  updateTreasuryPolicy(
    body: TreasuryPolicyUpdateRequest,
  ): Promise<TreasuryPolicyMutationResponse>;
  // Receiving
  getReceivingOverview(options?: {
    includeRetired?: boolean;
  }): Promise<ReceivingOverviewResponse>;
  listDeposits(
    query?: EthStealthDepositListQuery,
  ): Promise<EthStealthDepositListResponse>;
  // Portfolio
  listInventoryWallets(
    query?: WalletInventoryListQuery,
  ): Promise<WalletInventoryListResponse>;
  // Audit / health
  listAudit(query?: AuditListQuery): Promise<AuditResponse>;
  runSelfCheck(): Promise<SelfCheckRunResponse>;
  getDiagnostics(): Promise<DiagnosticsResponse>;
}

export function createDaemonApi(): DaemonApi {
  return {
    getStatus: () => request("GET", "/api/status"),

    listOperations: () => request("GET", "/api/operations"),
    getOperation: (id) =>
      request("GET", `/api/operations/${encodeURIComponent(id)}`),
    cancelOperation: (id) =>
      request("POST", `/api/operations/${encodeURIComponent(id)}/cancel`),

    listQueueJobs: (query) =>
      request("GET", `/api/queue/jobs${buildQuery({ ...(query ?? {}) })}`),
    pauseQueue: () => request("POST", "/api/queue/pause"),
    resumeQueue: () => request("POST", "/api/queue/resume"),
    processQueue: (body) => request("POST", "/api/queue/process", body ?? {}),

    listPlans: (query) =>
      request(
        "GET",
        `/api/plans/consolidation${buildQuery({ ...(query ?? {}) })}`,
      ),

    getTreasuryOverview: () => request("GET", "/api/treasury/overview"),
    getTreasuryPolicy: () => request("GET", "/api/treasury/policy"),
    updateTreasuryPolicy: (body) =>
      request("POST", "/api/treasury/policy/update", body),

    getReceivingOverview: (options) =>
      request(
        "GET",
        `/api/receiving/overview${buildQuery({
          include_retired: options?.includeRetired,
        })}`,
      ),
    listDeposits: (query) =>
      request(
        "GET",
        `/api/deposits/eth-stealth${buildQuery({ ...(query ?? {}) })}`,
      ),

    listInventoryWallets: (query) =>
      request(
        "GET",
        `/api/inventory/wallets${buildQuery({ ...(query ?? {}) })}`,
      ),

    listAudit: (query) =>
      request("GET", `/api/audit${buildQuery({ ...(query ?? {}) })}`),
    runSelfCheck: () => request("POST", "/api/selfcheck/run"),
    getDiagnostics: () => request("GET", "/api/diagnostics"),
  };
}
