//! Lifecycle checkpoints and fail-closed terminalization for discovery scans.

use sigillum_api::{EvmProviderProfile, WalletDiscoveryCheckpoint, WalletDiscoveryJob};

use super::wallet_selection::{DERIVATION_PATTERN_PROJECT, DiscoveryWallet};
use super::{load_inventory_state, save_inventory_state};
use crate::operation_registry::OperationHandle;
use crate::service::helpers::now_unix;
use crate::service::{ServiceError, ServiceResult, SigillumService};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalletResumeProgress {
    Continue {
        next_index: u32,
        consecutive_empty: u32,
    },
    Completed,
}

/// Resolve the newest relevant durable checkpoint for one wallet. A wallet
/// whose selected providers all reached the completed phase is skipped
/// entirely; an incomplete wallet resumes from the most conservative
/// provider cursor.
pub(super) fn latest_wallet_resume_progress(
    jobs: &[WalletDiscoveryJob],
    wallet: &DiscoveryWallet,
    providers: &[EvmProviderProfile],
) -> Option<WalletResumeProgress> {
    for job in jobs.iter().rev().filter(|job| job.status != "completed") {
        let matching = job
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint.wallet_family == wallet.family
                    && checkpoint.wallet_profile == wallet.profile
                    && checkpoint_matches_derivation(checkpoint, wallet)
                    && providers
                        .iter()
                        .any(|provider| provider.name == checkpoint.provider_profile)
            })
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        if let Some(checkpoint) = matching
            .iter()
            .copied()
            .filter(|checkpoint| !checkpoint.completed)
            .min_by_key(|checkpoint| checkpoint.next_index)
        {
            return Some(WalletResumeProgress::Continue {
                next_index: checkpoint.next_index,
                consecutive_empty: checkpoint.consecutive_empty,
            });
        }
        if providers.iter().all(|provider| {
            matching.iter().any(|checkpoint| {
                checkpoint.provider_profile == provider.name && checkpoint.completed
            })
        }) {
            return Some(WalletResumeProgress::Completed);
        }
        // A legacy partial provider set cannot prove whole-wallet completion.
        return Some(WalletResumeProgress::Continue {
            next_index: 0,
            consecutive_empty: 0,
        });
    }
    None
}

fn checkpoint_matches_derivation(
    checkpoint: &WalletDiscoveryCheckpoint,
    wallet: &DiscoveryWallet,
) -> bool {
    let pattern = checkpoint
        .derivation_pattern
        .as_deref()
        .unwrap_or(DERIVATION_PATTERN_PROJECT);
    if pattern != wallet.derivation_pattern {
        return false;
    }
    let legacy_account_index = if pattern == DERIVATION_PATTERN_PROJECT {
        wallet.account_index
    } else {
        0
    };
    checkpoint.account_index.unwrap_or(legacy_account_index) == wallet.account_index
}

impl SigillumService {
    /// Revalidate the live scan after each provider await and before each
    /// durable checkpoint write. Operation cancellation deliberately bypasses
    /// `operation_lock`, and the lock latch is the other preemptive signal
    /// that can change while the scan owns that mutex.
    pub(super) fn discovery_scan_checkpoint(
        &self,
        operation: &OperationHandle,
    ) -> ServiceResult<bool> {
        if self.state.is_locking() {
            return Err(ServiceError::locked(
                "Daemon began locking while the discovery scan was running.",
            ));
        }
        Ok(operation.cancellation_requested())
    }

    /// Terminalize from the last authorized durable snapshot. Reloading the
    /// store deliberately discards any in-memory observation or block-cursor
    /// mutation produced by a provider step that failed or was canceled before
    /// its checkpoint.
    pub(super) fn finalize_terminal_discovery_scan(
        &self,
        job_id: &str,
        status: &str,
        message: Option<&str>,
    ) -> ServiceResult<WalletDiscoveryJob> {
        let mut inventory = load_inventory_state(&self.state.base_dir)?;
        let (job, changed) = {
            let job = inventory
                .jobs
                .iter_mut()
                .find(|job| job.id == job_id)
                .ok_or_else(|| ServiceError::internal("Running discovery job disappeared."))?;
            let changed = job.status == "running";
            if changed {
                job.status = status.into();
                job.completed_at_unix = Some(now_unix());
                job.last_error = message.map(|value| value.chars().take(512).collect());
            }
            (job.clone(), changed)
        };
        if changed {
            save_inventory_state(&self.state.base_dir, &inventory)?;
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{EvmProviderProfile, WalletDiscoveryCheckpoint, WalletDiscoveryJob};
    use tempfile::TempDir;

    use crate::operation_registry::OperationCancelRequest;
    use crate::{AppState, service::SigillumService};

    use super::{WalletResumeProgress, latest_wallet_resume_progress};
    use crate::service::inventory::wallet_selection::DiscoveryWallet;

    #[test]
    fn cancellation_checkpoint_observes_operation_registry_signal() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("state should initialize"));
        let service = SigillumService::new(state.clone());
        let operation = state.start_operation("inventory_scan_evm", Vec::new());

        assert!(!service.discovery_scan_checkpoint(&operation).unwrap());
        assert!(matches!(
            state.request_operation_cancel(operation.id()),
            OperationCancelRequest::Signaled(_)
        ));
        assert!(service.discovery_scan_checkpoint(&operation).unwrap());
    }

    #[test]
    fn terminalization_reloads_last_durable_job_snapshot() {
        let dir = TempDir::new().unwrap();
        let state =
            Arc::new(AppState::new(dir.path().to_path_buf()).expect("state should initialize"));
        let service = SigillumService::new(state);
        let durable_job = sample_job("running", 3);
        crate::inventory::save_wallet_inventory(
            dir.path(),
            &crate::inventory::WalletInventoryState {
                jobs: vec![durable_job.clone()],
                ..Default::default()
            },
        )
        .unwrap();

        // Model an observation that mutated only the runner's in-memory job.
        // The terminalizer must ignore it and start from the durable count.
        let mut in_memory_job = durable_job;
        in_memory_job.addresses_scanned = 99;

        let failed = service
            .finalize_terminal_discovery_scan(
                &in_memory_job.id,
                "failed",
                Some("injected provider failure"),
            )
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.addresses_scanned, 3);
        assert_eq!(
            failed.last_error.as_deref(),
            Some("injected provider failure")
        );
        assert!(failed.completed_at_unix.is_some());

        let reloaded = crate::inventory::load_wallet_inventory(dir.path()).unwrap();
        assert_eq!(reloaded.jobs, vec![failed]);
    }

    #[test]
    fn resume_skips_wallet_when_every_selected_provider_completed() {
        let wallet = sample_wallet();
        let providers = vec![sample_provider("provider-a"), sample_provider("provider-b")];
        let mut job = sample_job("canceled", 4);
        job.checkpoints = providers
            .iter()
            .map(|provider| sample_checkpoint(provider, true, 4, 1))
            .collect();

        assert_eq!(
            latest_wallet_resume_progress(&[job], &wallet, &providers),
            Some(WalletResumeProgress::Completed)
        );
    }

    #[test]
    fn resume_uses_most_conservative_incomplete_provider_cursor() {
        let wallet = sample_wallet();
        let providers = vec![sample_provider("provider-a"), sample_provider("provider-b")];
        let mut job = sample_job("failed", 7);
        job.checkpoints = vec![
            sample_checkpoint(&providers[0], false, 7, 2),
            sample_checkpoint(&providers[1], false, 5, 1),
        ];

        assert_eq!(
            latest_wallet_resume_progress(&[job], &wallet, &providers),
            Some(WalletResumeProgress::Continue {
                next_index: 5,
                consecutive_empty: 1,
            })
        );
    }

    fn sample_wallet() -> DiscoveryWallet {
        DiscoveryWallet {
            family: "eth-seed".into(),
            profile: "seed-a".into(),
            receive_path: "m/44'/60'/0'/0".into(),
            receive_xpub: "test-xpub".into(),
            derivation_pattern: "project".into(),
            account_index: 0,
        }
    }

    fn sample_provider(name: &str) -> EvmProviderProfile {
        EvmProviderProfile {
            name: name.into(),
            rpc_url: format!("http://localhost/{name}"),
            auth_token_key: None,
            compartment_id: 0,
            chain_id: 1,
            max_priority_fee_per_gas_hex: None,
            max_fee_per_gas_hex: None,
            native_gas_limit: None,
            erc20_gas_limit: None,
            fee_estimation_enabled: false,
        }
    }

    fn sample_checkpoint(
        provider: &EvmProviderProfile,
        completed: bool,
        next_index: u32,
        consecutive_empty: u32,
    ) -> WalletDiscoveryCheckpoint {
        WalletDiscoveryCheckpoint {
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-a".into(),
            provider_profile: provider.name.clone(),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            next_index,
            last_scanned_index: next_index.checked_sub(1),
            consecutive_empty,
            completed,
            updated_at_unix: 1,
        }
    }

    fn sample_job(status: &str, addresses_scanned: usize) -> WalletDiscoveryJob {
        WalletDiscoveryJob {
            id: "job-terminal".into(),
            status: status.into(),
            source: "local-rpc".into(),
            wallet_families: Vec::new(),
            wallet_profiles: Vec::new(),
            provider_profiles: Vec::new(),
            chain_ids: Vec::new(),
            gap_limit: 20,
            max_index: 100,
            addresses_scanned,
            active_addresses: 0,
            holdings_detected: 0,
            checkpoints: Vec::new(),
            block_cursors: Vec::new(),
            started_at_unix: 1,
            completed_at_unix: None,
            last_error: None,
            partition_providers: None,
            provider_partition_observations: Vec::new(),
        }
    }
}
