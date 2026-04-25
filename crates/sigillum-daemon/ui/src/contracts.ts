export interface ErrorResponse {
  error: string;
}

export interface ActiveCompartment {
  id: number;
  label: string;
  threshold: number;
  api_key_count: number;
  secret_count?: number | null;
}

export interface StatusResponse {
  initialized: boolean;
  locked: boolean;
  active_compartment?: ActiveCompartment | null;
  unlocked_compartments: ActiveCompartment[];
  session_token?: string;
}

export interface ChainProfile {
  name: string;
  chain_family: string;
  chain_id?: number | null;
  enabled: boolean;
  native_symbol: string;
  rpc_profile?: string | null;
  explorer_url?: string | null;
  capabilities: string[];
  source: string;
  updated_at_unix: number;
}

export interface WalletDiscoveryJob {
  id: string;
  kind: string;
  status: string;
  source: string;
  created_at_unix: number;
  updated_at_unix: number;
  failure_reason?: string | null;
}

export interface RiskFinding {
  id: string;
  severity: string;
  category: string;
  title: string;
  detail: string;
  source: string;
  updated_at_unix: number;
}

export interface ConsolidationPlanSummary {
  plan_id: string;
  status: string;
  step_count: number;
  blocked_step_count: number;
  review_required_step_count: number;
  approved_step_count: number;
  estimated_native_gas_wei?: string | null;
  created_at_unix: number;
  updated_at_unix: number;
}

export interface ApiRequestOptions<TBody = unknown> {
  method: "GET" | "POST" | "DELETE";
  path: string;
  body?: TBody;
  sessionToken?: string | null;
}
