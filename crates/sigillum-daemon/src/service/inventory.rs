//! Wallet inventory and read-only discovery operations.

mod allowance_discovery;
mod nft_discovery;
mod observation;
mod planner;
mod risk;
mod support;
mod token_discovery;

use sigillum_api::{
    ChainProfile, ChainProfileDeleteRequest, ChainProfileListResponse,
    ChainProfileMutationResponse, ChainProfileUpsertRequest, ConsolidationPlan,
    ConsolidationPlanApproveRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanListResponse, ConsolidationPlanMutationResponse, DiscoveryJobListResponse,
    DiscoveryJobMutationRequest, DiscoveryJobMutationResponse, EthSeedWalletProfile,
    EthXpubWalletProfile, RiskFindingListResponse, WalletDiscoveryJob, WalletInventoryListResponse,
    WalletInventoryScanRequest, WalletInventoryScanResponse,
};
use sigillum_core::{
    VaultLifecycle, decode_quantity_hex, derive_ethereum_address_from_control_xpub,
    derive_ethereum_address_from_xpub, derive_sigillum_ethereum_xpub_receive_branch,
};

use crate::audit_log::AuditEventSpec;

use allowance_discovery::erc20_allowance_discovery_config;
use nft_discovery::{erc721_transfer_discovery_config, erc1155_transfer_discovery_config};
use planner::{plan_step_for_holding, signer_status_for_holding, summarize_plan_steps};
use risk::derive_inventory_risk_findings;
use support::{
    default_native_symbol, load_inventory_state, normalized_wallet_family, quantity_hex_is_nonzero,
    remove_holding, save_inventory_state, select_providers, trimmed_optional, trimmed_required,
    unique_strings, upsert_address, upsert_holding, validated_gap_limit, validated_max_index,
};
use token_discovery::erc20_transfer_discovery_config;

use super::evm::normalize_address;
use super::helpers::{compare_u256, map_xpub_error, now_unix, random_id};
use super::{ServiceError, ServiceResult, SigillumService};

const WALLET_FAMILY_ETH_SEED: &str = "eth-seed";
const WALLET_FAMILY_ETH_XPUB: &str = "eth-xpub";
const DISCOVERY_SOURCE_LOCAL_RPC: &str = "local-rpc";
const DISCOVERY_SOURCE_OPERATOR: &str = "operator";
const DEFAULT_GAP_LIMIT: u32 = 20;
const MAX_GAP_LIMIT: u32 = 100;
const DEFAULT_MAX_INDEX: u32 = 200;
const MAX_SCAN_INDEX: u32 = 10_000;

#[derive(Clone, Debug)]
struct DiscoveryWallet {
    family: String,
    profile: String,
    receive_path: String,
    receive_xpub: String,
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
        let destination_address = body.destination_address.and_then(trimmed_optional);
        let mut steps = Vec::new();

        for holding in state
            .holdings
            .iter()
            .filter(|holding| quantity_hex_is_nonzero(&holding.amount_hex))
            .filter(|holding| {
                body.wallet_family
                    .as_deref()
                    .is_none_or(|family| family == holding.wallet_family)
            })
            .filter(|holding| {
                body.wallet_profile
                    .as_deref()
                    .is_none_or(|profile| profile == holding.wallet_profile)
            })
            .filter(|holding| {
                body.provider_profile
                    .as_deref()
                    .is_none_or(|profile| profile == holding.provider_profile)
            })
        {
            let signer_status = signer_status_for_holding(holding);
            if signer_status == "watch_only" && body.include_watch_only != Some(true) {
                continue;
            }
            let step_destination = if destination_address.is_some() {
                destination_address.clone()
            } else if holding.wallet_family == WALLET_FAMILY_ETH_SEED {
                if let Some(seed_profile) = registry
                    .eth_seed_wallets
                    .iter()
                    .find(|p| p.name == holding.wallet_profile)
                {
                    if seed_profile.hot_address.is_some() && seed_profile.treasury_address.is_some()
                    {
                        let hot_addr = seed_profile.hot_address.as_ref().unwrap();
                        let treasury_addr = seed_profile.treasury_address.as_ref().unwrap();
                        let hot_balance = state
                            .addresses
                            .iter()
                            .find(|addr| {
                                addr.wallet_profile == holding.wallet_profile
                                    && addr.address == *hot_addr
                            })
                            .and_then(|addr| decode_quantity_hex(&addr.native_balance_wei_hex).ok())
                            .unwrap_or([0u8; 32]);
                        let target_refill = decode_quantity_hex("0xde0b6b3a7640000").unwrap(); // 1.0 ETH in wei
                        if compare_u256(&hot_balance, &target_refill).is_lt() {
                            Some(hot_addr.clone())
                        } else {
                            Some(treasury_addr.clone())
                        }
                    } else {
                        seed_profile.default_destination_address.clone()
                    }
                } else {
                    None
                }
            } else {
                None
            };
            steps.push(plan_step_for_holding(
                holding,
                step_destination,
                signer_status,
            ));
        }

        let summary = summarize_plan_steps(&steps);
        let status = if summary.total_steps == 0 {
            "empty"
        } else if summary.blocked_steps > 0 {
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
                step.approved = true;
                step.status = "approved".into();
            }
        }
        plan.updated_at_unix = now_unix();
        plan.summary = summarize_plan_steps(&plan.steps);
        plan.status = if plan.summary.blocked_steps > 0 {
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
        let requested_family = normalized_wallet_family(body.wallet_family.as_deref())?;

        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let providers =
            select_providers(&registry.evm_providers, body.provider_profile.as_deref())?;
        let wallets = self.select_discovery_wallets(
            token,
            &registry.eth_seed_wallets,
            &registry.eth_xpub_wallets,
            requested_family.as_deref(),
            body.wallet_profile.as_deref(),
        )?;

        let started_at_unix = now_unix();
        let mut job = WalletDiscoveryJob {
            id: random_id(),
            status: "running".into(),
            source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
            wallet_families: unique_strings(wallets.iter().map(|wallet| wallet.family.clone())),
            wallet_profiles: unique_strings(wallets.iter().map(|wallet| wallet.profile.clone())),
            provider_profiles: unique_strings(
                providers.iter().map(|provider| provider.name.clone()),
            ),
            gap_limit,
            max_index,
            addresses_scanned: 0,
            active_addresses: 0,
            holdings_detected: 0,
            started_at_unix,
            completed_at_unix: None,
            last_error: None,
        };

        let _guard = self.state.operation_guard().await;
        let mut inventory =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        let mut scanned_addresses = Vec::new();
        let mut detected_holdings = Vec::new();

        for wallet in &wallets {
            let mut empty_run = 0u32;
            let mut index = 0u32;
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
                            nft_discovery.as_ref(),
                            erc1155_discovery.as_ref(),
                            started_at_unix,
                        )
                        .await?;
                    job.addresses_scanned += 1;
                    if observation.address.activity_state != "empty" {
                        job.active_addresses += 1;
                        index_has_activity = true;
                    }
                    for holding in &observation.holdings {
                        if quantity_hex_is_nonzero(&holding.amount_hex) {
                            job.holdings_detected += 1;
                        }
                    }

                    upsert_address(&mut inventory.addresses, observation.address.clone());
                    for holding in observation.holdings.iter().cloned() {
                        if quantity_hex_is_nonzero(&holding.amount_hex) {
                            upsert_holding(&mut inventory.holdings, holding.clone());
                            detected_holdings.push(holding);
                        } else {
                            remove_holding(&mut inventory.holdings, &holding);
                        }
                    }
                    scanned_addresses.push(observation.address);
                }

                if index_has_activity {
                    empty_run = 0;
                } else {
                    empty_run += 1;
                }
                index += 1;
            }

            if wallet.family == WALLET_FAMILY_ETH_SEED {
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
                                        nft_discovery.as_ref(),
                                        erc1155_discovery.as_ref(),
                                        started_at_unix,
                                    )
                                    .await?;
                                job.addresses_scanned += 1;
                                if observation.address.activity_state != "empty" {
                                    job.active_addresses += 1;
                                }
                                for holding in &observation.holdings {
                                    if quantity_hex_is_nonzero(&holding.amount_hex) {
                                        job.holdings_detected += 1;
                                    }
                                }
                                upsert_address(
                                    &mut inventory.addresses,
                                    observation.address.clone(),
                                );
                                for holding in observation.holdings.iter().cloned() {
                                    if quantity_hex_is_nonzero(&holding.amount_hex) {
                                        upsert_holding(&mut inventory.holdings, holding.clone());
                                        detected_holdings.push(holding);
                                    } else {
                                        remove_holding(&mut inventory.holdings, &holding);
                                    }
                                }
                                scanned_addresses.push(observation.address);
                            }
                        }
                    }
                }
            }
        }

        job.status = "completed".into();
        job.completed_at_unix = Some(now_unix());
        inventory.jobs.push(job.clone());
        crate::inventory::save_wallet_inventory(&self.state.base_dir, &inventory).map_err(
            |error| ServiceError::internal(format!("Failed to save wallet inventory: {error}")),
        )?;

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

    fn select_discovery_wallets(
        &self,
        _token: &str,
        seed_profiles: &[EthSeedWalletProfile],
        xpub_profiles: &[EthXpubWalletProfile],
        requested_family: Option<&str>,
        requested_profile: Option<&str>,
    ) -> ServiceResult<Vec<DiscoveryWallet>> {
        let mut wallets = Vec::new();

        if requested_family.is_none() || requested_family == Some(WALLET_FAMILY_ETH_SEED) {
            for profile in seed_profiles {
                if requested_profile.is_some_and(|name| name != profile.name) {
                    continue;
                }
                wallets.push(DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_SEED.into(),
                    profile: profile.name.clone(),
                    receive_path: profile.receive_path.clone(),
                    receive_xpub: profile.receive_xpub.clone(),
                });
            }
        }

        if requested_family.is_none() || requested_family == Some(WALLET_FAMILY_ETH_XPUB) {
            for profile in xpub_profiles {
                if requested_profile.is_some_and(|name| name != profile.name) {
                    continue;
                }
                let export = self.with_vault(profile.compartment_id, |vault| {
                    let master_key = vault
                        .extract_master_key()
                        .ok_or_else(|| ServiceError::forbidden("Wallet compartment is locked."))?;
                    derive_sigillum_ethereum_xpub_receive_branch(
                        master_key.as_ref(),
                        profile.project_account,
                    )
                    .map_err(map_xpub_error)
                })?;
                wallets.push(DiscoveryWallet {
                    family: WALLET_FAMILY_ETH_XPUB.into(),
                    profile: profile.name.clone(),
                    receive_path: export.receive_path,
                    receive_xpub: export.receive_xpub,
                });
            }
        }

        if wallets.is_empty() {
            return Err(ServiceError::not_found(
                "No matching seed or xpub wallet profiles found.",
            ));
        }

        Ok(wallets)
    }
}
