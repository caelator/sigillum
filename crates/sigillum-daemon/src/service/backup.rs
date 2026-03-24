//! Encrypted snapshot export and restore for vault backup.
//!
//! Provides password-protected encryption and decryption of complete
//! vault snapshots for backup and disaster recovery.

use sigillum_api::{
    PassphraseRequest, SetupResetRequest, SnapshotExportResponse, SnapshotRestoreRequest,
    SnapshotRestoreResponse, response::GenericStatusResponse,
};
use sigillum_core::{
    export_encrypted_snapshot, inspect_encrypted_snapshot, restore_encrypted_snapshot,
};
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;
use crate::operations::PendingOperationSpec;

use super::{ServiceError, ServiceResult, SigillumService};

const SETUP_RESET_CONFIRMATION: &str = "RESET LOCAL SIGILLUM DATA";

fn clear_base_dir(base_dir: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(base_dir)?;
    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

impl SigillumService {
    pub(crate) async fn backup_export(
        &self,
        token: Option<&str>,
        body: PassphraseRequest,
    ) -> ServiceResult<SnapshotExportResponse> {
        let _ = self.require_session(token)?;
        if !self.state.is_initialized() {
            return Err(ServiceError::not_found("Sigillum is not initialized."));
        }
        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        let _guard = self.state.operation_guard().await;
        let (snapshot, summary) =
            export_encrypted_snapshot(&self.state.base_dir, passphrase.as_str())
                .map_err(|error| Self::snapshot_error("Snapshot export failed", error))?;
        self.record_audit(
            None,
            AuditEventSpec::SnapshotExport {
                file_count: summary.file_count,
                total_bytes: summary.total_bytes,
            },
        )?;

        Ok(SnapshotExportResponse {
            status: "exported".into(),
            snapshot_hex: hex::encode(snapshot),
            summary,
        })
    }

    pub(crate) async fn backup_restore(
        &self,
        token: Option<&str>,
        body: SnapshotRestoreRequest,
    ) -> ServiceResult<SnapshotRestoreResponse> {
        if self.state.is_initialized() {
            let _ = self.require_session(token)?;
        }

        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        let snapshot = hex::decode(&body.snapshot_hex).map_err(|error| {
            ServiceError::bad_request(format!("Invalid snapshot encoding: {error}"))
        })?;

        let _guard = self.state.operation_guard().await;
        let journal = self.begin_operation(
            PendingOperationSpec::snapshot_restore(snapshot.len()),
            Some("vault".into()),
        )?;
        inspect_encrypted_snapshot(passphrase.as_str(), &snapshot)
            .map_err(|error| Self::snapshot_error("Snapshot restore failed", error))?;
        self.state.lock_all();
        let summary =
            restore_encrypted_snapshot(&self.state.base_dir, passphrase.as_str(), &snapshot)
                .map_err(|error| Self::snapshot_error("Snapshot restore failed", error))?;
        journal.complete().map_err(|error| {
            ServiceError::internal(format!("Failed to finalize operation: {error}"))
        })?;
        self.record_audit(
            None,
            AuditEventSpec::SnapshotRestore {
                file_count: summary.file_count,
                total_bytes: summary.total_bytes,
            },
        )?;

        Ok(SnapshotRestoreResponse {
            status: "restored".into(),
            summary,
            requires_reauth: true,
        })
    }

    pub(crate) async fn setup_reset(
        &self,
        body: SetupResetRequest,
    ) -> ServiceResult<GenericStatusResponse> {
        if body.confirmation != SETUP_RESET_CONFIRMATION {
            return Err(ServiceError::bad_request(format!(
                "Type '{SETUP_RESET_CONFIRMATION}' exactly to reset local Sigillum data."
            )));
        }

        let _guard = self.state.operation_guard().await;
        self.state.lock_all();
        self.state.reset_unlock_throttle();
        self.state.set_startup_recovery_summary(Default::default());
        clear_base_dir(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to reset local data: {error}"))
        })?;

        Ok(GenericStatusResponse {
            status: "reset".into(),
        })
    }
}
