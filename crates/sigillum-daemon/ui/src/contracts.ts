export interface ErrorResponse {
  error: string;
  action?: string;
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
  checkpoints?: WalletDiscoveryCheckpoint[];
  created_at_unix: number;
  updated_at_unix: number;
  failure_reason?: string | null;
}

export interface WalletDiscoveryCheckpoint {
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  derivation_pattern?: string | null;
  account_index?: number | null;
  next_index: number;
  last_scanned_index?: number | null;
  consecutive_empty: number;
  completed: boolean;
  updated_at_unix: number;
}

export interface NftMetadataCacheEntry {
  chain_id: number;
  contract_address: string;
  token_id_hex: string;
  metadata_uri?: string | null;
  name?: string | null;
  spam_label: string;
  updated_at_unix: number;
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
  derivation_pattern?: string | null;
  account_index?: number | null;
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
  policy_violations?: string[];
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

export interface TreasuryChainSummary {
  chain_id: number;
  native_symbol: string;
  address_count: number;
  funded_address_count: number;
  native_total_wei_hex: string;
}

export interface TreasuryGroupSummary {
  wallet_family: string;
  wallet_profile: string;
  chain_id: number;
  address_count: number;
  funded_address_count: number;
  native_total_wei_hex: string;
  signer_address_count: number;
  watch_only_address_count: number;
  erc20_holding_count: number;
  nft_holding_count: number;
  defi_holding_count: number;
  claimable_holding_count: number;
  approval_exposure_count: number;
  dormant_candidate_count: number;
}

export interface TreasuryRoutingStatus {
  wallet_profile: string;
  hot_address?: string | null;
  treasury_address?: string | null;
  default_destination_address?: string | null;
  hot_native_balance_wei_hex?: string | null;
  treasury_native_balance_wei_hex?: string | null;
  routing_ready: boolean;
}

export interface TreasuryRiskSummary {
  total_findings: number;
  critical_findings: number;
  high_findings: number;
  medium_findings: number;
  low_findings: number;
}

export interface TreasuryPlanSummary {
  total_plans: number;
  latest_plan_id?: string | null;
  latest_plan_status?: string | null;
  latest_review_required_steps: number;
  latest_approved_steps: number;
  latest_executable_steps: number;
  latest_blocked_steps: number;
  policy_violations?: string[];
  latest_policy_violations?: string[];
}

export interface TreasuryReceiveSummary {
  active_allocations: number;
  retired_allocations: number;
  purposes: number;
}

export interface TreasuryOverviewResponse {
  generated_at_unix: number;
  tracked_address_count: number;
  funded_address_count: number;
  watch_only_address_count: number;
  signer_address_count: number;
  chains?: TreasuryChainSummary[];
  groups?: TreasuryGroupSummary[];
  routing?: TreasuryRoutingStatus[];
  risk: TreasuryRiskSummary;
  plans: TreasuryPlanSummary;
  receive?: TreasuryReceiveSummary;
}

export interface TreasuryAllowedDestination {
  address: string;
  label?: string | null;
}

export interface TreasuryPolicy {
  enabled: boolean;
  allowed_destinations?: TreasuryAllowedDestination[];
  max_step_native_wei_hex?: string | null;
  max_plan_native_wei_hex?: string | null;
  require_simulation: boolean;
  created_at_unix: number;
  updated_at_unix: number;
}

export interface TreasuryPolicyUpdateRequest {
  enabled: boolean;
  allowed_destinations?: TreasuryAllowedDestination[];
  max_step_native_wei_hex?: string | null;
  max_plan_native_wei_hex?: string | null;
  require_simulation?: boolean | null;
}

export interface TreasuryReceiveAllocation {
  id: string;
  wallet_family: string;
  wallet_profile: string;
  address: string;
  derivation_path: string;
  address_index: number;
  purpose: string;
  label?: string | null;
  status: string;
  created_at_unix: number;
  retired_at_unix?: number | null;
}

export type SelfCheckStatus = "pass" | "warn" | "fail";

export interface SelfCheckResult {
  id: string;
  domain: string;
  subject: string;
  status: SelfCheckStatus;
  detail: string;
  latency_ms?: number | null;
}

export interface SelfCheckRunResponse {
  status: SelfCheckStatus;
  generated_at_unix: number;
  checks: SelfCheckResult[];
}

export interface EthXpubWalletProfile {
  name: string;
  project_account: number;
  provider_profile: string;
  compartment_id: number;
  chain_id?: number | null;
  external_receive_xpub?: string | null;
  external_receive_path?: string | null;
  external_account_xpub?: string | null;
  external_account_path?: string | null;
  default_destination_address?: string | null;
  execution_enabled: boolean;
}

export interface EthSeedWalletProfile {
  name: string;
  label?: string | null;
  project_account: number;
  provider_profile: string;
  compartment_id: number;
  chain_id?: number | null;
  word_count: number;
  mnemonic_secret_key: string;
  account_path: string;
  receive_path: string;
  receive_xpub: string;
  first_receive_address: string;
  default_destination_address?: string | null;
  control_xpub?: string | null;
  sponsor_address?: string | null;
  hot_address?: string | null;
  treasury_address?: string | null;
  execution_enabled: boolean;
}

export interface EthSeedWalletCreateResponse {
  status: string;
  /**
   * Server-generated BIP-39 phrase, returned exactly once for operator
   * backup. Never log, persist, or re-display this value.
   */
  mnemonic: string;
  profile: EthSeedWalletProfile;
}

export interface ApiRequestOptions<TBody = unknown> {
  method: "GET" | "POST" | "DELETE";
  path: string;
  body?: TBody;
  sessionToken?: string | null;
}
