//! Wallet inventory and read-only discovery operations.

use sigillum_api::{
    ChainProfile, ChainProfileDeleteRequest, ChainProfileListResponse,
    ChainProfileMutationResponse, ChainProfileUpsertRequest, ConsolidationPlan,
    ConsolidationPlanApproveRequest, ConsolidationPlanGenerateRequest,
    ConsolidationPlanListResponse, ConsolidationPlanMutationResponse, ConsolidationPlanStep,
    ConsolidationPlanSummary, DiscoveryJobListResponse, DiscoveryJobMutationRequest,
    DiscoveryJobMutationResponse, EthSeedWalletProfile, EthXpubWalletProfile, EvmProviderProfile,
    RiskFinding, RiskFindingListResponse, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress, WalletInventoryListResponse, WalletInventoryScanRequest,
    WalletInventoryScanResponse,
};
use sigillum_core::{
    VaultLifecycle, derive_ethereum_address_from_xpub, derive_sigillum_ethereum_xpub_receive_branch,
};

use crate::audit_log::AuditEventSpec;

use super::evm::normalize_address;
use super::helpers::{map_xpub_error, now_unix, random_id};
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
            steps.push(plan_step_for_holding(
                holding,
                destination_address.clone(),
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

    #[allow(clippy::too_many_arguments)]
    async fn observe_inventory_address(
        &self,
        wallet: &DiscoveryWallet,
        provider: &EvmProviderProfile,
        address: &str,
        derivation_path: &str,
        address_index: u32,
        block_tag: &str,
        token_addresses: &[String],
        now: u64,
    ) -> ServiceResult<InventoryAddressObservation> {
        let address = normalize_address(address)?;
        let native_balance_wei_hex = self
            .evm_native_balance_for_provider(provider.compartment_id, provider, &address, block_tag)
            .await?;
        let transaction_count = self
            .evm_transaction_count_for_provider(
                provider.compartment_id,
                provider,
                &address,
                block_tag,
            )
            .await?;
        let mut activity_state = if quantity_hex_is_nonzero(&native_balance_wei_hex) {
            "funded"
        } else if transaction_count > 0 {
            "active"
        } else {
            "empty"
        };

        let record_context = InventoryRecordContext {
            wallet,
            provider,
            address: &address,
            derivation_path,
            now,
        };
        let mut holdings = vec![holding_record(
            &record_context,
            "native",
            None,
            &native_balance_wei_hex,
        )];

        for token_address in token_addresses {
            let amount_hex = self
                .evm_erc20_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    token_address,
                    &address,
                    block_tag,
                )
                .await?;
            if quantity_hex_is_nonzero(&amount_hex) {
                activity_state = "funded";
            }
            holdings.push(holding_record(
                &record_context,
                "erc20",
                Some(token_address.clone()),
                &amount_hex,
            ));
        }

        Ok(InventoryAddressObservation {
            address: address_record(
                &record_context,
                address_index,
                activity_state,
                &native_balance_wei_hex,
                transaction_count,
            ),
            holdings,
        })
    }
}

#[derive(Clone, Debug)]
struct InventoryAddressObservation {
    address: WalletInventoryAddress,
    holdings: Vec<WalletAssetHolding>,
}

#[derive(Clone, Copy, Debug)]
struct InventoryRecordContext<'a> {
    wallet: &'a DiscoveryWallet,
    provider: &'a EvmProviderProfile,
    address: &'a str,
    derivation_path: &'a str,
    now: u64,
}

fn load_inventory_state(
    base_dir: &std::path::Path,
) -> ServiceResult<crate::inventory::WalletInventoryState> {
    crate::inventory::load_wallet_inventory(base_dir).map_err(|error| {
        ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
    })
}

fn save_inventory_state(
    base_dir: &std::path::Path,
    state: &crate::inventory::WalletInventoryState,
) -> ServiceResult<()> {
    crate::inventory::save_wallet_inventory(base_dir, state).map_err(|error| {
        ServiceError::internal(format!("Failed to save wallet inventory: {error}"))
    })
}

fn trimmed_required(field: &str, value: &str) -> ServiceResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ServiceError::bad_request(format!("{field} is required")));
    }
    Ok(value.to_string())
}

fn trimmed_optional(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn default_native_symbol(chain_family: &str) -> &'static str {
    match chain_family.trim() {
        "bitcoin" | "utxo" => "BTC",
        "solana" => "SOL",
        "tron" => "TRX",
        "cosmos" => "ATOM",
        _ => "ETH",
    }
}

fn derive_inventory_risk_findings(
    addresses: &[WalletInventoryAddress],
    holdings: &[WalletAssetHolding],
) -> Vec<RiskFinding> {
    let mut findings = Vec::new();
    for holding in holdings
        .iter()
        .filter(|holding| quantity_hex_is_nonzero(&holding.amount_hex))
    {
        if holding.asset_kind != "native"
            && native_balance_for_holding(addresses, holding)
                .is_none_or(|balance| !quantity_hex_is_nonzero(balance))
        {
            findings.push(RiskFinding {
                id: stable_finding_id("stranded_gas", holding),
                category: "stranded_value".into(),
                risk_level: "medium".into(),
                status: "open".into(),
                wallet_family: holding.wallet_family.clone(),
                wallet_profile: holding.wallet_profile.clone(),
                provider_profile: holding.provider_profile.clone(),
                chain_id: holding.chain_id,
                address: holding.address.clone(),
                subject_type: holding.asset_kind.clone(),
                subject: holding
                    .asset_address
                    .clone()
                    .unwrap_or_else(|| "native".into()),
                source: "local-risk-engine".into(),
                recommendation: "Fund gas or route through an approved sponsor before sweeping."
                    .into(),
                evidence: vec![
                    "Positive non-native holding detected".into(),
                    "No native gas balance detected for the same address".into(),
                ],
                first_seen_at_unix: holding.first_seen_at_unix,
                last_checked_at_unix: holding.last_checked_at_unix,
            });
        }
    }
    findings
}

fn native_balance_for_holding<'a>(
    addresses: &'a [WalletInventoryAddress],
    holding: &WalletAssetHolding,
) -> Option<&'a str> {
    addresses
        .iter()
        .find(|address| {
            address.wallet_family == holding.wallet_family
                && address.wallet_profile == holding.wallet_profile
                && address.provider_profile == holding.provider_profile
                && address.chain_id == holding.chain_id
                && address.address == holding.address
        })
        .map(|address| address.native_balance_wei_hex.as_str())
}

fn stable_finding_id(prefix: &str, holding: &WalletAssetHolding) -> String {
    format!(
        "{prefix}:{}:{}:{}:{}:{}",
        holding.wallet_family,
        holding.wallet_profile,
        holding.provider_profile,
        holding.chain_id,
        holding.address
    )
}

fn signer_status_for_holding(holding: &WalletAssetHolding) -> &'static str {
    match holding.wallet_family.as_str() {
        WALLET_FAMILY_ETH_XPUB => "watch_only",
        WALLET_FAMILY_ETH_SEED => "signing_not_implemented",
        _ => "unknown",
    }
}

fn plan_step_for_holding(
    holding: &WalletAssetHolding,
    destination_address: Option<String>,
    signer_status: &str,
) -> ConsolidationPlanStep {
    let action = match holding.asset_kind.as_str() {
        "native" => "sweep_native",
        "erc20" => "sweep_erc20",
        "erc721" | "erc1155" | "nft" => "sweep_nft",
        "approval" => "revoke_approval",
        "defi" => "exit_defi_position",
        "airdrop" | "reward" => "claim_reward",
        _ => "review_asset",
    };
    let mut blockers = Vec::new();
    if destination_address.is_none() && action.starts_with("sweep") {
        blockers.push("missing_destination".into());
    }
    if signer_status != "available" {
        blockers.push(signer_status.to_string());
    }
    let blocked_by_kind = matches!(
        holding.asset_kind.as_str(),
        "approval" | "defi" | "airdrop" | "reward"
    );
    if blocked_by_kind {
        blockers.push("requires_protocol_adapter".into());
    }
    let status = if blockers.is_empty() {
        "review_required"
    } else {
        "blocked"
    };

    ConsolidationPlanStep {
        id: random_id(),
        action: action.into(),
        status: status.into(),
        wallet_family: holding.wallet_family.clone(),
        wallet_profile: holding.wallet_profile.clone(),
        provider_profile: holding.provider_profile.clone(),
        chain_id: holding.chain_id,
        address: holding.address.clone(),
        derivation_path: holding.derivation_path.clone(),
        asset_kind: holding.asset_kind.clone(),
        asset_address: holding.asset_address.clone(),
        amount_hex: holding.amount_hex.clone(),
        destination_address,
        signer_status: signer_status.into(),
        simulation_status: "not_run".into(),
        risk_level: if blockers.is_empty() {
            "low".into()
        } else {
            "blocked".into()
        },
        blockers,
        auto_eligible: false,
        approved: false,
    }
}

fn summarize_plan_steps(steps: &[ConsolidationPlanStep]) -> ConsolidationPlanSummary {
    ConsolidationPlanSummary {
        total_steps: steps.len(),
        blocked_steps: steps.iter().filter(|step| step.status == "blocked").count(),
        review_required_steps: steps
            .iter()
            .filter(|step| step.status == "review_required")
            .count(),
        approved_steps: steps.iter().filter(|step| step.approved).count(),
        executable_steps: steps
            .iter()
            .filter(|step| step.status == "approved" && step.blockers.is_empty())
            .count(),
        value_items: steps
            .iter()
            .filter(|step| quantity_hex_is_nonzero(&step.amount_hex))
            .count(),
    }
}

fn validated_gap_limit(value: Option<u32>) -> ServiceResult<u32> {
    let value = value.unwrap_or(DEFAULT_GAP_LIMIT);
    if value == 0 || value > MAX_GAP_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "gap_limit must be between 1 and {MAX_GAP_LIMIT}"
        )));
    }
    Ok(value)
}

fn validated_max_index(value: Option<u32>) -> ServiceResult<u32> {
    let value = value.unwrap_or(DEFAULT_MAX_INDEX);
    if value > MAX_SCAN_INDEX {
        return Err(ServiceError::bad_request(format!(
            "max_index must be <= {MAX_SCAN_INDEX}"
        )));
    }
    Ok(value)
}

fn normalized_wallet_family(value: Option<&str>) -> ServiceResult<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(WALLET_FAMILY_ETH_SEED) => Ok(Some(WALLET_FAMILY_ETH_SEED.into())),
        Some(WALLET_FAMILY_ETH_XPUB) => Ok(Some(WALLET_FAMILY_ETH_XPUB.into())),
        Some(_) => Err(ServiceError::bad_request(
            "wallet_family must be 'eth-seed' or 'eth-xpub'",
        )),
    }
}

fn select_providers(
    providers: &[EvmProviderProfile],
    requested_profile: Option<&str>,
) -> ServiceResult<Vec<EvmProviderProfile>> {
    let selected = providers
        .iter()
        .filter(|provider| requested_profile.is_none_or(|name| name == provider.name))
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(ServiceError::not_found(
            "No matching EVM provider profiles found.",
        ));
    }
    Ok(selected)
}

fn address_record(
    context: &InventoryRecordContext<'_>,
    address_index: u32,
    activity_state: &str,
    native_balance_wei_hex: &str,
    transaction_count: u64,
) -> WalletInventoryAddress {
    WalletInventoryAddress {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        address_index,
        activity_state: activity_state.to_string(),
        native_balance_wei_hex: native_balance_wei_hex.to_string(),
        transaction_count,
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        first_seen_at_unix: context.now,
        last_checked_at_unix: context.now,
    }
}

fn holding_record(
    context: &InventoryRecordContext<'_>,
    asset_kind: &str,
    asset_address: Option<String>,
    amount_hex: &str,
) -> WalletAssetHolding {
    WalletAssetHolding {
        id: random_id(),
        wallet_family: context.wallet.family.clone(),
        wallet_profile: context.wallet.profile.clone(),
        provider_profile: context.provider.name.clone(),
        chain_id: context.provider.chain_id,
        address: context.address.to_string(),
        derivation_path: context.derivation_path.to_string(),
        asset_kind: asset_kind.to_string(),
        asset_address,
        amount_hex: amount_hex.to_string(),
        source: DISCOVERY_SOURCE_LOCAL_RPC.into(),
        status: if quantity_hex_is_nonzero(amount_hex) {
            "detected".into()
        } else {
            "not_detected".into()
        },
        first_seen_at_unix: context.now,
        last_checked_at_unix: context.now,
    }
}

fn upsert_address(addresses: &mut Vec<WalletInventoryAddress>, mut next: WalletInventoryAddress) {
    if let Some(existing) = addresses.iter_mut().find(|existing| {
        existing.wallet_family == next.wallet_family
            && existing.wallet_profile == next.wallet_profile
            && existing.provider_profile == next.provider_profile
            && existing.chain_id == next.chain_id
            && existing.address == next.address
    }) {
        next.id = existing.id.clone();
        next.first_seen_at_unix = existing.first_seen_at_unix;
        *existing = next;
    } else {
        addresses.push(next);
    }
}

fn upsert_holding(holdings: &mut Vec<WalletAssetHolding>, mut next: WalletAssetHolding) {
    if let Some(existing) = holdings
        .iter_mut()
        .find(|existing| holding_key_matches(existing, &next))
    {
        next.id = existing.id.clone();
        next.first_seen_at_unix = existing.first_seen_at_unix;
        *existing = next;
    } else {
        holdings.push(next);
    }
}

fn remove_holding(holdings: &mut Vec<WalletAssetHolding>, target: &WalletAssetHolding) {
    holdings.retain(|existing| !holding_key_matches(existing, target));
}

fn holding_key_matches(left: &WalletAssetHolding, right: &WalletAssetHolding) -> bool {
    left.wallet_family == right.wallet_family
        && left.wallet_profile == right.wallet_profile
        && left.provider_profile == right.provider_profile
        && left.chain_id == right.chain_id
        && left.address == right.address
        && left.asset_kind == right.asset_kind
        && left.asset_address == right.asset_address
}

fn quantity_hex_is_nonzero(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value)
        .bytes()
        .any(|byte| byte != b'0')
}

fn unique_strings(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}
