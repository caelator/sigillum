use sigillum_api::{WalletDiscoveryCheckpoint, WalletDiscoveryJob};

use super::wallet_selection::{DERIVATION_PATTERN_PROJECT, DiscoveryWallet};

pub(super) struct ScanCheckpointProgress {
    pub(super) next_index: u32,
    pub(super) last_scanned_index: Option<u32>,
    pub(super) consecutive_empty: u32,
    pub(super) completed: bool,
    pub(super) updated_at_unix: u64,
}

pub(super) fn latest_resume_checkpoint(
    jobs: &[WalletDiscoveryJob],
    wallet: &DiscoveryWallet,
    providers: &[sigillum_api::EvmProviderProfile],
) -> Option<(u32, u32)> {
    jobs.iter()
        .rev()
        .filter(|job| job.status != "completed")
        .flat_map(|job| job.checkpoints.iter())
        .filter(|checkpoint| {
            checkpoint.wallet_family == wallet.family
                && checkpoint.wallet_profile == wallet.profile
                && checkpoint_matches_derivation(checkpoint, wallet)
                && !checkpoint.completed
                && providers
                    .iter()
                    .any(|provider| provider.name == checkpoint.provider_profile)
        })
        .min_by_key(|checkpoint| checkpoint.next_index)
        .map(|checkpoint| (checkpoint.next_index, checkpoint.consecutive_empty))
}

pub(super) fn update_scan_checkpoint(
    checkpoints: &mut Vec<WalletDiscoveryCheckpoint>,
    wallet: &DiscoveryWallet,
    provider: &sigillum_api::EvmProviderProfile,
    progress: ScanCheckpointProgress,
) {
    let next = WalletDiscoveryCheckpoint {
        wallet_family: wallet.family.clone(),
        wallet_profile: wallet.profile.clone(),
        provider_profile: provider.name.clone(),
        derivation_pattern: Some(wallet.derivation_pattern.clone()),
        account_index: Some(wallet.account_index),
        next_index: progress.next_index,
        last_scanned_index: progress.last_scanned_index,
        consecutive_empty: progress.consecutive_empty,
        completed: progress.completed,
        updated_at_unix: progress.updated_at_unix,
    };
    if let Some(existing) = checkpoints.iter_mut().find(|existing| {
        existing.wallet_family == next.wallet_family
            && existing.wallet_profile == next.wallet_profile
            && existing.provider_profile == next.provider_profile
            && existing
                .derivation_pattern
                .as_deref()
                .unwrap_or(DERIVATION_PATTERN_PROJECT)
                == next
                    .derivation_pattern
                    .as_deref()
                    .unwrap_or(DERIVATION_PATTERN_PROJECT)
            && existing
                .account_index
                .unwrap_or(next.account_index.unwrap_or(0))
                == next.account_index.unwrap_or(0)
    }) {
        *existing = next;
    } else {
        checkpoints.push(next);
    }
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

pub(super) fn sync_inventory_job(
    inventory: &mut crate::inventory::WalletInventoryState,
    job: &WalletDiscoveryJob,
) {
    if let Some(existing) = inventory
        .jobs
        .iter_mut()
        .find(|existing| existing.id == job.id)
    {
        *existing = job.clone();
    } else {
        inventory.jobs.push(job.clone());
    }
}
