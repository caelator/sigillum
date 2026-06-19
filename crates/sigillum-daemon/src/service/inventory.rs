//! Wallet inventory and read-only discovery operations.

mod allowance_discovery;
mod checkpoints;
mod claim_discovery;
mod defi_adapters;
mod defi_discovery;
mod export;
mod nft_approval_discovery;
mod nft_discovery;
mod observation;
mod permit2_discovery;
mod planner;
mod preflight;
mod risk;
mod risk_catalog;
mod simulation;
mod support;
mod token_discovery;
mod treasury;
mod wallet_selection;
mod watch_book;
mod watch_discovery;

use sigillum_api::{
    ChainProfile, ChainProfileDeleteRequest, ChainProfileListResponse,
    ChainProfileMutationResponse, ChainProfileUpsertRequest, ConsolidationPlan,
    ConsolidationPlanApproveRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanListResponse, ConsolidationPlanMutationResponse, DiscoveryJobListResponse,
    DiscoveryJobMutationRequest, DiscoveryJobMutationResponse, RiskFindingListResponse,
    WalletDiscoveryJob, WalletInventoryListResponse, WalletInventoryScanRequest,
    WalletInventoryScanResponse, WatchAddressProbe,
};
use sigillum_core::{derive_ethereum_address_from_control_xpub, derive_ethereum_address_from_xpub};

use crate::audit_log::AuditEventSpec;

use allowance_discovery::erc20_allowance_discovery_config;
use checkpoints::{
    ScanCheckpointProgress, latest_resume_checkpoint, sync_inventory_job, update_scan_checkpoint,
};
use claim_discovery::claim_candidate_discovery_config;
use defi_discovery::defi_token_position_discovery_config;
use nft_approval_discovery::nft_operator_approval_discovery_config;
use nft_discovery::{erc721_transfer_discovery_config, erc1155_transfer_discovery_config};
use permit2_discovery::permit2_allowance_discovery_config;
use planner::{
    apply_policy_blockers_to_step, build_plan_steps, plan_policy_violations, summarize_plan_steps,
};
use risk::derive_inventory_risk_findings;
use support::{
    default_native_symbol, load_inventory_state, normalized_wallet_family,
    record_inventory_observation, save_inventory_state, select_providers, trimmed_optional,
    trimmed_required, unique_strings, validated_gap_limit, validated_max_index,
};
use token_discovery::erc20_transfer_discovery_config;
use wallet_selection::{
    DERIVATION_PATTERN_PROJECT, DiscoveryWallet, SeedDerivationPattern, scan_account_limit,
    select_discovery_wallets,
};
use watch_discovery::select_watch_addresses;

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

    pub(crate) fn list_chain_profiles(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<ChainProfileListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(ChainProfileListResponse {
            profiles: state.chain_profiles,
        })
    }

    pub(crate) async fn upsert_chain_profile(
        &self,
        token: Option<&str>,
        body: ChainProfileUpsertRequest,
    ) -> ServiceResult<ChainProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let now = now_unix();
        let name = trimmed_required("name", &body.name)?;
        let mut profile = ChainProfile {
            name: name.clone(),
            chain_family: trimmed_required("chain_family", &body.chain_family)?,
            chain_id: body.chain_id,
            provider_profile: body.provider_profile.and_then(trimmed_optional),
            native_symbol: body
                .native_symbol
                .and_then(trimmed_optional)
                .unwrap_or_else(|| default_native_symbol(&body.chain_family).to_string()),
            explorer_url: body.explorer_url.and_then(trimmed_optional),
            capabilities: unique_strings(
                body.capabilities.into_iter().filter_map(trimmed_optional),
            ),
            enabled: body.enabled.unwrap_or(true),
            source: DISCOVERY_SOURCE_OPERATOR.into(),
            created_at_unix: now,
            updated_at_unix: now,
        };

        if let Some(existing) = state
            .chain_profiles
            .iter_mut()
            .find(|existing| existing.name == name)
        {
            profile.created_at_unix = existing.created_at_unix;
            *existing = profile.clone();
        } else {
            state.chain_profiles.push(profile.clone());
        }
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryChainProfileUpsert {
                name: profile.name.clone(),
                chain_family: profile.chain_family.clone(),
            },
        )?;

        Ok(ChainProfileMutationResponse {
            status: "upserted".into(),
            profile,
        })
    }

    pub(crate) async fn delete_chain_profile(
        &self,
        token: Option<&str>,
        body: ChainProfileDeleteRequest,
    ) -> ServiceResult<ChainProfileMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let name = trimmed_required("name", &body.name)?;
        let position = state
            .chain_profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| ServiceError::not_found("Chain profile not found."))?;
        let profile = state.chain_profiles.remove(position);
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryChainProfileDelete {
                name: profile.name.clone(),
            },
        )?;

        Ok(ChainProfileMutationResponse {
            status: "deleted".into(),
            profile,
        })
    }

    pub(crate) fn list_discovery_jobs(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<DiscoveryJobListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(DiscoveryJobListResponse { jobs: state.jobs })
    }

    pub(crate) async fn cancel_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        self.update_discovery_job_status(token, body, "canceled")
            .await
    }

    pub(crate) async fn resume_discovery_job(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        self.update_discovery_job_status(token, body, "resume_requested")
            .await
    }

    async fn update_discovery_job_status(
        &self,
        token: Option<&str>,
        body: DiscoveryJobMutationRequest,
        status: &str,
    ) -> ServiceResult<DiscoveryJobMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Discovery job not found."))?;
        job.status = status.to_string();
        job.completed_at_unix = Some(now_unix());
        let job = job.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletInventoryDiscoveryJobUpdate {
                id: job.id.clone(),
                status: job.status.clone(),
            },
        )?;

        Ok(DiscoveryJobMutationResponse {
            status: job.status.clone(),
            job,
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
        ));
        Ok(RiskFindingListResponse { findings })
    }

    pub(crate) fn list_consolidation_plans(
        &self,
        token: Option<&str>,
    ) -> ServiceResult<ConsolidationPlanListResponse> {
        let _ = self.require_session(token)?;
        let state = load_inventory_state(&self.state.base_dir)?;
        Ok(ConsolidationPlanListResponse {
            plans: state.consolidation_plans,
        })
    }

    pub(crate) async fn generate_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanGenerateRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let registry = crate::profiles::load_profiles(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load profiles: {error}")))?;
        let now = now_unix();
        let destination_address = body.destination_address.clone().and_then(trimmed_optional);
        let policy = state.treasury_policy.clone();
        let mut steps = build_plan_steps(&state, &registry, &body, &destination_address);

        // Policy runs after planning so planner blockers and policy verdicts
        // are both visible on each step, then the summary reflects the final
        // step statuses.
        if let Some(policy) = policy.as_ref() {
            for step in &mut steps {
                apply_policy_blockers_to_step(policy, step);
            }
        }
        let policy_violations = policy
            .as_ref()
            .map(|policy| plan_policy_violations(policy, &steps))
            .unwrap_or_default();
        let summary = summarize_plan_steps(&steps);
        let status = if summary.total_steps == 0 {
            "empty"
        } else if summary.blocked_steps > 0 || !policy_violations.is_empty() {
            "blocked"
        } else {
            "review_required"
        };
        let plan = ConsolidationPlan {
            id: random_id(),
            status: status.into(),
            destination_address,
            created_at_unix: now,
            updated_at_unix: now,
            summary,
            policy_violations,
            steps,
        };
        state.consolidation_plans.push(plan.clone());
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanGenerate {
                id: plan.id.clone(),
                steps: plan.summary.total_steps,
                blocked: plan.summary.blocked_steps,
            },
        )?;

        Ok(ConsolidationPlanMutationResponse {
            status: "generated".into(),
            plan,
        })
    }

    pub(crate) async fn approve_consolidation_plan(
        &self,
        token: Option<&str>,
        body: ConsolidationPlanApproveRequest,
    ) -> ServiceResult<ConsolidationPlanMutationResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut state = load_inventory_state(&self.state.base_dir)?;
        let policy = state.treasury_policy.clone();
        let plan = state
            .consolidation_plans
            .iter_mut()
            .find(|plan| plan.id == body.plan_id)
            .ok_or_else(|| ServiceError::not_found("Consolidation plan not found."))?;
        let approve_all = body.step_ids.is_empty();
        for step in &mut plan.steps {
            if step.status == "review_required"
                && (approve_all || body.step_ids.iter().any(|id| id == &step.id))
            {
                // Approval is the last review gate, so candidates are
                // re-checked against the CURRENT policy: a step planned
                // before a policy change must not slip through approval.
                if let Some(policy) = policy.as_ref() {
                    apply_policy_blockers_to_step(policy, step);
                    if step.status == "blocked" {
                        continue;
                    }
                }
                step.approved = true;
                step.status = "approved".into();
            }
        }
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = if plan.summary.blocked_steps > 0 || !plan.policy_violations.is_empty() {
            "blocked".into()
        } else if plan.summary.review_required_steps > 0 {
            "review_required".into()
        } else {
            "approved".into()
        };
        let plan = plan.clone();
        save_inventory_state(&self.state.base_dir, &state)?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::WalletConsolidationPlanApprove {
                id: plan.id.clone(),
                approved: plan.summary.approved_steps,
            },
        )?;

        Ok(ConsolidationPlanMutationResponse {
            status: "approved".into(),
            plan,
        })
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
        let permit2_allowance_discovery = permit2_allowance_discovery_config(
            body.discover_permit2_allowances,
            &body.permit2_contract_addresses,
            &body.permit2_spender_addresses,
            body.permit2_allowance_limit,
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
        let requested_family = normalized_wallet_family(body.wallet_family.as_deref())?;
        let seed_derivation_pattern =
            SeedDerivationPattern::parse(body.derivation_pattern.as_deref())?;
        let account_limit = scan_account_limit(body.account_limit)?;

        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let providers =
            select_providers(&registry.evm_providers, body.provider_profile.as_deref())?;
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
            gap_limit,
            max_index,
            addresses_scanned: 0,
            active_addresses: 0,
            holdings_detected: 0,
            checkpoints: Vec::new(),
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
                let derived = derive_ethereum_address_from_xpub(&wallet.receive_xpub, index)
                    .map_err(map_xpub_error)?;
                let derivation_path = format!("{}/{index}", wallet.receive_path);
                let mut index_has_activity = false;

                for provider in &providers {
                    let observation = self
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
                            started_at_unix,
                        )
                        .await?;
                    if observation.address.activity_state != "empty" {
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
                                let observation = self
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
                                        started_at_unix,
                                    )
                                    .await?;
                                record_inventory_observation(
                                    &mut job,
                                    &mut inventory,
                                    observation,
                                    &mut detected_holdings,
                                    &mut scanned_addresses,
                                );
                            }
                        }
                    }
                }
            }
        }

        for watch in &watch_addresses {
            let derivation_path = format!("{}/{}", watch.wallet.receive_path, watch.address_index);
            for provider in &providers {
                let observation = self
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
                        started_at_unix,
                    )
                    .await?;
                record_inventory_observation(
                    &mut job,
                    &mut inventory,
                    observation,
                    &mut detected_holdings,
                    &mut scanned_addresses,
                );
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
