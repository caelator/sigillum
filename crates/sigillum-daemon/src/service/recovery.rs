//! Daemon startup recovery: normalize interrupted state before accepting requests.
//!
//! When the daemon starts (or restarts after a crash), the on-disk queue and
//! deposit state may be inconsistent. [`SigillumService::recover_runtime_state`]
//! performs four idempotent reconciliation passes:
//!
//! 1. **Recover pending operations** — scans the operations journal for
//!    incomplete entries, finalizes journals that are already recoverable from
//!    on-disk state, and leaves only genuinely unresolved operations pending.
//! 2. **Recover queue jobs** — transitions `deferred` jobs to `blocked`
//!    (clearing stale `next_attempt_after_unix`) and backfills missing retry
//!    schedules, so the queue processor can resume without manual intervention.
//! 3. **Reconcile deposits** — synchronizes each deposit's cached
//!    `queue_job_state` and `status` with the canonical queue state, ensuring
//!    the UI and API reflect the true job lifecycle.
//! 4. **Expose startup recovery telemetry** — records how many operations were
//!    discovered, recovered, and left unresolved for diagnostics.
//!
//! This runs exactly once at startup, before the HTTP listener opens.

use std::io;
use std::path::{Path, PathBuf};

use sigillum_fido2::config::Fido2Config;

use crate::operations::{PendingOperation, PendingOperationSpec};
use crate::state::StartupRecoverySummary;

use super::SigillumService;

impl SigillumService {
    pub(crate) fn recover_runtime_state(&self) -> Result<StartupRecoverySummary, io::Error> {
        let (interrupted_operation_count, recovered_operation_count, unresolved_operation_count) =
            recover_pending_operations(&self.state.base_dir)?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)?;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)?;
        let mut recovered_queue_job_count = 0usize;

        for job in &mut queue.jobs {
            if super::queue::recover_queue_job(job) {
                recovered_queue_job_count += 1;
            }
        }

        let reconciled_deposit_count =
            super::deposits::sync_eth_stealth_deposits_with_queue(&mut deposits, &queue);

        if recovered_queue_job_count > 0 {
            crate::queue_store::save_queue(&self.state.base_dir, &queue)?;
        }
        if reconciled_deposit_count > 0 {
            crate::deposits::save_deposits(&self.state.base_dir, &deposits)?;
        }

        let summary = StartupRecoverySummary {
            interrupted_operation_count,
            recovered_operation_count,
            unresolved_operation_count,
            recovered_queue_job_count,
            reconciled_deposit_count,
        };
        self.state.set_startup_recovery_summary(summary);
        Ok(summary)
    }
}

fn recover_pending_operations(base_dir: &Path) -> Result<(usize, usize, usize), io::Error> {
    let pending = crate::operations::list_pending_operations(base_dir)?;
    let interrupted = pending.len();
    let mut recovered = 0usize;
    let mut unresolved = 0usize;

    for operation in pending {
        if recover_pending_operation(base_dir, &operation)? {
            recovered += 1;
        } else {
            unresolved += 1;
        }
    }

    Ok((interrupted, recovered, unresolved))
}

fn recover_pending_operation(
    base_dir: &Path,
    operation: &PendingOperation,
) -> Result<bool, io::Error> {
    let recovered = match &operation.spec {
        PendingOperationSpec::SnapshotRestore { .. } => recover_snapshot_operation(base_dir)?,
        PendingOperationSpec::CompartmentRemove { id } => {
            recover_compartment_remove(base_dir, *id)?
        }
        PendingOperationSpec::CompartmentAdd { .. } => {
            recover_compartment_add(base_dir, operation)?
        }
        PendingOperationSpec::CompartmentInit { .. } => {
            recover_compartment_init(base_dir, operation)?
        }
        PendingOperationSpec::Fido2Setup {
            compartment_count, ..
        } => recover_fido2_setup(base_dir, *compartment_count)?,
        PendingOperationSpec::Fido2Register { .. } => recover_fido2_register(base_dir, operation)?,
        PendingOperationSpec::Fido2Remove { .. } => recover_fido2_remove(base_dir, operation)?,
    };

    if recovered {
        crate::operations::clear_pending_operation(base_dir, &operation.operation_id)?;
    }

    Ok(recovered)
}

fn recover_snapshot_operation(base_dir: &Path) -> Result<bool, io::Error> {
    let rollback = snapshot_temp_path(base_dir, "rollback");
    if rollback.exists() && snapshot_placeholder_dir(base_dir)? {
        std::fs::remove_dir_all(base_dir)?;
    }
    sigillum_core::recover_snapshot_restore(base_dir).map_err(vault_error_to_io)?;
    Ok(base_dir.join(".initialized").exists()
        && !snapshot_temp_path(base_dir, "restoring").exists()
        && !snapshot_temp_path(base_dir, "rollback").exists())
}

fn recover_compartment_remove(base_dir: &Path, id: usize) -> Result<bool, io::Error> {
    crate::state::recover_compartment_replacements(base_dir)?;
    let live = compartment_dir(base_dir, id);
    Ok(live.exists()
        && !live.with_extension("replacing").exists()
        && !live.with_extension("rollback").exists())
}

fn recover_compartment_add(
    base_dir: &Path,
    operation: &PendingOperation,
) -> Result<bool, io::Error> {
    let Some(id) = operation_compartment_id(operation) else {
        return Ok(false);
    };
    let dir = compartment_dir(base_dir, id);
    if !dir.exists() {
        return Ok(true);
    }
    Ok(compartment_files_present(&dir, &["vault.enc", "meta.enc"]))
}

fn recover_compartment_init(
    base_dir: &Path,
    operation: &PendingOperation,
) -> Result<bool, io::Error> {
    let Some(id) = operation_compartment_id(operation) else {
        return Ok(false);
    };
    let dir = compartment_dir(base_dir, id);
    let initialized = base_dir.join(".initialized").exists();
    if !dir.exists() {
        return Ok(!initialized);
    }
    Ok(initialized
        && compartment_files_present(
            &dir,
            &[
                "vault.enc",
                "meta.enc",
                "passphrase.salt",
                "passphrase_wrapped_key.enc",
            ],
        ))
}

fn recover_fido2_setup(base_dir: &Path, compartment_count: usize) -> Result<bool, io::Error> {
    let config = load_fido2_config(base_dir)?;
    if !config.is_fido2_enabled() {
        return Ok(!base_dir.join(".initialized").exists());
    }
    Ok(base_dir.join(".initialized").exists()
        && count_compartments_with_file(base_dir, "meta.enc") >= compartment_count)
}

fn recover_fido2_register(
    base_dir: &Path,
    operation: &PendingOperation,
) -> Result<bool, io::Error> {
    if operation.subject.is_none() {
        return Ok(false);
    }
    let _ = load_fido2_config(base_dir)?;
    Ok(true)
}

fn recover_fido2_remove(base_dir: &Path, operation: &PendingOperation) -> Result<bool, io::Error> {
    let Some(label) = operation.subject.as_deref() else {
        return Ok(false);
    };
    let config = load_fido2_config(base_dir)?;
    Ok(config.keys.iter().all(|key| key.label != label))
}

fn load_fido2_config(base_dir: &Path) -> Result<Fido2Config, io::Error> {
    sigillum_fido2::config::load_config(&base_dir.join("fido2_keys.json"))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

fn count_compartments_with_file(base_dir: &Path, file_name: &str) -> usize {
    let compartments_dir = base_dir.join("compartments");
    let Ok(entries) = std::fs::read_dir(compartments_dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(file_name).exists())
        .count()
}

fn compartment_files_present(dir: &Path, file_names: &[&str]) -> bool {
    file_names.iter().all(|name| dir.join(name).exists())
}

fn operation_compartment_id(operation: &PendingOperation) -> Option<usize> {
    operation
        .subject
        .as_deref()
        .and_then(|subject| subject.strip_prefix("compartment/"))
        .and_then(|id| id.parse::<usize>().ok())
}

fn snapshot_temp_path(base_dir: &Path, suffix: &str) -> PathBuf {
    let parent = base_dir.parent().unwrap_or(Path::new("."));
    let name = base_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sigillum".into());
    parent.join(format!(".{name}.{suffix}"))
}

fn compartment_dir(base_dir: &Path, id: usize) -> PathBuf {
    base_dir.join("compartments").join(id.to_string())
}

fn snapshot_placeholder_dir(base_dir: &Path) -> Result<bool, io::Error> {
    let mut entries = match std::fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };

    entries.try_fold(true, |is_placeholder, entry| {
        let entry = entry?;
        Ok(is_placeholder && entry.file_name() == ".ops")
    })
}

fn vault_error_to_io(error: sigillum_core::VaultError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sigillum_api::{EthStealthDeposit, QueueJob, QueueJobPayload};
    use sigillum_fido2::config::{Fido2Config, RegisteredKey};
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;

    fn sample_job(state: &str, next_attempt_after_unix: Option<u64>) -> QueueJob {
        QueueJob {
            id: "job-1".into(),
            state: state.into(),
            attempts: 1,
            created_at_unix: 1,
            updated_at_unix: 2,
            next_attempt_after_unix,
            payload: QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "payments".into(),
                stealth_address: "0x0000000000000000000000000000000000000001".into(),
                ephemeral_public_key_hex: "03".repeat(33),
                destination_address: Some("0x0000000000000000000000000000000000000002".into()),
                min_value_wei_hex: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: Some("not ready".into()),
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: Some("0xfeed".into()),
        }
    }

    fn sample_deposit(job_id: &str) -> EthStealthDeposit {
        EthStealthDeposit {
            id: "dep-1".into(),
            status: "pending".into(),
            asset_kind: "native".into(),
            wallet_profile: "payments".into(),
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: "payments".into(),
            short_name: "eth".into(),
            stealth_meta_address: "st:eth:example".into(),
            stealth_address: "0x0000000000000000000000000000000000000001".into(),
            ephemeral_public_key_hex: "03".repeat(33),
            view_tag_hex: "0xaa".into(),
            announcement: None,
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: Some("0x1".into()),
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: true,
            sweep_destination_address: None,
            min_sweep_amount_hex: None,
            queue_job_id: Some(job_id.into()),
            queue_job_state: Some("queued".into()),
            note: None,
            created_at_unix: 1,
            updated_at_unix: 2,
            last_checked_at_unix: Some(2),
            broadcast_transaction_hash_hex: None,
            counterparty_id: None,
        }
    }

    #[test]
    fn startup_recovery_normalizes_queue_state_and_syncs_deposits() {
        let dir = TempDir::new().unwrap();
        let queue = crate::queue_store::QueueState {
            jobs: vec![sample_job("deferred", Some(99))],
        };
        crate::queue_store::save_queue(dir.path(), &queue).unwrap();

        let deposits = crate::deposits::DepositState {
            eth_stealth: vec![sample_deposit("job-1")],
        };
        crate::deposits::save_deposits(dir.path(), &deposits).unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state.clone());

        let summary = service.recover_runtime_state().unwrap();
        assert_eq!(summary.interrupted_operation_count, 0);
        assert_eq!(summary.recovered_operation_count, 0);
        assert_eq!(summary.unresolved_operation_count, 0);
        assert_eq!(summary.recovered_queue_job_count, 1);
        assert_eq!(summary.reconciled_deposit_count, 1);
        assert_eq!(state.startup_recovery_summary(), summary);

        let queue = crate::queue_store::load_queue(dir.path()).unwrap();
        assert_eq!(queue.jobs[0].state, "blocked");
        assert_eq!(queue.jobs[0].next_attempt_after_unix, None);

        let deposits = crate::deposits::load_deposits(dir.path()).unwrap();
        assert_eq!(
            deposits.eth_stealth[0].queue_job_state.as_deref(),
            Some("blocked")
        );
        assert_eq!(deposits.eth_stealth[0].status, "sweep_blocked");
        assert_eq!(
            deposits.eth_stealth[0]
                .broadcast_transaction_hash_hex
                .as_deref(),
            Some("0xfeed")
        );
    }

    #[test]
    fn startup_recovery_backfills_missing_retry_schedule() {
        let dir = TempDir::new().unwrap();
        let queue = crate::queue_store::QueueState {
            jobs: vec![sample_job("retrying", None)],
        };
        crate::queue_store::save_queue(dir.path(), &queue).unwrap();

        let state = Arc::new(AppState::new(dir.path().to_path_buf()));
        let service = SigillumService::new(state.clone());

        let summary = service.recover_runtime_state().unwrap();
        assert_eq!(summary.recovered_operation_count, 0);
        assert_eq!(summary.recovered_queue_job_count, 1);

        let queue = crate::queue_store::load_queue(dir.path()).unwrap();
        assert_eq!(queue.jobs[0].state, "retrying");
        assert!(queue.jobs[0].next_attempt_after_unix.is_some());
    }

    #[test]
    fn startup_recovery_finalizes_snapshot_journal_after_filesystem_recovery() {
        let root = TempDir::new().unwrap();
        let base = root.path().join("sigillum");
        let rollback = root.path().join(".sigillum.rollback");
        std::fs::create_dir_all(&rollback).unwrap();
        std::fs::write(rollback.join(".initialized"), b"1").unwrap();

        let _journal = crate::operations::begin_operation(
            &base,
            PendingOperationSpec::snapshot_restore(4),
            Some("vault".into()),
        )
        .unwrap();

        let state = Arc::new(AppState::new(base.clone()));
        let service = SigillumService::new(state.clone());

        let summary = service.recover_runtime_state().unwrap();
        assert_eq!(summary.interrupted_operation_count, 1);
        assert_eq!(summary.recovered_operation_count, 1);
        assert_eq!(summary.unresolved_operation_count, 0);
        assert_eq!(state.pending_operation_count(), 0);
        assert!(base.join(".initialized").exists());
    }

    #[test]
    fn startup_recovery_finalizes_compartment_remove_journal_after_rollback_recovery() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_path_buf();
        let compartments = base.join("compartments");
        std::fs::create_dir_all(&compartments).unwrap();
        let rollback = compartments.join("0.rollback");
        std::fs::create_dir_all(&rollback).unwrap();
        std::fs::write(rollback.join("meta.enc"), b"meta").unwrap();
        std::fs::write(rollback.join("vault.enc"), b"vault").unwrap();

        let _journal = crate::operations::begin_operation(
            &base,
            PendingOperationSpec::compartment_remove(0),
            Some("compartment/0".into()),
        )
        .unwrap();

        let state = Arc::new(AppState::new(base.clone()));
        let service = SigillumService::new(state.clone());

        let summary = service.recover_runtime_state().unwrap();
        assert_eq!(summary.interrupted_operation_count, 1);
        assert_eq!(summary.recovered_operation_count, 1);
        assert_eq!(summary.unresolved_operation_count, 0);
        assert_eq!(state.pending_operation_count(), 0);
        assert!(compartments.join("0").exists());
        assert!(!rollback.exists());
    }

    #[test]
    fn startup_recovery_finalizes_fido2_register_journal_when_key_is_present() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_path_buf();
        let _journal = crate::operations::begin_operation(
            &base,
            PendingOperationSpec::fido2_register(None),
            Some("backup-key".into()),
        )
        .unwrap();

        let config = Fido2Config {
            total_shares: 1,
            keys: vec![RegisteredKey {
                label: "backup-key".into(),
                credential_id_hex: "11".repeat(16),
                public_key_der_hex: "22".repeat(16),
                public_key_pem: "pem".into(),
                shards: vec!["00".into(); sigillum_fido2::config::SHARD_SLOTS],
                registered_at: "2026-03-22T00:00:00Z".into(),
            }],
        };
        sigillum_fido2::config::save_config(&base.join("fido2_keys.json"), &config).unwrap();

        let state = Arc::new(AppState::new(base.clone()));
        let service = SigillumService::new(state.clone());

        let summary = service.recover_runtime_state().unwrap();
        assert_eq!(summary.interrupted_operation_count, 1);
        assert_eq!(summary.recovered_operation_count, 1);
        assert_eq!(summary.unresolved_operation_count, 0);
        assert_eq!(state.pending_operation_count(), 0);
    }
}
