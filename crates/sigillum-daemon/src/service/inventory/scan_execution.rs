use std::collections::BTreeMap;

use sigillum_api::{
    OPERATION_STATE_CANCELED, OPERATION_STATE_COMPLETED, OPERATION_STATE_FAILED,
    WalletAssetHolding, WalletDiscoveryJob, WalletInventoryAddress, WalletInventoryScanResponse,
};
use sigillum_core::derive_ethereum_address_from_control_xpub;

use crate::audit_log::AuditEventSpec;
use crate::operation_registry::OperationHandle;
use crate::service::chains::chain_profile_for_id;
use crate::service::helpers::{map_xpub_error, now_unix, random_id};
use crate::service::{ServiceResult, SigillumService};

use super::checkpoints::{
    ScanCheckpointProgress, latest_block_scan_cursors, latest_resume_checkpoint,
    sync_inventory_job, update_scan_checkpoint,
};
use super::partition::{self, ProviderPartitions};
use super::permit2_discovery::permit2_allowance_discovery_config;
use super::support::{
    announcement_activity_blocks, load_inventory_state, record_inventory_observation,
    save_inventory_state,
};
use super::wallet_selection::{DERIVATION_PATTERN_PROJECT, derive_discovery_wallet_address};
use super::{
    PreparedEvmScan, TokenRegistryObservationProbe, WALLET_FAMILY_ETH_SEED, accepted_scan_job,
    activity_context_for_observation,
};

impl SigillumService {
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
    pub(super) async fn execute_evm_scan(
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
        // Partitioned scans attribute observations per provider (job-level
        // disjoint-coverage evidence) and pace provider batches with jitter.
        let partitioning_engaged = prepared.provider_partitions.is_some();
        let mut provider_batches_started = 0usize;
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

                    // Partitioning (opt-in): exactly one provider per
                    // multi-provider chain probes this address, chosen by
                    // stable hash; otherwise every provider, as today.
                    let address_providers = ProviderPartitions::select_for_address(
                        prepared.provider_partitions.as_ref(),
                        &prepared.providers,
                        &derived.address,
                    );
                    for provider in address_providers {
                        if partitioning_engaged {
                            partition::sleep_between_provider_batches(provider_batches_started)
                                .await;
                            provider_batches_started += 1;
                        }
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
                            partitioning_engaged,
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
                                let address_providers = ProviderPartitions::select_for_address(
                                    prepared.provider_partitions.as_ref(),
                                    &prepared.providers,
                                    &derived.address,
                                );
                                for provider in address_providers {
                                    if partitioning_engaged {
                                        partition::sleep_between_provider_batches(
                                            provider_batches_started,
                                        )
                                        .await;
                                        provider_batches_started += 1;
                                    }
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
                                        partitioning_engaged,
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
                let address_providers = ProviderPartitions::select_for_address(
                    prepared.provider_partitions.as_ref(),
                    &prepared.providers,
                    &watch.address,
                );
                for provider in address_providers {
                    if partitioning_engaged {
                        partition::sleep_between_provider_batches(provider_batches_started).await;
                        provider_batches_started += 1;
                    }
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
                        partitioning_engaged,
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
}
