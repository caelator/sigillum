export interface FieldError {
  field: string;
  message: string;
}

export interface ErrorResponse {
  code: string;
  error: string;
  action?: string;
  fields?: FieldError[];
}

/// Mirrors sigillum-api response.rs exactly: structs that *reference* the
/// active compartment use the qualified `compartment_id`/`compartment_label`
/// names, while structs that *are* a compartment (`UnlockedCompartment`)
/// use the short `id`/`label`. Do not collapse these into one shape.
export interface ActiveCompartment {
  compartment_id: number;
  compartment_label: string;
  api_key_count: number;
  secret_count?: number | null;
}

export interface UnlockedCompartment {
  id: number;
  label: string;
  threshold: number;
  passphrase_mode?: string | null;
}

export interface StatusResponse {
  initialized: boolean;
  locked: boolean;
  active_compartment?: ActiveCompartment | null;
  unlocked_compartments: UnlockedCompartment[];
  session_token?: string;
}

export interface UnlockResponse {
  status: string;
  method: string;
  cascading?: boolean | null;
  session_token: string;
  unlocked_compartments: UnlockedCompartment[];
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
  native_decimals: number;
  finality_blocks: number;
  dormancy_block_window?: number;
  permit2_address?: string | null;
  uniswap_v2_router_address?: string | null;
  rpc_profile?: string | null;
  explorer_url?: string | null;
  capabilities: string[];
  source: string;
  builtin: boolean;
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

export interface TokenRegistryEntry {
  chain_id: number;
  address: string;
  symbol: string;
  decimals: number;
}

export interface TokenRegistryList {
  id: string;
  name: string;
  compartment_id: number;
  source: string;
  entries: TokenRegistryEntry[];
  created_at_unix: number;
  updated_at_unix: number;
}

export interface WalletDiscoveryJob {
  id: string;
  status: string;
  source: string;
  wallet_families?: string[];
  wallet_profiles?: string[];
  provider_profiles?: string[];
  chain_ids?: number[];
  gap_limit?: number;
  max_index?: number;
  addresses_scanned?: number;
  active_addresses?: number;
  holdings_detected?: number;
  checkpoints?: WalletDiscoveryCheckpoint[];
  block_cursors?: WalletDiscoveryBlockCursor[];
  started_at_unix?: number;
  completed_at_unix?: number | null;
  last_error?: string | null;
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

export interface WalletDiscoveryBlockCursor {
  address: string;
  chain_id: number;
  topic_family: string;
  last_scanned_block: number;
  updated_at_unix: number;
}

// Background daemon operations (GET /api/operations, scan run_async).
// Mirrors sigillum-api response/operations.rs; states/kinds are free-form
// strings — treat unrecognized values as opaque.
export interface OperationProgress {
  processed: number;
  total?: number;
}

export interface Operation {
  id: string;
  kind: string;
  state: string;
  progress: OperationProgress;
  related_ids?: string[];
  created_at_unix: number;
  updated_at_unix: number;
  completed_at_unix?: number | null;
  error?: string | null;
}

export interface NftMetadataCacheEntry {
  chain_id: number;
  contract_address: string;
  token_id_hex: string;
  metadata_uri?: string | null;
  name?: string | null;
  spam_label: string;
  spam_reasons?: string[];
  fetched_at_unix?: number | null;
  fetched_uri?: string | null;
  content_sha256?: string | null;
  fetch_skipped_reason?: string | null;
  updated_at_unix: number;
}

export interface NftMetadataCollectionOptIn {
  chain_id: number;
  contract_address: string;
  enabled: boolean;
  created_at_unix: number;
  updated_at_unix: number;
}

export interface RiskFinding {
  id: string;
  category: string;
  risk_level: string;
  status: string;
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  chain_id: number;
  address: string;
  subject_type: string;
  subject: string;
  source: string;
  recommendation: string;
  evidence?: string[];
  first_seen_at_unix: number;
  last_checked_at_unix: number;
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

type WireString<T extends string> = T | (string & {});

export type WalletAddressActivityState = WireString<"funded" | "active" | "empty">;

export type WalletAddressClassification = WireString<
  | "signer_available"
  | "watch_only"
  | "signer_unknown"
  | "gas_available"
  | "transaction_history"
  | "token_holding"
  | "nft_holding"
  | "protocol_holding"
  | "value_detected"
  | "asset_value_detected"
  | "stranded_value"
  | "approval_exposure"
  | "dormant_candidate"
  | "empty_candidate"
>;

export type WalletAssetKind = WireString<
  | "native"
  | "erc20"
  | "erc721"
  | "erc1155"
  | "nft"
  | "approval"
  | "defi"
  | "airdrop"
  | "reward"
>;

export type WalletPlanStepAction = WireString<
  | "sweep_native"
  | "sweep_erc20"
  | "sweep_nft"
  | "revoke_erc20_approval"
  | "revoke_permit2_allowance"
  | "revoke_nft_operator_approval"
  | "revoke_approval"
  | "exit_defi_position"
  | "claim_reward"
  | "review_asset"
>;

export type WalletPlanStepStatus = WireString<"review_required" | "blocked" | "approved">;
export type WalletSignerStatus = WireString<"watch_only" | "available" | "unknown">;
export type WalletSimulationStatus = WireString<
  "required" | "not_run" | "passed" | "failed" | "unsupported" | "blocked"
>;
export type WalletPlanStatus = WireString<"empty" | "blocked" | "review_required" | "approved">;

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
  activity_state: WalletAddressActivityState;
  native_balance_wei_hex: string;
  transaction_count: number;
  last_activity_block?: number | null;
  classifications?: WalletAddressClassification[];
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
  sequence?: number;
  depends_on?: string[];
  action: WalletPlanStepAction;
  status: WalletPlanStepStatus;
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  chain_id: number;
  address: string;
  derivation_path: string;
  asset_kind: WalletAssetKind;
  asset_address?: string | null;
  token_id_hex?: string | null;
  counterparty_address?: string | null;
  protocol_address?: string | null;
  claim_adapter?: string | null;
  claim_index_hex?: string | null;
  claim_proof?: string[];
  exit_token0_address?: string | null;
  exit_token1_address?: string | null;
  exit_amount0_min_hex?: string | null;
  exit_amount1_min_hex?: string | null;
  exit_deadline_unix?: number | null;
  amount_hex: string;
  destination_address?: string | null;
  signer_status: WalletSignerStatus;
  simulation_status: WalletSimulationStatus;
  simulation_evidence?: string[];
  risk_level: string;
  blockers: string[];
  linkage_warnings?: string[];
  auto_eligible: boolean;
  approved: boolean;
  /** Queue job id once the step has been enqueued for execution (W7.2). */
  queued_job_id?: string | null;
}

export interface ConsolidationPlan {
  id: string;
  status: WalletPlanStatus;
  chain_id: number;
  destination_address?: string | null;
  /** Absent for operator-generated plans; `treasury_automation` for maintenance drafts. */
  origin?: string | null;
  created_at_unix: number;
  updated_at_unix: number;
  summary: ConsolidationPlanSummary;
  steps: ConsolidationPlanStep[];
  policy_violations?: string[];
  linkage_findings?: string[];
  risk_findings?: RiskFinding[];
}

export interface PartyDestination {
  counterparty_id: string;
  destination_address: string;
}

export interface ConsolidationPlanGenerateRequest {
  destination_address?: string | null;
  wallet_family?: string | null;
  wallet_profile?: string | null;
  provider_profile?: string | null;
  chain_id?: number | null;
  include_watch_only?: boolean | null;
  auto_queue_low_risk?: boolean | null;
  routing_strategy?: string | null;
  party_destinations?: PartyDestination[];
}

export interface ConsolidationPlanExportCall {
  step_id: string;
  action: WalletPlanStepAction;
  from_address: string;
  to_address: string;
  value_wei_hex: string;
  data_hex: string;
  operation: number;
  chain_id: number;
  provider_profile: string;
  asset_kind: WalletAssetKind;
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
  action: WalletPlanStepAction;
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
  automation?: TreasuryAutomationStatus;
}

export interface TreasuryAutomationStatus {
  enabled: boolean;
  hot_overflow_wei_hex?: string | null;
  generated_steps: number;
  enqueued_steps: number;
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
  hot_floor_wei_hex?: string;
  hot_target_wei_hex?: string;
  hot_overflow_wei_hex?: string | null;
  require_simulation: boolean;
  block_cross_party_linkage?: boolean;
  allow_claim_execution?: boolean;
  allow_gas_topups?: boolean;
  allow_treasury_automation?: boolean;
  max_gas_topup_wei_hex?: string | null;
  allow_plan_execution?: boolean;
  allow_sweep_execution?: boolean;
  allow_revoke_execution?: boolean;
  allow_exit_execution?: boolean;
  execution_paused?: boolean;
  max_fee_per_gas_cap_hex?: string | null;
  simulation_freshness_secs?: number;
  created_at_unix: number;
  updated_at_unix: number;
}

export interface TreasuryPolicyUpdateRequest {
  enabled: boolean;
  allowed_destinations?: TreasuryAllowedDestination[];
  max_step_native_wei_hex?: string | null;
  max_plan_native_wei_hex?: string | null;
  hot_floor_wei_hex?: string | null;
  hot_target_wei_hex?: string | null;
  hot_overflow_wei_hex?: string | null;
  require_simulation?: boolean | null;
  block_cross_party_linkage?: boolean | null;
  allow_claim_execution?: boolean | null;
  allow_gas_topups?: boolean | null;
  allow_treasury_automation?: boolean | null;
  max_gas_topup_wei_hex?: string | null;
  allow_plan_execution?: boolean | null;
  allow_sweep_execution?: boolean | null;
  allow_revoke_execution?: boolean | null;
  allow_exit_execution?: boolean | null;
  max_fee_per_gas_cap_hex?: string | null;
  simulation_freshness_secs?: number | null;
}

export interface TreasuryReceiveAllocation {
  id: string;
  wallet_family: string;
  wallet_profile: string;
  chain_id: number;
  chain_id_assumed?: boolean;
  address: string;
  derivation_path: string;
  address_index: number;
  purpose: string;
  label?: string | null;
  status: string;
  created_at_unix: number;
  retired_at_unix?: number | null;
  counterparty_id?: string | null;
  // Plan task 3.3 one-time mode (auto-watch → auto-sweep → retire → optional purge).
  one_time?: boolean;
  sweep_destination_address?: string | null;
  min_sweep_amount_hex?: string | null;
  purge_after_sweep?: boolean;
  sweep_job_id?: string | null;
  // Read-time derivation: watching | sweep_queued | swept | retired.
  lifecycle_state?: string | null;
  // Why a watching allocation has not swept yet (see docs/architecture.md).
  sweep_blocker?: string | null;
}

export interface Counterparty {
  id: string;
  name: string;
  note?: string | null;
  sweep_destination_address?: string | null;
  created_at_unix: number;
}

export interface ReceivingItem {
  source_type: string;
  address: string;
  chain_id: number;
  chain_id_assumed?: boolean;
  derivation_path?: string | null;
  purpose?: string | null;
  label?: string | null;
  counterparty_id?: string | null;
  linkage_warning?: string | null;
  balance_native_wei_hex?: string | null;
  balance_known: boolean;
  balance_last_checked_at_unix?: number | null;
  status: string;
  created_at_unix: number;
}

export interface ReceivingPartyGroup {
  counterparty?: Counterparty | null;
  item_count: number;
  native_total_wei_hex: string;
  items: ReceivingItem[];
}

export interface ReceivingTotals {
  item_count: number;
  hd_count: number;
  stealth_count: number;
  native_total_wei_hex: string;
}

export interface ReceivingCoverage {
  addresses_total: number;
  addresses_with_known_balance: number;
  note: string;
}

export interface ReceivingOverviewResponse {
  generated_at_unix: number;
  include_retired: boolean;
  groups: ReceivingPartyGroup[];
  totals: ReceivingTotals;
  coverage: ReceivingCoverage;
}

export interface ReceivingRefreshResponse {
  generated_at_unix: number;
  addresses_requested: number;
  addresses_refreshed: number;
  addresses_skipped: number;
  stealth_refreshed: boolean;
  provider_status: string;
  errors: string[];
}

export interface ReceivingDepositTagRequest {
  deposit_id: string;
  counterparty_id?: string | null;
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

export interface EthStealthAnnouncementPayload {
  announcer_address: string;
  announce_function: string;
  scheme_id: number;
  stealth_address: string;
  ephemeral_public_key_hex: string;
  metadata_hex: string;
  calldata_hex: string;
  value_wei_hex: string;
}

export interface EthStealthGenerateResponse {
  short_name: string;
  scheme_id: number;
  stealth_meta_address: string;
  stealth_address: string;
  ephemeral_public_key_hex: string;
  view_tag_hex: string;
  announcement?: EthStealthAnnouncementPayload | null;
  /**
   * Non-blocking cautionary warnings (e.g. foreign meta-address, ephemeral
   * key reuse). Empty when nothing suspicious was detected.
   */
  warnings: string[];
}

export interface ApiRequestOptions<TBody = unknown> {
  method: "GET" | "POST" | "DELETE";
  path: string;
  body?: TBody;
  sessionToken?: string | null;
}

// ── List pagination / filtering / sorting (plan task 1.5) ────────────
// Query-string shapes for the six list endpoints (queue jobs, inventory
// wallets, deposits, consolidation plans, risk findings, discovery jobs).
// A parameterless request keeps the legacy response: full list in store
// order and no `pagination` key. Unknown enum values are rejected by the
// daemon with 400 `validation_failed` naming the parameter.

/** Offset pagination window shared by all paginated list endpoints. */
export interface PaginationQuery {
  limit?: number;
  offset?: number;
}

/**
 * Pagination metadata on list responses. Present only when the request
 * supplied `limit` and/or `offset`; legacy parameterless responses have no
 * `pagination` key. `total` counts items after filtering, before the
 * window; `has_more` is `offset + returned length < total`.
 */
export interface PaginationInfo {
  total: number;
  limit: number;
  offset: number;
  has_more: boolean;
}

/**
 * Sort direction for list endpoints. When `sort` is given without `order`
 * the daemon defaults to `desc` for time/severity fields and `asc` for
 * `address`. `order` requires `sort`.
 */
export type ListSortOrder = "asc" | "desc";

export interface QueueJobListQuery extends PaginationQuery {
  state?:
    | "queued"
    | "blocked"
    | "retrying"
    | "prepared"
    | "submitted_unknown"
    | "sent"
    | "confirmed"
    | "failed_terminal"
    | "operator_action_required"
    | "deferred"
    | "failed";
  kind?:
    | "eth_stealth_transfer"
    | "eth_stealth_erc20_transfer"
    | "eth_stealth_native_sweep"
    | "eth_stealth_erc20_sweep"
    | "eth_seed_transfer"
    | "eth_seed_native_sweep"
    | "eth_seed_erc20_sweep"
    | "plan_step_execution";
  /** Matches only payloads that carry a chain id (`plan_step_execution`). */
  chain_id?: number;
  sort?: "created" | "updated";
  order?: ListSortOrder;
}

export interface WalletInventoryListQuery extends PaginationQuery {
  chain_id?: number;
  /** true = non-zero native balance (`activity_state == "funded"`). */
  funded?: boolean;
  sort?: "address" | "last_scanned";
  order?: ListSortOrder;
}

export interface EthStealthDepositListQuery extends PaginationQuery {
  status?:
    | "pending"
    | "underfunded"
    | "funded_needs_gas"
    | "funded"
    | "sweep_queued"
    | "sweep_blocked"
    | "sweep_retrying"
    | "sweep_prepared"
    | "sweep_submitted_unknown"
    | "sweep_sent"
    | "sweep_confirmed"
    | "sweep_failed"
    | "sweep_operator_action_required";
  chain_id?: number;
  /** Exact match; free-form (not value-validated). */
  counterparty_id?: string;
  sort?: "created" | "updated";
  order?: ListSortOrder;
}

export interface ConsolidationPlanListQuery extends PaginationQuery {
  status?: "empty" | "blocked" | "review_required" | "approved";
  sort?: "created" | "updated";
  order?: ListSortOrder;
}

export interface RiskFindingListQuery extends PaginationQuery {
  /** Exact match on the finding's `risk_level`. */
  severity?: "critical" | "high" | "medium" | "low" | "trusted";
  /** Exact match on the finding's `category`; free-form (not validated). */
  kind?: string;
  chain_id?: number;
  sort?: "severity" | "found_at";
  order?: ListSortOrder;
}

export interface DiscoveryJobListQuery extends PaginationQuery {
  state?: "running" | "completed" | "canceled" | "failed" | "resume_requested";
  sort?: "created" | "updated";
  order?: ListSortOrder;
}

// ── Response envelopes & SSE DTOs (plan task 4.1, core/api + core/events) ──
// Mirrors sigillum-api response/{operations,queue,deposits,inventory,
// treasury}.rs, response.rs (audit/diagnostics), and response/events.rs.
// These complete the wire contract for the strict-typed core client; shapes
// are additive and never restate an existing interface above.

export interface OperationListResponse {
  operations: Operation[];
}

export interface OperationResponse {
  operation: Operation;
}

/** `POST /api/operations/{id}/cancel` — status mirrors the post-request state. */
export interface OperationMutationResponse {
  status: string;
  operation: Operation;
}

// ── Queue ────────────────────────────────────────────────────────────

export type QueueJobKind = WireString<
  | "eth_stealth_transfer"
  | "eth_stealth_erc20_transfer"
  | "eth_stealth_native_sweep"
  | "eth_stealth_erc20_sweep"
  | "eth_stealth_gas_topup"
  | "eth_seed_transfer"
  | "eth_seed_native_sweep"
  | "eth_seed_erc20_sweep"
  | "plan_step_execution"
>;

/**
 * Queue job record (`GET /api/queue/jobs`, enqueue/process responses).
 * The Rust wire shape flattens a `kind`-tagged payload enum plus the W7.4
 * receipt fields into one object; which optional fields are present depends
 * on `kind` (e.g. `stealth_address` on stealth jobs, `plan_id`/`step_id` on
 * plan-step executions). Receipt fields are absent until a receipt exists.
 */
export interface QueueJob {
  id: string;
  kind: QueueJobKind;
  state: string;
  attempts: number;
  created_at_unix: number;
  updated_at_unix: number;
  next_attempt_after_unix?: number | null;
  last_error?: string | null;
  transaction_hash_hex?: string | null;
  broadcast_transaction_hash_hex?: string | null;
  // ── Flattened payload fields (subset; presence depends on `kind`) ──
  wallet_profile?: string;
  address?: string;
  stealth_address?: string;
  derivation_path?: string;
  token_address?: string;
  destination_address?: string | null;
  recipient_address?: string | null;
  sponsor_address?: string;
  value_wei_hex?: string;
  amount_hex?: string;
  min_value_wei_hex?: string | null;
  min_amount_hex?: string | null;
  plan_id?: string;
  step_id?: string;
  chain_id?: number;
  source_address?: string;
  action?: WalletPlanStepAction;
  asset_kind?: WalletAssetKind;
  // ── Flattened receipt fields (W7.4; all absent pre-broadcast) ──
  prepared_at_unix?: number | null;
  broadcast_at_unix?: number | null;
  confirmations?: number | null;
  receipt_block_number?: number | null;
  receipt_gas_used_hex?: string | null;
  /** `"success"` or `"reverted"` once a receipt is recorded. */
  receipt_status?: string | null;
}

export interface QueueJobListResponse {
  jobs: QueueJob[];
  pagination?: PaginationInfo | null;
}

export interface QueueEnqueueResponse {
  status: string;
  job: QueueJob;
}

export interface QueueExecutionPauseResponse {
  status: string;
  execution_paused: boolean;
}

export interface MaintenanceFailureBreakdown {
  provider_error: number;
  policy_block: number;
  insufficient_gas: number;
  validation: number;
  unknown: number;
  on_chain_revert: number;
  broadcast_rejected: number;
  receipt_timeout: number;
}

export interface QueueProcessResponse {
  processed: number;
  succeeded: number;
  blocked: number;
  retrying: number;
  operator_action_required: number;
  failed: number;
  confirmed: number;
  failures_by_cause?: MaintenanceFailureBreakdown;
  paused_reason?: string | null;
  jobs: QueueJob[];
  /** Present only for `run_async` requests; tallies are then zero. */
  operation?: Operation | null;
}

/** `POST /api/queue/process` request body (all fields optional). */
export interface QueueProcessRequest {
  id?: string;
  limit?: number;
  run_async?: boolean;
}

// ── Consolidation plans ──────────────────────────────────────────────

export interface ConsolidationPlanListResponse {
  plans: ConsolidationPlan[];
  pagination?: PaginationInfo | null;
}

// ── Stealth deposits ─────────────────────────────────────────────────

export interface EthStealthDeposit {
  id: string;
  status: string;
  asset_kind: string;
  wallet_profile: string;
  chain_id: number;
  chain_id_assumed?: boolean;
  wallet_compartment_id?: number;
  provider_compartment_id?: number;
  wallet: string;
  short_name: string;
  stealth_meta_address: string;
  stealth_address: string;
  ephemeral_public_key_hex: string;
  view_tag_hex: string;
  stealth_hash_convention?: string;
  announcement?: EthStealthAnnouncementPayload | null;
  token_address?: string | null;
  expected_amount_hex?: string | null;
  observed_amount_hex?: string | null;
  observed_native_balance_wei_hex?: string | null;
  auto_queue_sweep: boolean;
  sweep_destination_address?: string | null;
  min_sweep_amount_hex?: string | null;
  queue_job_id?: string | null;
  queue_job_state?: string | null;
  note?: string | null;
  created_at_unix: number;
  updated_at_unix: number;
  last_checked_at_unix?: number | null;
  broadcast_transaction_hash_hex?: string | null;
  counterparty_id?: string | null;
  requested_gas_wei_hex?: string | null;
  gas_topup_job_id?: string | null;
  gas_topup_job_state?: string | null;
}

export interface EthStealthDepositListResponse {
  deposits: EthStealthDeposit[];
  pagination?: PaginationInfo | null;
}

// ── Wallet inventory ─────────────────────────────────────────────────

export interface WalletAssetHolding {
  id: string;
  wallet_family: string;
  wallet_profile: string;
  provider_profile: string;
  chain_id: number;
  address: string;
  derivation_path: string;
  asset_kind: WalletAssetKind;
  asset_address?: string | null;
  token_id_hex?: string | null;
  counterparty_address?: string | null;
  protocol_address?: string | null;
  claim_adapter?: string | null;
  claim_index_hex?: string | null;
  claim_proof?: string[];
  metadata_uri?: string | null;
  metadata_name?: string | null;
  spam_label?: string | null;
  amount_hex: string;
  source: string;
  status: string;
  first_seen_at_unix: number;
  last_checked_at_unix: number;
}

export interface WalletInventoryListResponse {
  jobs: WalletDiscoveryJob[];
  addresses: WalletInventoryAddress[];
  holdings: WalletAssetHolding[];
  nft_metadata_cache?: NftMetadataCacheEntry[];
  /** Window metadata for `addresses` only; other lists are always full. */
  pagination?: PaginationInfo | null;
}

// ── Treasury policy ──────────────────────────────────────────────────

/** Current treasury policy; `policy` is null until one is configured. */
export interface TreasuryPolicyResponse {
  policy: TreasuryPolicy | null;
}

export interface TreasuryPolicyMutationResponse {
  status: string;
  policy: TreasuryPolicy;
}

// ── Audit ────────────────────────────────────────────────────────────

export interface AuditEvent {
  created_at_unix: number;
  kind: string;
  compartment_id?: number | null;
  details?: Record<string, unknown>;
}

export interface AuditResponse {
  events: AuditEvent[];
}

/** `GET /api/audit` query params (`tail` and `limit` are aliases). */
export interface AuditListQuery {
  tail?: number;
  limit?: number;
  kind?: string;
  since?: number;
  key?: string;
}

// ── Diagnostics ──────────────────────────────────────────────────────

export interface RuntimePolicyResponse {
  queue_default_process_limit: number;
  queue_max_process_limit: number;
  deposit_default_refresh_limit: number;
  deposit_max_refresh_limit: number;
  audit_default_limit: number;
  audit_max_limit: number;
  queue_retry_base_delay_secs: number;
  queue_retry_max_delay_secs: number;
  provider_balance_observation_concurrency: number;
  receiving_refresh_address_cap: number;
  idle_lock_secs: number;
  idle_lock_drain_secs: number;
  idle_lock_force_after_secs: number;
}

export interface SchedulerStatusResponse {
  enabled: boolean;
  queue_tick_secs: number;
  refresh_secs: number;
  last_tick_at_unix?: number | null;
  /** `advanced | idle | skipped_locked | skipped_guard_busy | failed`. */
  last_cycle_outcome?: string | null;
  consecutive_failures: number;
  due_queue_job_count: number;
  next_retry_at_unix?: number | null;
}

export interface DiagnosticsResponse {
  status: string;
  version: string;
  unlock_scope: string;
  session_scope: string;
  started_at_unix: number;
  initialized: boolean;
  unlocked_compartment_count: number;
  active_session_count: number;
  default_active_compartment_id?: number | null;
  max_unlocked_threshold?: number | null;
  audit_log_present: boolean;
  pending_operation_count: number;
  queue_job_count: number;
  blocked_queue_job_count: number;
  retrying_queue_job_count: number;
  failed_queue_job_count: number;
  operator_action_required_queue_job_count: number;
  deferred_queue_job_count: number;
  startup_interrupted_operation_count: number;
  startup_recovered_operation_count: number;
  startup_unresolved_operation_count: number;
  startup_recovered_queue_job_count: number;
  startup_reconciled_deposit_count: number;
  runtime_policy: RuntimePolicyResponse;
  eth_stealth_deposit_count: number;
  funded_eth_stealth_deposit_count: number;
  scheduler: SchedulerStatusResponse;
}

// ── SSE channel (GET /api/events; mirrors sigillum-api response/events.rs) ──
// Wire framing: the SSE `event:` field carries the name, `data:` one JSON
// payload. Every payload carries `v: EVENTS_PROTOCOL_VERSION`. Within 1.x
// the daemon may add event names and optional fields; clients MUST ignore
// unknown names and unknown fields (see parseDaemonEvent in core/events.ts).

export const EVENTS_PROTOCOL_VERSION = 1;

export const EVENT_NAME_SNAPSHOT = "snapshot";
export const EVENT_NAME_OPERATION = "operation";
export const EVENT_NAME_QUEUE = "queue";
export const EVENT_NAME_STATUS = "status";

export const STATUS_EVENT_LOCKED = "locked";
export const STATUS_EVENT_UNLOCKED = "unlocked";
export const STATUS_EVENT_COMPARTMENT_SWITCHED = "compartment_switched";

/** `operation` event: the full operation record after the transition. */
export interface OperationEvent {
  v: number;
  operation: Operation;
}

/** `queue` event: job id + new state (`last_error` when one was recorded). */
export interface QueueJobEvent {
  v: number;
  job_id: string;
  state: string;
  last_error?: string | null;
}

/** `status` event: lock state or active-compartment changes. */
export interface StatusEvent {
  v: number;
  /** `locked`, `unlocked`, or `compartment_switched`. */
  kind: string;
  active_compartment_id?: number | null;
}

/**
 * `snapshot` event: first frame on every connection, and the resync frame
 * after a lagging subscriber misses events. Queue state is NOT included
 * (durable; list via `GET /api/queue/jobs`).
 */
export interface EventsSnapshot {
  v: number;
  locked: boolean;
  active_compartment_id?: number | null;
  operations: Operation[];
}
