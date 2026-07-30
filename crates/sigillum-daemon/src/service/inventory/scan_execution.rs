use std::collections::BTreeMap;

use sigillum_api::{
    OPERATION_STATE_CANCELED, OPERATION_STATE_FAILED, WalletAssetHolding, WalletDiscoveryJob,
    WalletInventoryAddress, WalletInventoryScanResponse,
};
use sigillum_core::derive_ethereum_address_from_control_xpub;

use crate::audit_log::AuditEventSpec;
use crate::operation_registry::OperationHandle;
use crate::service::helpers::{map_xpub_error, now_unix, random_id};
use crate::service::{ServiceError, ServiceResult, SessionOperationContext, SigillumService};

use super::checkpoints::{
    ScanCheckpointProgress, latest_block_scan_cursors, sync_inventory_job, update_scan_checkpoint,
};
use super::partition::{self, ProviderPartitions};
use super::scan_lifecycle::{WalletResumeProgress, latest_wallet_resume_progress};
use super::scan_provider::ScanAddressContext;
use super::support::{
    announcement_activity_blocks, load_inventory_state, record_inventory_observation,
    save_inventory_state,
};
use super::wallet_selection::{DERIVATION_PATTERN_PROJECT, derive_discovery_wallet_address};
use super::{PreparedEvmScan, WALLET_FAMILY_ETH_SEED, accepted_scan_job};

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
        session_context: SessionOperationContext,
        prepared: PreparedEvmScan,
        operation: OperationHandle,
        preset_job_id: Option<String>,
    ) -> ServiceResult<WalletInventoryScanResponse> {
        let _guard = match self.acquire_session_operation(&session_context).await {
            Ok(guard) => guard,
            Err(error) => {
                self.state.finish_operation(
                    operation.id(),
                    OPERATION_STATE_FAILED,
                    Some(error.message().to_string()),
                );
                return Err(error);
            }
        };
        let token = session_context.token.as_str();
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let chain_profiles = inventory.chain_profiles.clone();
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map(|deposits| deposits.eth_stealth)
            .unwrap_or_default();
        let announcement_activity = announcement_activity_blocks(&deposits);
        let mut chain_tip_blocks = BTreeMap::new();

        // Persist the accepted job before the first provider await. This
        // closes the submission/cancel race and gives every provider failure a
        // durable `running` record that can be terminalized.
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
        let mut canceled = false;

        // The fallible scan loop runs as one async block so mid-run errors
        // can be finalized below (job and operation marked `failed`) instead
        // of leaking a permanently `running` record.
        let loop_result: ServiceResult<()> = async {
            // Chain-tip lookups are provider awaits too. Honor a cancellation
            // that races submission before starting the next provider and
            // again immediately after each response.
            for provider in &prepared.providers {
                if self.discovery_scan_checkpoint(&operation)? {
                    canceled = true;
                    return Ok(());
                }
                let tip = match self.provider_rpc_for_profile(provider.compartment_id, provider) {
                    Ok(rpc) => rpc.get_block_number().await.ok(),
                    Err(_) => None,
                };
                if self.discovery_scan_checkpoint(&operation)? {
                    canceled = true;
                    return Ok(());
                }
                chain_tip_blocks.insert(provider.name.clone(), tip);
            }

            for wallet in &prepared.wallets {
                let (mut index, mut empty_run) = if prepared.resume_from_latest_checkpoint {
                    match latest_wallet_resume_progress(
                        &inventory.jobs,
                        wallet,
                        &prepared.providers,
                    ) {
                        Some(WalletResumeProgress::Completed) => continue,
                        Some(WalletResumeProgress::Continue {
                            next_index,
                            consecutive_empty,
                        }) => (next_index, consecutive_empty),
                        None => (0, 0),
                    }
                } else {
                    (0, 0)
                };
                while index <= prepared.max_index && empty_run < prepared.gap_limit {
                    // Cooperative cancel checkpoint: at least once per
                    // address index, before any provider call for it.
                    if self.discovery_scan_checkpoint(&operation)? {
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
                        if self.discovery_scan_checkpoint(&operation)? {
                            canceled = true;
                            break;
                        }
                        if partitioning_engaged {
                            partition::sleep_between_provider_batches(provider_batches_started)
                                .await;
                            provider_batches_started += 1;
                            if self.discovery_scan_checkpoint(&operation)? {
                                canceled = true;
                                break;
                            }
                        }
                        let Some(observation) = self
                            .observe_scan_address(
                                &operation,
                                &mut job.block_cursors,
                                ScanAddressContext {
                                    prepared: &prepared,
                                    chain_profiles: &chain_profiles,
                                    announcement_activity: &announcement_activity,
                                    chain_tip_blocks: &chain_tip_blocks,
                                    inventory: &inventory,
                                    wallet,
                                    provider,
                                    address: &derived.address,
                                    derivation_path: &derivation_path,
                                    address_index: index,
                                    started_at_unix,
                                },
                            )
                            .await?
                        else {
                            canceled = true;
                            break;
                        };
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

                    if canceled {
                        break;
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
                                if self.discovery_scan_checkpoint(&operation)? {
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
                                    if self.discovery_scan_checkpoint(&operation)? {
                                        canceled = true;
                                        break;
                                    }
                                    if partitioning_engaged {
                                        partition::sleep_between_provider_batches(
                                            provider_batches_started,
                                        )
                                        .await;
                                        provider_batches_started += 1;
                                        if self.discovery_scan_checkpoint(&operation)? {
                                            canceled = true;
                                            break;
                                        }
                                    }
                                    let Some(observation) = self
                                        .observe_scan_address(
                                            &operation,
                                            &mut job.block_cursors,
                                            ScanAddressContext {
                                                prepared: &prepared,
                                                chain_profiles: &chain_profiles,
                                                announcement_activity: &announcement_activity,
                                                chain_tip_blocks: &chain_tip_blocks,
                                                inventory: &inventory,
                                                wallet,
                                                provider,
                                                address: &derived.address,
                                                derivation_path: &derivation_path,
                                                address_index: control_index,
                                                started_at_unix,
                                            },
                                        )
                                        .await?
                                    else {
                                        canceled = true;
                                        break;
                                    };
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
                                if canceled {
                                    break;
                                }
                            }
                        }
                    }
                }
                if canceled {
                    break;
                }
                // A completed wallet checkpoint covers both the receive path
                // and its project-control path. Persist it only after every
                // provider await in both phases is authorized and durable.
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
            }

            for watch in &prepared.watch_addresses {
                if canceled {
                    break;
                }
                if self.discovery_scan_checkpoint(&operation)? {
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
                    if self.discovery_scan_checkpoint(&operation)? {
                        canceled = true;
                        break;
                    }
                    if partitioning_engaged {
                        partition::sleep_between_provider_batches(provider_batches_started).await;
                        provider_batches_started += 1;
                        if self.discovery_scan_checkpoint(&operation)? {
                            canceled = true;
                            break;
                        }
                    }
                    let Some(observation) = self
                        .observe_scan_address(
                            &operation,
                            &mut job.block_cursors,
                            ScanAddressContext {
                                prepared: &prepared,
                                chain_profiles: &chain_profiles,
                                announcement_activity: &announcement_activity,
                                chain_tip_blocks: &chain_tip_blocks,
                                inventory: &inventory,
                                wallet: &watch.wallet,
                                provider,
                                address: &watch.address,
                                derivation_path: &derivation_path,
                                address_index: watch.address_index,
                                started_at_unix,
                            },
                        )
                        .await?
                    else {
                        canceled = true;
                        break;
                    };
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
                if canceled {
                    break;
                }
            }
            if self.discovery_scan_checkpoint(&operation)? {
                canceled = true;
            }
            Ok(())
        }
        .await;

        if let Err(error) = loop_result {
            job = match self.finalize_terminal_discovery_scan(
                &job.id,
                "failed",
                Some(error.message()),
            ) {
                Ok(job) => job,
                Err(finalize_error) => {
                    let combined = format!(
                        "{error}; additionally failed to terminalize discovery job: \
                         {finalize_error}"
                    );
                    self.state.finish_operation(
                        operation.id(),
                        OPERATION_STATE_FAILED,
                        Some(combined.clone()),
                    );
                    return Err(ServiceError::internal(combined));
                }
            };
            self.state
                .operation_set_progress(operation.id(), job.addresses_scanned as u64);
            self.state.finish_operation(
                operation.id(),
                OPERATION_STATE_FAILED,
                Some(error.message().to_string()),
            );
            return Err(error);
        }

        if !canceled {
            job.status = "completed".into();
            job.completed_at_unix = Some(now_unix());
            sync_inventory_job(&mut inventory, &job);
            self.state
                .operation_set_progress(operation.id(), job.addresses_scanned as u64);
            match self
                .state
                .complete_operation_if_not_canceled(operation.id(), || {
                    save_inventory_state(&self.state.base_dir, &inventory)
                }) {
                Ok(true) => {}
                Ok(false) => canceled = true,
                Err(error) => {
                    let terminalized = self.finalize_terminal_discovery_scan(
                        &job.id,
                        "failed",
                        Some(error.message()),
                    );
                    let final_error = match terminalized {
                        Ok(_) => error.message().to_string(),
                        Err(finalize_error) => format!(
                            "{error}; additionally failed to terminalize discovery job: \
                             {finalize_error}"
                        ),
                    };
                    self.state.finish_operation(
                        operation.id(),
                        OPERATION_STATE_FAILED,
                        Some(final_error.clone()),
                    );
                    return Err(ServiceError::internal(final_error));
                }
            }
        }

        if canceled {
            job = match self.finalize_terminal_discovery_scan(&job.id, "canceled", None) {
                Ok(job) => job,
                Err(finalize_error) => {
                    let message =
                        format!("Failed to terminalize canceled discovery job: {finalize_error}");
                    self.state.finish_operation(
                        operation.id(),
                        OPERATION_STATE_FAILED,
                        Some(message.clone()),
                    );
                    return Err(ServiceError::internal(message));
                }
            };
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
                // Canceled provider results after the last durable checkpoint
                // are deliberately not represented in the response.
                addresses: Vec::new(),
                holdings: Vec::new(),
                operation: None,
            });
        }

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
