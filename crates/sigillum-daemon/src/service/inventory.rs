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
mod nft_approval_discovery;
mod nft_discovery;
mod nft_metadata;
mod observation;
mod permit2_discovery;
mod planner;
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
    ChainProfile, DEFAULT_DORMANCY_BLOCK_WINDOW, EvmProviderProfile, RiskFindingListResponse,
    WalletDiscoveryJob, WalletInventoryListResponse, WalletInventoryScanRequest,
    WalletInventoryScanResponse, WatchAddressProbe,
};
use sigillum_core::derive_ethereum_address_from_control_xpub;

use crate::audit_log::AuditEventSpec;

use allowance_discovery::erc20_allowance_discovery_config;
use checkpoints::{
    ScanCheckpointProgress, latest_block_scan_cursors, latest_resume_checkpoint,
    sync_inventory_job, update_scan_checkpoint,
};
use claim_discovery::claim_candidate_discovery_config;
use defi_discovery::defi_token_position_discovery_config;
use nft_approval_discovery::nft_operator_approval_discovery_config;
use nft_discovery::{erc721_transfer_discovery_config, erc1155_transfer_discovery_config};
use observation::AddressActivityContext;
use permit2_discovery::permit2_allowance_discovery_config;
use risk::derive_inventory_risk_findings;
use support::{
    announcement_activity_blocks, load_inventory_state, normalized_wallet_family,
    record_inventory_observation, save_inventory_state, select_providers, unique_strings,
    unique_u64s, validated_gap_limit, validated_max_index,
};
use token_discovery::erc20_transfer_discovery_config;
use token_registry::{TokenRegistryProbeConfig, token_registry_probe_config};
use wallet_selection::{
    DERIVATION_PATTERN_PROJECT, DiscoveryWallet, SeedDerivationPattern,
    derive_discovery_wallet_address, scan_account_limit, select_discovery_wallets,
};
use watch_discovery::select_watch_addresses;

use super::chains::chain_profile_for_id;
use super::evm::normalize_address;
use super::helpers::{map_xpub_error, now_unix, random_id};
use super::{ServiceError, ServiceResult, SigillumService};

const WALLET_FAMILY_ETH_SEED: &str = "eth-seed";
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

impl SigillumService {
    pub(crate) fn list_wallet_inventory(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<WalletInventoryListResponse> {
        let _ = self.require_session(token)?;
        let state =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        Ok(WalletInventoryListResponse {
            jobs: state.jobs,
            addresses: state.addresses,
            holdings: state.holdings,
            nft_metadata_cache: state.nft_metadata_cache,
        })
    }

    pub(crate) fn list_risk_findings(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<RiskFindingListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        let mut findings = state.risk_findings;
        findings.extend(derive_inventory_risk_findings(
            &state.addresses,
            &state.holdings,
            &state.risk_catalog,
            &state.chain_profiles,
        ));
        Ok(RiskFindingListResponse { findings })
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

    pub(crate) async fn scan_wallet_inventory_evm(
        &self,
        token: Option<&str>,
        body: WalletInventoryScanRequest,
    ) -> ServiceResult<WalletInventoryScanResponse> {
        let token = self.require_session(token)?;
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
                .ok_or_else(|| ServiceError::forbidden("No active compartment."))?;
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
        let _guard = self.state.operation_guard().await;
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let chain_profiles = inventory.chain_profiles.clone();
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map(|deposits| deposits.eth_stealth)
            .unwrap_or_default();
        let announcement_activity = announcement_activity_blocks(&deposits);
        let mut chain_tip_blocks = BTreeMap::new();
        for provider in &providers {
            let tip = match self.provider_rpc_for_profile(provider.compartment_id, provider) {
                Ok(rpc) => rpc.get_block_number().await.ok(),
                Err(_) => None,
            };
            chain_tip_blocks.insert(provider.name.clone(), tip);
        }
        let permit2_allowance_discovery_for_provider =
            |provider: &sigillum_api::EvmProviderProfile| {
                permit2_allowance_discovery_config(
                    body.discover_permit2_allowances,
                    &body.permit2_contract_addresses,
                    &body.permit2_spender_addresses,
                    body.permit2_allowance_limit,
                    chain_profile_for_id(&chain_profiles, provider.chain_id)
                        .and_then(|profile| profile.permit2_address.as_deref()),
                )
            };
        let mut watch_probes = body.watch_addresses.clone();
        if body.include_watch_book.unwrap_or(false) {
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

        let started_at_unix = now_unix();
        let mut job = WalletDiscoveryJob {
            id: random_id(),
            status: "running".into(),
            source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
            wallet_families: unique_strings(
                wallets.iter().map(|wallet| wallet.family.clone()).chain(
                    watch_addresses
                        .iter()
                        .map(|watch| watch.wallet.family.clone()),
                ),
            ),
            wallet_profiles: unique_strings(
                wallets.iter().map(|wallet| wallet.profile.clone()).chain(
                    watch_addresses
                        .iter()
                        .map(|watch| watch.wallet.profile.clone()),
                ),
            ),
            provider_profiles: unique_strings(
                providers.iter().map(|provider| provider.name.clone()),
            ),
            chain_ids: unique_u64s(providers.iter().map(|provider| provider.chain_id)),
            gap_limit,
            max_index,
            addresses_scanned: 0,
            active_addresses: 0,
            holdings_detected: 0,
            checkpoints: Vec::new(),
            block_cursors: latest_block_scan_cursors(
                &inventory.jobs,
                providers.iter().map(|provider| provider.chain_id),
            ),
            started_at_unix,
            completed_at_unix: None,
            last_error: None,
        };
        inventory.jobs.push(job.clone());
        save_inventory_state(&self.state.base_dir, &inventory)?;

        let mut scanned_addresses = Vec::new();
        let mut detected_holdings = Vec::new();

        for wallet in &wallets {
            let (mut index, mut empty_run) = if body.resume_from_latest_checkpoint.unwrap_or(false)
            {
                latest_resume_checkpoint(&inventory.jobs, wallet, &providers).unwrap_or((0, 0))
            } else {
                (0, 0)
            };
            while index <= max_index && empty_run < gap_limit {
                let derived =
                    derive_discovery_wallet_address(wallet, index).map_err(map_xpub_error)?;
                let derivation_path = format!("{}/{index}", wallet.receive_path);
                let mut index_has_activity = false;

                for provider in &providers {
                    let permit2_allowance_discovery =
                        permit2_allowance_discovery_for_provider(provider)?;
                    let mut observation = self
                        .observe_inventory_address(
                            wallet,
                            provider,
                            &derived.address,
                            &derivation_path,
                            index,
                            &block_tag,
                            &token_addresses,
                            token_discovery.as_ref(),
                            allowance_discovery.as_ref(),
                            permit2_allowance_discovery.as_ref(),
                            nft_discovery.as_ref(),
                            erc1155_discovery.as_ref(),
                            nft_operator_approval_discovery.as_ref(),
                            defi_position_discovery.as_ref(),
                            claim_candidate_discovery.as_ref(),
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
                            block_tag: &block_tag,
                            config: token_registry_probe.as_ref(),
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
                for provider in &providers {
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
                index += 1;
            }
            for provider in &providers {
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
                if let Some(seed_profile) = registry
                    .eth_seed_wallets
                    .iter()
                    .find(|p| p.name == wallet.profile)
                {
                    if let Some(control_xpub) = &seed_profile.control_xpub {
                        let control_path = format!("m/44'/60'/{}'/1", seed_profile.project_account);
                        for control_index in 0..=2 {
                            let derived = derive_ethereum_address_from_control_xpub(
                                control_xpub,
                                control_index,
                            )
                            .map_err(map_xpub_error)?;
                            let derivation_path = format!("{control_path}/{control_index}");
                            for provider in &providers {
                                let permit2_allowance_discovery =
                                    permit2_allowance_discovery_for_provider(provider)?;
                                let mut observation = self
                                    .observe_inventory_address(
                                        wallet,
                                        provider,
                                        &derived.address,
                                        &derivation_path,
                                        control_index,
                                        &block_tag,
                                        &token_addresses,
                                        token_discovery.as_ref(),
                                        allowance_discovery.as_ref(),
                                        permit2_allowance_discovery.as_ref(),
                                        nft_discovery.as_ref(),
                                        erc1155_discovery.as_ref(),
                                        nft_operator_approval_discovery.as_ref(),
                                        defi_position_discovery.as_ref(),
                                        claim_candidate_discovery.as_ref(),
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
                                        block_tag: &block_tag,
                                        config: token_registry_probe.as_ref(),
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
        }

        for watch in &watch_addresses {
            let derivation_path = format!("{}/{}", watch.wallet.receive_path, watch.address_index);
            for provider in &providers {
                let permit2_allowance_discovery =
                    permit2_allowance_discovery_for_provider(provider)?;
                let mut observation = self
                    .observe_inventory_address(
                        &watch.wallet,
                        provider,
                        &watch.address,
                        &derivation_path,
                        watch.address_index,
                        &block_tag,
                        &token_addresses,
                        token_discovery.as_ref(),
                        allowance_discovery.as_ref(),
                        permit2_allowance_discovery.as_ref(),
                        nft_discovery.as_ref(),
                        erc1155_discovery.as_ref(),
                        nft_operator_approval_discovery.as_ref(),
                        defi_position_discovery.as_ref(),
                        claim_candidate_discovery.as_ref(),
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
                        block_tag: &block_tag,
                        config: token_registry_probe.as_ref(),
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

        job.status = "completed".into();
        job.completed_at_unix = Some(now_unix());
        sync_inventory_job(&mut inventory, &job);
        save_inventory_state(&self.state.base_dir, &inventory)?;

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
        })
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
