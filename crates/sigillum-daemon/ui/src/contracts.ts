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

export interface WatchAddressBookEntry {
  id: string;
  address: string;
  label: string;
  tags: string[];
  source: string;
  enabled: boolean;
  created_at_unix: number;
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

export interface WalletInventoryAddress {
  id: string;
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  chain_id: number;
  address: string;
  derivation_path: string;
  address_index: number;
  activity_state: string;
  native_balance_wei_hex: string;
  transaction_count: number;
  classifications?: string[];
  source: string;
  first_seen_at_unix: number;
  last_checked_at_unix: number;
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
  protocol_address?: string | null;
  claim_adapter?: string | null;
  claim_index_hex?: string | null;
  claim_proof?: string[];
  amount_hex: string;
  destination_address?: string | null;
  signer_status: string;
  simulation_status: string;
  simulation_evidence?: string[];
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

export interface ConsolidationPlanExportCall {
  step_id: string;
  action: string;
  from_address: string;
  to_address: string;
  value_wei_hex: string;
  data_hex: string;
  operation: number;
  chain_id: number;
  provider_profile: string;
  asset_kind: string;
  amount_hex: string;
  evidence: string[];
}

export interface SafeTransactionBuilderTransaction {
  to: string;
  value: string;
  data: string;
  operation: number;
}

export interface SafeTransactionBuilderBatch {
  version: string;
  chainId: string;
  meta: {
    name: string;
    description: string;
    txBuilderVersion: string;
    createdFromSafeAddress?: string | null;
  };
  transactions: SafeTransactionBuilderTransaction[];
}

export interface ConsolidationPlanExportBundle {
  chain_id: number;
  provider_profile: string;
  source_address?: string | null;
  safe_address?: string | null;
  calls: ConsolidationPlanExportCall[];
  safe_transaction_builder?: SafeTransactionBuilderBatch | null;
}

export interface ConsolidationPlanExportSkippedStep {
  step_id: string;
  action: string;
  reason: string;
  blockers: string[];
}

export interface ConsolidationPlanExportResponse {
  status: string;
  plan_id: string;
  format: string;
  exported_steps: number;
  skipped_steps: ConsolidationPlanExportSkippedStep[];
  bundles: ConsolidationPlanExportBundle[];
}

export interface ApiRequestOptions<TBody = unknown> {
  method: "GET" | "POST" | "DELETE";
  path: string;
  body?: TBody;
  sessionToken?: string | null;
}
