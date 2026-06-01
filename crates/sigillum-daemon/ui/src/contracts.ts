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

export interface UnlockResponse {
  status: string;
  method: string;
  cascading?: boolean | null;
  session_token: string;
  unlocked_compartments: ActiveCompartment[];
  active_compartment_id?: number | null;
}

export interface LockResponse {
  status: string;
  message: string;
}

export interface SessionRevokeResponse {
  status: string;
  requires_reauth: boolean;
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

export interface RiskCatalogEntry {
  address: string;
  label: string;
  risk_level: string;
  source: string;
  notes: string[];
  created_at_unix: number;
  updated_at_unix: number;
}

export interface ConsolidationPlanSummary {
  total_steps: number;
  blocked_steps: number;
  review_required_steps: number;
  approved_steps: number;
  executable_steps: number;
  value_items: number;
}

export interface ConsolidationPlanStep {
  id: string;
  action: string;
  status: string;
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  chain_id: number;
  address: string;
  derivation_path: string;
  asset_kind: string;
  asset_address?: string | null;
  token_id_hex?: string | null;
  counterparty_address?: string | null;
  amount_hex: string;
  destination_address?: string | null;
  signer_status: string;
  simulation_status: string;
  risk_level: string;
  blockers: string[];
  auto_eligible: boolean;
  approved: boolean;
}

export interface ConsolidationPlan {
  id: string;
  status: string;
  destination_address?: string | null;
  created_at_unix: number;
  updated_at_unix: number;
  summary: ConsolidationPlanSummary;
  steps: ConsolidationPlanStep[];
}

export interface ApiRequestOptions<TBody = unknown> {
  method: "GET" | "POST" | "DELETE";
  path: string;
  body?: TBody;
  sessionToken?: string | null;
}
