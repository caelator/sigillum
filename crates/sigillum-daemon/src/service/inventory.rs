//! Wallet inventory and read-only discovery operations.

use std::collections::BTreeMap;

mod allowance_discovery;
mod chain_profiles;
mod checkpoints;
mod claim_discovery;
mod claim_gate;
mod consolidation;
mod defi_adapters;
mod defi_discovery;
mod defi_exit_planning;
mod discovery_jobs;
mod export;
mod gas_topup;
mod nft_approval_discovery;
mod nft_discovery;
mod nft_metadata;
mod observation;
mod permit2_discovery;
mod plan_execution_enqueue;
pub(in crate::service) mod planner;
mod preflight;
mod risk;
mod risk_catalog;
mod simulation;
mod support;
mod token_discovery;
mod token_registry;
mod treasury;
mod wallet_selection;
mod watch_book;
mod watch_discovery;

use sigillum_api::{
    ChainProfile, DEFAULT_DORMANCY_BLOCK_WINDOW, EthSeedWalletProfile, EvmProviderProfile,
    OPERATION_KIND_INVENTORY_SCAN_EVM, OPERATION_STATE_CANCELED, OPERATION_STATE_COMPLETED,
    OPERATION_STATE_FAILED, RiskFindingListResponse, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress, WalletInventoryListResponse, WalletInventoryScanRequest,
    WalletInventoryScanResponse, WatchAddressProbe,
};
use sigillum_core::derive_ethereum_address_from_control_xpub;

use crate::audit_log::AuditEventSpec;
use crate::operation_registry::OperationHandle;

use allowance_discovery::{Erc20AllowanceDiscoveryConfig, erc20_allowance_discovery_config};
use checkpoints::{
    ScanCheckpointProgress, latest_block_scan_cursors, latest_resume_checkpoint,
    sync_inventory_job, update_scan_checkpoint,
};
use claim_discovery::{ClaimCandidateDiscoveryConfig, claim_candidate_discovery_config};
use defi_discovery::{DefiTokenPositionDiscoveryConfig, defi_token_position_discovery_config};
use nft_approval_discovery::{
    NftOperatorApprovalDiscoveryConfig, nft_operator_approval_discovery_config,
};
use nft_discovery::{
    Erc721TransferDiscoveryConfig, Erc1155TransferDiscoveryConfig,
    erc721_transfer_discovery_config, erc1155_transfer_discovery_config,
};
use observation::AddressActivityContext;
use permit2_discovery::permit2_allowance_discovery_config;
use risk::derive_inventory_risk_findings;
use support::{
    announcement_activity_blocks, load_inventory_state, normalized_wallet_family,
    record_inventory_observation, save_inventory_state, select_providers, unique_strings,
    unique_u64s, validated_gap_limit, validated_max_index,
};
use token_discovery::{Erc20TransferDiscoveryConfig, erc20_transfer_discovery_config};
use token_registry::{TokenRegistryProbeConfig, token_registry_probe_config};
use wallet_selection::{
    DERIVATION_PATTERN_PROJECT, DiscoveryWallet, SeedDerivationPattern,
    derive_discovery_wallet_address, scan_account_limit, select_discovery_wallets,
};
use watch_discovery::{WatchDiscoveryAddress, select_watch_addresses};

// W7.3 plan-step execution (service/queue/plan_steps.rs) reuses the
// inventory domain's evidence-hash re-verification and simulation-time gas
// limit assumptions so execution never diverges from what was validated at
// enqueue/simulation time.
pub(in crate::service) use plan_execution_enqueue::verify_plan_step_execution_evidence;
pub(in crate::service) use simulation::zero_value_transaction_gas_limit;

use super::chains::chain_profile_for_id;
use super::evm::normalize_address;
use super::helpers::{map_xpub_error, now_unix, random_id};
use super::list_query;
use super::{ServiceError, ServiceResult, SigillumService};

pub(in crate::service) const WALLET_FAMILY_ETH_SEED: &str = "eth-seed";
const WALLET_FAMILY_ETH_XPUB: &str = "eth-xpub";
const WALLET_FAMILY_ETH_WATCH: &str = "eth-watch";
const DISCOVERY_SOURCE_LOCAL_RPC: &str = "local-rpc";
const DISCOVERY_SOURCE_OPERATOR: &str = "operator";
const DEFAULT_GAP_LIMIT: u32 = 20;
const MAX_GAP_LIMIT: u32 = 100;
const DEFAULT_MAX_INDEX: u32 = 200;
const MAX_SCAN_INDEX: u32 = 10_000;
const NO_DISCOVERY_WALLETS_ERROR: &str = "No matching discovery wallets found.";

struct TokenRegistryObservationProbe<'a> {
    wallet: &'a DiscoveryWallet,
    provider: &'a EvmProviderProfile,
    derivation_path: &'a str,
    block_tag: &'a str,
    config: Option<&'a TokenRegistryProbeConfig>,
    now: u64,
}

/// Everything the EVM scan loop needs, resolved and validated up front.
///
/// [`SigillumService::prepare_evm_scan`] builds this before any state
/// mutation (and before spawning for async scans) so invalid requests fail
/// synchronously and the sync/async paths share one pipeline definition.
struct PreparedEvmScan {
    gap_limit: u32,
    max_index: u32,
    block_tag: String,
    token_addresses: Vec<String>,
    token_discovery: Option<Erc20TransferDiscoveryConfig>,
    allowance_discovery: Option<Erc20AllowanceDiscoveryConfig>,
    nft_discovery: Option<Erc721TransferDiscoveryConfig>,
    erc1155_discovery: Option<Erc1155TransferDiscoveryConfig>,
    nft_operator_approval_discovery: Option<NftOperatorApprovalDiscoveryConfig>,
    defi_position_discovery: Option<DefiTokenPositionDiscoveryConfig>,
    claim_candidate_discovery: Option<ClaimCandidateDiscoveryConfig>,
    token_registry_probe: Option<TokenRegistryProbeConfig>,
    providers: Vec<EvmProviderProfile>,
    wallets: Vec<DiscoveryWallet>,
    watch_addresses: Vec<WatchDiscoveryAddress>,
    resume_from_latest_checkpoint: bool,
    discover_permit2_allowances: Option<bool>,
    permit2_contract_addresses: Vec<String>,
    permit2_spender_addresses: Vec<String>,
    permit2_allowance_limit: Option<usize>,
    seed_profiles: Vec<EthSeedWalletProfile>,
}

/// The discovery job snapshot returned when an async scan is accepted.
///
/// The record is persisted by the runner as its first action under the
/// operation guard; until then the operation registry is the live view of
/// the scan. `block_cursors` are filled in at execution time from the
/// persisted jobs of prior scans.
fn accepted_scan_job(
    prepared: &PreparedEvmScan,
    id: String,
    started_at_unix: u64,
) -> WalletDiscoveryJob {
    WalletDiscoveryJob {
        id,
        status: "running".into(),
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        wallet_families: unique_strings(
            prepared
                .wallets
                .iter()
                .map(|wallet| wallet.family.clone())
                .chain(
                    prepared
                        .watch_addresses
                        .iter()
                        .map(|watch| watch.wallet.family.clone()),
                ),
        ),
        wallet_profiles: unique_strings(
            prepared
                .wallets
                .iter()
                .map(|wallet| wallet.profile.clone())
                .chain(
                    prepared
                        .watch_addresses
                        .iter()
                        .map(|watch| watch.wallet.profile.clone()),
                ),
        ),
        provider_profiles: unique_strings(
            prepared
                .providers
                .iter()
                .map(|provider| provider.name.clone()),
        ),
        chain_ids: unique_u64s(prepared.providers.iter().map(|provider| provider.chain_id)),
        gap_limit: prepared.gap_limit,
        max_index: prepared.max_index,
        addresses_scanned: 0,
        active_addresses: 0,
        holdings_detected: 0,
        checkpoints: Vec::new(),
        block_cursors: Vec::new(),
        started_at_unix,
        completed_at_unix: None,
        last_error: None,
    }
}

impl SigillumService {
    pub(crate) fn list_wallet_inventory(
        &self,
        token: Option<&str>,
        query: list_query::WalletInventoryListQuery,
    ) -> ServiceResult<WalletInventoryListResponse> {
        let _ = self.require_session(token)?;
        let state =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        // Filters, sorts, and the pagination window apply to `addresses`
        // only; the sibling lists always return in full.
        let mut addresses = state.addresses;
        if let Some(chain_id) = query.chain_id {
            addresses.retain(|address| address.chain_id == chain_id);
        }
        if let Some(funded) = query.funded {
            addresses.retain(|address| {
                (address.activity_state == sigillum_api::WalletAddressActivityState::Funded)
                    == funded
            });
        }
        if let Some(sort) = query.sort {
            let order = list_query::effective_order(query.sort.as_ref(), query.order);
            match (sort, order) {
                (list_query::WalletInventorySort::Address, list_query::SortOrder::Asc) => {
                    addresses.sort_by(|a, b| a.address.cmp(&b.address));
                }
                (list_query::WalletInventorySort::Address, list_query::SortOrder::Desc) => {
                    addresses.sort_by(|a, b| b.address.cmp(&a.address));
                }
                (list_query::WalletInventorySort::LastScanned, list_query::SortOrder::Asc) => {
                    addresses.sort_by_key(|address| address.last_checked_at_unix);
                }
                (list_query::WalletInventorySort::LastScanned, list_query::SortOrder::Desc) => {
                    addresses.sort_by(|a, b| b.last_checked_at_unix.cmp(&a.last_checked_at_unix));
                }
            }
        }
        let (addresses, pagination) = list_query::paginate(addresses, query.page);
        Ok(WalletInventoryListResponse {
            jobs: state.jobs,
            addresses,
            holdings: state.holdings,
            nft_metadata_cache: state.nft_metadata_cache,
            pagination,
        })
    }

    pub(crate) fn list_risk_findings(
        &self,
        token: Option<&str>,
        query: list_query::RiskFindingListQuery,
    ) -> ServiceResult<RiskFindingListResponse> {
        let _ = self.require_session(token)?;
        let severity = query
            .severity
            .map(|value| {
                list_query::validated_value("severity", value, &list_query::RISK_SEVERITIES)
            })
            .transpose()?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let mut findings = state.risk_findings;
        findings.extend(derive_inventory_risk_findings(
            &state.addresses,
            &state.holdings,
            &state.risk_catalog,
            &state.chain_profiles,
        ));
        if let Some(severity) = severity.as_deref() {
            findings.retain(|finding| finding.risk_level == severity);
        }
        if let Some(kind) = query.kind.as_deref() {
            findings.retain(|finding| finding.category == kind);
        }
        if let Some(chain_id) = query.chain_id {
            findings.retain(|finding| finding.chain_id == chain_id);
        }
        if let Some(sort) = query.sort {
            let order = list_query::effective_order(query.sort.as_ref(), query.order);
            match (sort, order) {
                (list_query::RiskFindingSort::Severity, list_query::SortOrder::Asc) => {
                    findings.sort_by_key(|finding| list_query::severity_rank(&finding.risk_level));
                }
                (list_query::RiskFindingSort::Severity, list_query::SortOrder::Desc) => {
                    findings.sort_by(|a, b| {
                        list_query::severity_rank(&b.risk_level)
                            .cmp(&list_query::severity_rank(&a.risk_level))
                    });
                }
                (list_query::RiskFindingSort::FoundAt, list_query::SortOrder::Asc) => {
                    findings.sort_by_key(|finding| finding.first_seen_at_unix);
                }
                (list_query::RiskFindingSort::FoundAt, list_query::SortOrder::Desc) => {
                    findings.sort_by(|a, b| b.first_seen_at_unix.cmp(&a.first_seen_at_unix));
                }
            }
        }
        let (findings, pagination) = list_query::paginate(findings, query.page);
        Ok(RiskFindingListResponse {
            findings,
            pagination,
        })
    }

    async fn apply_token_registry_probe(
        &self,
        probe: TokenRegistryObservationProbe<'_>,
        observation: &mut support::InventoryAddressObservation,
    ) -> ServiceResult<()> {
        let Some(config) = probe.config else {
            return Ok(());
        };
        let holdings = self
            .probe_token_registry_for_address(
                probe.wallet,
                probe.provider,
                &observation.address.address,
                probe.derivation_path,
                probe.block_tag,
                config,
                &observation.holdings,
                probe.now,
            )
            .await?;
        observation.holdings.extend(holdings);
        Ok(())
    }

    /// Run an EVM discovery scan.
    ///
    /// Both the synchronous and `run_async` paths share one pipeline:
    /// [`Self::prepare_evm_scan`] performs all request validation and
    /// wallet/provider/watch-address resolution up front (so async
    /// submissions fail synchronously on bad input), and
    /// [`Self::execute_evm_scan`] drives the scan loop under the operation
    /// guard with per-index persistence and cooperative cancellation.
    pub(crate) async fn scan_wallet_inventory_evm(
        &self,
        token: Option<&str>,
        body: WalletInventoryScanRequest,
    ) -> ServiceResult<WalletInventoryScanResponse> {
        let token = self.require_session(token)?;
        let run_async = body.run_async == Some(true);
        let prepared = self.prepare_evm_scan(token, body)?;
        if run_async {
            let (job, operation) = self.spawn_async_evm_scan(token, prepared);
            return Ok(WalletInventoryScanResponse {
                job,
                addresses: Vec::new(),
                holdings: Vec::new(),
                operation: Some(operation),
            });
        }
        // Synchronous path: identical behavior to the historical endpoint,
        // including the response contract (no `operation` field). The scan
        // is still registered as an operation so other clients can observe
        // or cancel it mid-run.
        let operation = self
            .state
            .start_operation(OPERATION_KIND_INVENTORY_SCAN_EVM, Vec::new());
        self.execute_evm_scan(token, prepared, operation, None)
            .await
    }

    /// Validate a scan request and resolve everything the scan loop needs.
    ///
    /// Runs before the operation guard (and before spawning for async
    /// scans) so invalid requests fail fast. The persisted watch address
    /// book is read without the guard — the same read-only access the list
    /// endpoints use; discovery is read-only, so a concurrent watch-book
    /// change before execution is harmless.
    fn prepare_evm_scan(
        &self,
        token: &str,
        body: WalletInventoryScanRequest,
    ) -> ServiceResult<PreparedEvmScan> {
        let gap_limit = validated_gap_limit(body.gap_limit)?;
        let max_index = validated_max_index(body.max_index)?;
        let block_tag = body
            .block_tag
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("latest")
            .to_string();
        let token_addresses = body
            .token_addresses
            .iter()
            .map(|address| normalize_address(address))
            .collect::<ServiceResult<Vec<_>>>()?;
        let token_discovery = erc20_transfer_discovery_config(
            body.discover_erc20_transfers,
            body.token_discovery_from_block.as_deref(),
            body.token_discovery_to_block.as_deref(),
            body.token_discovery_limit,
        )?;
        let allowance_discovery = erc20_allowance_discovery_config(
            body.discover_erc20_allowances,
            &body.allowance_spender_addresses,
            body.allowance_discovery_limit,
        )?;
        let nft_discovery = erc721_transfer_discovery_config(
            body.discover_erc721_transfers,
            body.nft_discovery_from_block.as_deref(),
            body.nft_discovery_to_block.as_deref(),
            body.nft_discovery_limit,
        )?;
        let erc1155_discovery = erc1155_transfer_discovery_config(
            body.discover_erc1155_transfers,
            body.nft_discovery_from_block.as_deref(),
            body.nft_discovery_to_block.as_deref(),
            body.nft_discovery_limit,
        )?;
        let nft_operator_approval_discovery = nft_operator_approval_discovery_config(
            body.discover_nft_operator_approvals,
            &body.nft_operator_addresses,
            body.nft_operator_approval_limit,
        )?;
        let defi_position_discovery = defi_token_position_discovery_config(
            body.discover_defi_token_positions,
            &body.defi_token_probes,
            body.defi_position_limit,
        )?;
        let claim_candidate_discovery = claim_candidate_discovery_config(
            body.discover_claim_candidates,
            &body.claim_candidate_probes,
            body.claim_candidate_limit,
        )?;
        let token_registry_probe = if body.probe_token_registry == Some(true) {
            let compartment_id = self
                .state
                .active_compartment_id_for(token)
                .ok_or_else(|| ServiceError::vault_locked("No active compartment."))?;
            let state = crate::token_registry::load_token_registry(&self.state.base_dir).map_err(
                |error| ServiceError::internal(format!("Failed to load token registry: {error}")),
            )?;
            let lists: Vec<_> = state
                .lists
                .into_iter()
                .filter(|list| list.compartment_id == compartment_id)
                .collect();
            token_registry_probe_config(body.probe_token_registry, &lists)?
        } else {
            None
        };
        let requested_family = normalized_wallet_family(body.wallet_family.as_deref())?;
        let seed_derivation_pattern =
            SeedDerivationPattern::parse(body.derivation_pattern.as_deref())?;
        let account_limit = scan_account_limit(body.account_limit)?;

        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        if body.all_configured_chains == Some(true)
            && body
                .provider_profile
                .as_deref()
                .is_some_and(|profile| !profile.trim().is_empty())
        {
            return Err(ServiceError::bad_request(
                "provider_profile cannot be combined with all_configured_chains",
            ));
        }
        let requested_provider_profile = if body.all_configured_chains == Some(true) {
            None
        } else {
            body.provider_profile
                .as_deref()
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
        };
        let providers = select_providers(&registry.evm_providers, requested_provider_profile)?;
        let wallets = select_discovery_wallets(
            self,
            &registry.eth_seed_wallets,
            &registry.eth_xpub_wallets,
            requested_family.as_deref(),
            body.wallet_profile.as_deref(),
            seed_derivation_pattern,
            account_limit,
        )?;
        let mut watch_probes = body.watch_addresses.clone();
        if body.include_watch_book.unwrap_or(false) {
            let inventory = load_inventory_state(&self.state.base_dir)?;
            watch_probes.extend(
                inventory
                    .watch_address_book
                    .iter()
                    .filter(|entry| entry.enabled)
                    .map(|entry| WatchAddressProbe {
                        address: entry.address.clone(),
                        label: Some(entry.label.clone()),
                    }),
            );
        }
        let watch_addresses = select_watch_addresses(
            &watch_probes,
            requested_family.as_deref(),
            body.wallet_profile.as_deref(),
        )?;
        if wallets.is_empty() && watch_addresses.is_empty() {
            return Err(ServiceError::not_found(NO_DISCOVERY_WALLETS_ERROR));
        }
        Ok(PreparedEvmScan {
            gap_limit,
            max_index,
            block_tag,
            token_addresses,
            token_discovery,
            allowance_discovery,
            nft_discovery,
            erc1155_discovery,
            nft_operator_approval_discovery,
            defi_position_discovery,
            claim_candidate_discovery,
            token_registry_probe,
            providers,
            wallets,
            watch_addresses,
            resume_from_latest_checkpoint: body.resume_from_latest_checkpoint.unwrap_or(false),
            discover_permit2_allowances: body.discover_permit2_allowances,
            permit2_contract_addresses: body.permit2_contract_addresses,
            permit2_spender_addresses: body.permit2_spender_addresses,
            permit2_allowance_limit: body.permit2_allowance_limit,
            seed_profiles: registry.eth_seed_wallets,
        })
    }

    /// Spawn a prepared scan as a background daemon operation, returning the
    /// accepted discovery job snapshot (pre-generated id; the record itself
    /// is persisted by the runner as its first action under the operation
    /// guard) and the operation tracking it.
    fn spawn_async_evm_scan(
        &self,
        token: &str,
        prepared: PreparedEvmScan,
    ) -> (WalletDiscoveryJob, sigillum_api::Operation) {
        let job_id = random_id();
        let accepted_job = accepted_scan_job(&prepared, job_id.clone(), now_unix());
        let accepted_job_id = accepted_job.id.clone();
        let operation = self
            .state
            .start_operation(OPERATION_KIND_INVENTORY_SCAN_EVM, vec![job_id]);
        let operation_id = operation.id().to_string();
        let service = self.clone();
        let token = token.to_string();
        tokio::spawn(async move {
            if let Err(error) = service
                .execute_evm_scan(&token, prepared, operation, Some(accepted_job_id))
                .await
            {
                tracing::warn!(error = %error, "async inventory scan failed");
            }
        });
        let operation = self
            .state
            .get_operation(&operation_id)
            .expect("operation registered above");
        (accepted_job, operation)
    }

    /// Execute a prepared scan under the operation guard.
    ///
    /// The guard is held for the whole run exactly like the historical
    /// synchronous path, so mutation-serialization semantics are unchanged.
    /// Cancellation is cooperative: the loop checks the operation's cancel
    /// flag at every address index and, when set, stops before the next
    /// index, persists state exactly like the per-index saves, and marks the
    /// job and operation `canceled`. Mid-run errors persist the job as
    /// `failed` with `last_error` so a later resume can continue from its
    /// checkpoints.
    async fn execute_evm_scan(
        &self,
        token: &str,
        prepared: PreparedEvmScan,
        operation: OperationHandle,
        preset_job_id: Option<String>,
    ) -> ServiceResult<WalletInventoryScanResponse> {
        let _guard = self.state.operation_guard().await;
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let chain_profiles = inventory.chain_profiles.clone();
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map(|deposits| deposits.eth_stealth)
            .unwrap_or_default();
        let announcement_activity = announcement_activity_blocks(&deposits);
        let mut chain_tip_blocks = BTreeMap::new();
        for provider in &prepared.providers {
            let tip = match self.provider_rpc_for_profile(provider.compartment_id, provider) {
                Ok(rpc) => rpc.get_block_number().await.ok(),
                Err(_) => None,
            };
            chain_tip_blocks.insert(provider.name.clone(), tip);
        }
        let permit2_allowance_discovery_for_provider =
            |provider: &sigillum_api::EvmProviderProfile| {
                permit2_allowance_discovery_config(
                    prepared.discover_permit2_allowances,
                    &prepared.permit2_contract_addresses,
                    &prepared.permit2_spender_addresses,
                    prepared.permit2_allowance_limit,
                    chain_profile_for_id(&chain_profiles, provider.chain_id)
                        .and_then(|profile| profile.permit2_address.as_deref()),
                )
            };

        let started_at_unix = now_unix();
        let mut job = WalletDiscoveryJob {
            block_cursors: latest_block_scan_cursors(
                &inventory.jobs,
                prepared.providers.iter().map(|provider| provider.chain_id),
            ),
            ..accepted_scan_job(
                &prepared,
                preset_job_id.unwrap_or_else(random_id),
                started_at_unix,
            )
        };
        inventory.jobs.push(job.clone());
        save_inventory_state(&self.state.base_dir, &inventory)?;
        self.state
            .operation_add_related(operation.id(), job.id.clone());
        self.state
            .operation_set_progress(operation.id(), job.addresses_scanned as u64);

        let mut scanned_addresses: Vec<WalletInventoryAddress> = Vec::new();
        let mut detected_holdings: Vec<WalletAssetHolding> = Vec::new();
        // A cancel that raced the submission (before the runner persisted
        // the job) is honored before any provider call happens.
        let mut canceled = operation.cancellation_requested();

        // The fallible scan loop runs as one async block so mid-run errors
        // can be finalized below (job and operation marked `failed`) instead
        // of leaking a permanently `running` record.
        let loop_result: ServiceResult<()> = async {
            for wallet in &prepared.wallets {
                let (mut index, mut empty_run) = if prepared.resume_from_latest_checkpoint {
                    latest_resume_checkpoint(&inventory.jobs, wallet, &prepared.providers)
                        .unwrap_or((0, 0))
                } else {
                    (0, 0)
                };
                while index <= prepared.max_index && empty_run < prepared.gap_limit {
                    // Cooperative cancel checkpoint: at least once per
                    // address index, before any provider call for it.
                    if operation.cancellation_requested() {
                        canceled = true;
                        break;
                    }
                    let derived =
                        derive_discovery_wallet_address(wallet, index).map_err(map_xpub_error)?;
                    let derivation_path = format!("{}/{index}", wallet.receive_path);
                    let mut index_has_activity = false;

                    for provider in &prepared.providers {
                        let permit2_allowance_discovery =
                            permit2_allowance_discovery_for_provider(provider)?;
                        let mut observation = self
                            .observe_inventory_address(
                                wallet,
                                provider,
                                &derived.address,
                                &derivation_path,
                                index,
                                &prepared.block_tag,
                                &prepared.token_addresses,
                                prepared.token_discovery.as_ref(),
                                prepared.allowance_discovery.as_ref(),
                                permit2_allowance_discovery.as_ref(),
                                prepared.nft_discovery.as_ref(),
                                prepared.erc1155_discovery.as_ref(),
                                prepared.nft_operator_approval_discovery.as_ref(),
                                prepared.defi_position_discovery.as_ref(),
                                prepared.claim_candidate_discovery.as_ref(),
                                activity_context_for_observation(
                                    &inventory,
                                    &chain_profiles,
                                    &announcement_activity,
                                    &chain_tip_blocks,
                                    wallet,
                                    provider,
                                    &derived.address,
                                ),
                                &mut job.block_cursors,
                                started_at_unix,
                            )
                            .await?;
                        self.apply_token_registry_probe(
                            TokenRegistryObservationProbe {
                                wallet,
                                provider,
                                derivation_path: &derivation_path,
                                block_tag: &prepared.block_tag,
                                config: prepared.token_registry_probe.as_ref(),
                                now: started_at_unix,
                            },
                            &mut observation,
                        )
                        .await?;
                        if observation.address.activity_state
                            != sigillum_api::WalletAddressActivityState::Empty
                        {
                            index_has_activity = true;
                        }
                        record_inventory_observation(
                            &mut job,
                            &mut inventory,
                            observation,
                            &mut detected_holdings,
                            &mut scanned_addresses,
                        );
                    }

                    if index_has_activity {
                        empty_run = 0;
                    } else {
                        empty_run += 1;
                    }
                    for provider in &prepared.providers {
                        update_scan_checkpoint(
                            &mut job.checkpoints,
                            wallet,
                            provider,
                            ScanCheckpointProgress {
                                next_index: index.saturating_add(1),
                                last_scanned_index: Some(index),
                                consecutive_empty: empty_run,
                                completed: false,
                                updated_at_unix: now_unix(),
                            },
                        );
                    }
                    sync_inventory_job(&mut inventory, &job);
                    save_inventory_state(&self.state.base_dir, &inventory)?;
                    self.state
                        .operation_set_progress(operation.id(), job.addresses_scanned as u64);
                    index += 1;
                }
                if canceled {
                    break;
                }
                for provider in &prepared.providers {
                    update_scan_checkpoint(
                        &mut job.checkpoints,
                        wallet,
                        provider,
                        ScanCheckpointProgress {
                            next_index: index,
                            last_scanned_index: index.checked_sub(1),
                            consecutive_empty: empty_run,
                            completed: true,
                            updated_at_unix: now_unix(),
                        },
                    );
                }
                sync_inventory_job(&mut inventory, &job);
                save_inventory_state(&self.state.base_dir, &inventory)?;

                if wallet.family == WALLET_FAMILY_ETH_SEED
                    && wallet.derivation_pattern == DERIVATION_PATTERN_PROJECT
                {
                    if let Some(seed_profile) = prepared
                        .seed_profiles
                        .iter()
                        .find(|p| p.name == wallet.profile)
                    {
                        if let Some(control_xpub) = &seed_profile.control_xpub {
                            let control_path =
                                format!("m/44'/60'/{}'/1", seed_profile.project_account);
                            for control_index in 0..=2 {
                                if operation.cancellation_requested() {
                                    canceled = true;
                                    break;
                                }
                                let derived = derive_ethereum_address_from_control_xpub(
                                    control_xpub,
                                    control_index,
                                )
                                .map_err(map_xpub_error)?;
                                let derivation_path = format!("{control_path}/{control_index}");
                                for provider in &prepared.providers {
                                    let permit2_allowance_discovery =
                                        permit2_allowance_discovery_for_provider(provider)?;
                                    let mut observation = self
                                        .observe_inventory_address(
                                            wallet,
                                            provider,
                                            &derived.address,
                                            &derivation_path,
                                            control_index,
                                            &prepared.block_tag,
                                            &prepared.token_addresses,
                                            prepared.token_discovery.as_ref(),
                                            prepared.allowance_discovery.as_ref(),
                                            permit2_allowance_discovery.as_ref(),
                                            prepared.nft_discovery.as_ref(),
                                            prepared.erc1155_discovery.as_ref(),
                                            prepared.nft_operator_approval_discovery.as_ref(),
                                            prepared.defi_position_discovery.as_ref(),
                                            prepared.claim_candidate_discovery.as_ref(),
                                            activity_context_for_observation(
                                                &inventory,
                                                &chain_profiles,
                                                &announcement_activity,
                                                &chain_tip_blocks,
                                                wallet,
                                                provider,
                                                &derived.address,
                                            ),
                                            &mut job.block_cursors,
                                            started_at_unix,
                                        )
                                        .await?;
                                    self.apply_token_registry_probe(
                                        TokenRegistryObservationProbe {
                                            wallet,
                                            provider,
                                            derivation_path: &derivation_path,
                                            block_tag: &prepared.block_tag,
                                            config: prepared.token_registry_probe.as_ref(),
                                            now: started_at_unix,
                                        },
                                        &mut observation,
                                    )
                                    .await?;
                                    record_inventory_observation(
                                        &mut job,
                                        &mut inventory,
                                        observation,
                                        &mut detected_holdings,
                                        &mut scanned_addresses,
                                    );
                                    sync_inventory_job(&mut inventory, &job);
                                    save_inventory_state(&self.state.base_dir, &inventory)?;
                                }
                            }
                        }
                    }
                }
                if canceled {
                    break;
                }
            }

            for watch in &prepared.watch_addresses {
                if canceled {
                    break;
                }
                if operation.cancellation_requested() {
                    canceled = true;
                    break;
                }
                let derivation_path =
                    format!("{}/{}", watch.wallet.receive_path, watch.address_index);
                for provider in &prepared.providers {
                    let permit2_allowance_discovery =
                        permit2_allowance_discovery_for_provider(provider)?;
                    let mut observation = self
                        .observe_inventory_address(
                            &watch.wallet,
                            provider,
                            &watch.address,
                            &derivation_path,
                            watch.address_index,
                            &prepared.block_tag,
                            &prepared.token_addresses,
                            prepared.token_discovery.as_ref(),
                            prepared.allowance_discovery.as_ref(),
                            permit2_allowance_discovery.as_ref(),
                            prepared.nft_discovery.as_ref(),
                            prepared.erc1155_discovery.as_ref(),
                            prepared.nft_operator_approval_discovery.as_ref(),
                            prepared.defi_position_discovery.as_ref(),
                            prepared.claim_candidate_discovery.as_ref(),
                            activity_context_for_observation(
                                &inventory,
                                &chain_profiles,
                                &announcement_activity,
                                &chain_tip_blocks,
                                &watch.wallet,
                                provider,
                                &watch.address,
                            ),
                            &mut job.block_cursors,
                            started_at_unix,
                        )
                        .await?;
                    self.apply_token_registry_probe(
                        TokenRegistryObservationProbe {
                            wallet: &watch.wallet,
                            provider,
                            derivation_path: &derivation_path,
                            block_tag: &prepared.block_tag,
                            config: prepared.token_registry_probe.as_ref(),
                            now: started_at_unix,
                        },
                        &mut observation,
                    )
                    .await?;
                    record_inventory_observation(
                        &mut job,
                        &mut inventory,
                        observation,
                        &mut detected_holdings,
                        &mut scanned_addresses,
                    );
                    sync_inventory_job(&mut inventory, &job);
                    save_inventory_state(&self.state.base_dir, &inventory)?;
                }
            }
            Ok(())
        }
        .await;

        if let Err(error) = loop_result {
            self.finalize_scan_job(&mut inventory, &mut job, "failed", Some(&error))?;
            self.state
                .operation_set_progress(operation.id(), job.addresses_scanned as u64);
            self.state.finish_operation(
                operation.id(),
                OPERATION_STATE_FAILED,
                Some(error.message().to_string()),
            );
            return Err(error);
        }

        if canceled {
            self.finalize_scan_job(&mut inventory, &mut job, "canceled", None)?;
            self.state
                .operation_set_progress(operation.id(), job.addresses_scanned as u64);
            self.state
                .finish_operation(operation.id(), OPERATION_STATE_CANCELED, None);
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                    id: job.id.clone(),
                    status: "canceled".into(),
                },
            )?;
            return Ok(WalletInventoryScanResponse {
                job,
                addresses: scanned_addresses,
                holdings: detected_holdings,
                operation: None,
            });
        }

        job.status = "completed".into();
        job.completed_at_unix = Some(now_unix());
        sync_inventory_job(&mut inventory, &job);
        save_inventory_state(&self.state.base_dir, &inventory)?;
        self.state
            .operation_set_progress(operation.id(), job.addresses_scanned as u64);
        self.state
            .finish_operation(operation.id(), OPERATION_STATE_COMPLETED, None);

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryScan {
                id: job.id.clone(),
                wallets: job.wallet_profiles.len(),
                providers: job.provider_profiles.len(),
                addresses: job.addresses_scanned,
                holdings: job.holdings_detected,
            },
        )?;

        Ok(WalletInventoryScanResponse {
            job,
            addresses: scanned_addresses,
            holdings: detected_holdings,
            operation: None,
        })
    }

    /// Persist a terminal scan-job state (`canceled` or `failed`) with the
    /// same per-index durability the loop uses.
    fn finalize_scan_job(
        &self,
        inventory: &mut crate::inventory::WalletInventoryState,
        job: &mut WalletDiscoveryJob,
        status: &str,
        error: Option<&ServiceError>,
    ) -> ServiceResult<()> {
        job.status = status.to_string();
        job.completed_at_unix = Some(now_unix());
        job.last_error = error.map(|error| error.message().to_string());
        sync_inventory_job(inventory, job);
        save_inventory_state(&self.state.base_dir, inventory)
    }
}

fn activity_context_for_observation(
    inventory: &crate::inventory::WalletInventoryState,
    chain_profiles: &[ChainProfile],
    announcement_activity: &BTreeMap<(u64, String), u64>,
    chain_tip_blocks: &BTreeMap<String, Option<u64>>,
    wallet: &DiscoveryWallet,
    provider: &sigillum_api::EvmProviderProfile,
    address: &str,
) -> AddressActivityContext {
    let prior_last_activity_block = inventory
        .addresses
        .iter()
        .find(|existing| {
            existing.wallet_family == wallet.family
                && existing.wallet_profile == wallet.profile
                && existing.provider_profile == provider.name
                && existing.chain_id == provider.chain_id
                && existing.address.eq_ignore_ascii_case(address)
        })
        .and_then(|existing| existing.last_activity_block);
    let announcement_activity_block = announcement_activity
        .get(&(provider.chain_id, address.to_ascii_lowercase()))
        .copied();
    let chain_tip_block = chain_tip_blocks.get(&provider.name).copied().flatten();
    let dormancy_block_window = chain_profile_for_id(chain_profiles, provider.chain_id)
        .map(|profile| profile.dormancy_block_window)
        .filter(|window| *window > 0)
        .unwrap_or(DEFAULT_DORMANCY_BLOCK_WINDOW);

    AddressActivityContext {
        prior_last_activity_block,
        announcement_activity_block,
        chain_tip_block,
        dormancy_block_window,
    }
}
