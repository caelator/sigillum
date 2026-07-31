//! Encrypted snapshot export and restore for vault backup.
//!
//! Provides password-protected encryption and decryption of complete
//! vault snapshots for backup and disaster recovery.

use sigillum_api::{
    PassphraseRequest, SetupResetRequest, SetupResetResponse, SnapshotExportResponse,
    SnapshotRestoreRequest, SnapshotRestoreResponse,
};
use sigillum_core::{
    export_encrypted_snapshot, inspect_encrypted_snapshot, restore_encrypted_snapshot,
};
use zeroize::Zeroizing;

use crate::audit_log::AuditEventSpec;
use crate::operations::PendingOperationSpec;

use super::{ServiceError, ServiceResult, SigillumService};

const SETUP_RESET_CONFIRMATION: &str = "RESET LOCAL SIGILLUM DATA";

/// Move the data directory to a timestamped sibling archive and recreate it
/// empty, returning the archive path when anything was archived.
///
/// Reset must never destroy key material: the most likely operator resetting
/// is one who is locked out and frustrated, which is exactly when an
/// irreversible delete of a vault (that a hardware key or remembered
/// passphrase could still open later) would hurt the most. The archive stays
/// encrypted at rest exactly as it was and can be restored by pointing
/// `SIGILLUM_BASE_DIR` at it, or removed deliberately once the operator is
/// sure it holds nothing of value.
fn archive_base_dir(
    base_dir: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, std::io::Error> {
    if !base_dir.exists() {
        create_private_dir(base_dir)?;
        return Ok(None);
    }
    if std::fs::read_dir(base_dir)?.next().is_none() {
        return Ok(None);
    }

    let dir_name = base_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sigillum".to_string());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let mut archive_path = base_dir.with_file_name(format!("{dir_name}.archived-{timestamp}"));
    let mut suffix = 1u32;
    while archive_path.exists() {
        archive_path = base_dir.with_file_name(format!("{dir_name}.archived-{timestamp}-{suffix}"));
        suffix += 1;
    }

    std::fs::rename(base_dir, &archive_path)?;
    create_private_dir(base_dir)?;
    Ok(Some(archive_path))
}

fn create_private_dir(path: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

impl SigillumService {
    pub(crate) async fn backup_export(
        &self,
        token: Option<&str>,
        body: PassphraseRequest,
    ) -> ServiceResult<SnapshotExportResponse> {
        let token = self.require_session(token)?;
        if !self.state.is_initialized() {
            return Err(ServiceError::not_initialized(
                "Sigillum is not initialized.",
            ));
        }
        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        let session_context = self.capture_session_operation_context(Some(token))?;
        let _guard = self.acquire_session_operation(&session_context).await?;
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
        let session_context = if self.state.is_initialized() {
            let token = self.require_session(token)?;
            Some(self.capture_session_operation_context(Some(token))?)
        } else {
            None
        };

        let passphrase = Zeroizing::new(body.passphrase);
        super::helpers::require_valid_passphrase(&passphrase)?;

        let snapshot = hex::decode(&body.snapshot_hex).map_err(|error| {
            ServiceError::bad_request(format!("Invalid snapshot encoding: {error}"))
        })?;

        let _guard = if let Some(session_context) = session_context.as_ref() {
            self.acquire_session_operation(session_context).await?
        } else {
            let guard = self.state.operation_guard().await;
            if self.state.is_initialized() {
                return Err(ServiceError::unauthorized(
                    "Sigillum was initialized while snapshot restore was waiting; retry with a valid session.",
                ));
            }
            guard
        };
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
    ) -> ServiceResult<SetupResetResponse> {
        if body.confirmation != SETUP_RESET_CONFIRMATION {
            return Err(ServiceError::bad_request(format!(
                "Type '{SETUP_RESET_CONFIRMATION}' exactly to reset local Sigillum data."
            )));
        }

        let _guard = self.state.operation_guard().await;
        self.state.lock_all();
        self.state.reset_unlock_throttle();
        self.state.set_startup_recovery_summary(Default::default());
        let archived_to = archive_base_dir(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to reset local data: {error}"))
        })?;

        Ok(SetupResetResponse {
            status: "reset".into(),
            archived_to: archived_to.map(|path| path.display().to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, poll_fn};
    use std::sync::Arc;
    use std::task::Poll;

    use tempfile::TempDir;

    use crate::AppState;

    use super::*;

    #[tokio::test]
    async fn queued_first_run_restore_rejects_initialization_before_lock_acquisition() {
        let snapshot_dir = TempDir::new().unwrap();
        std::fs::write(snapshot_dir.path().join(".initialized"), b"snapshot").unwrap();
        std::fs::write(snapshot_dir.path().join("snapshot-only"), b"old vault").unwrap();
        let (snapshot, _) =
            export_encrypted_snapshot(snapshot_dir.path(), "snapshot passphrase").unwrap();

        let target_dir = TempDir::new().unwrap();
        let state = Arc::new(
            AppState::new(target_dir.path().to_path_buf()).expect("app state should initialize"),
        );
        let service = SigillumService::new(state.clone());

        let held_operation = state.operation_guard().await;
        let mut queued = Box::pin(service.backup_restore(
            None,
            SnapshotRestoreRequest {
                passphrase: "snapshot passphrase".into(),
                snapshot_hex: hex::encode(snapshot),
            },
        ));
        poll_fn(|context| match queued.as_mut().poll(context) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("restore bypassed the serialized mutation boundary"),
        })
        .await;

        std::fs::write(target_dir.path().join(".initialized"), b"new vault").unwrap();
        std::fs::write(target_dir.path().join("new-vault-only"), b"preserve").unwrap();
        drop(held_operation);

        let error = queued.await.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            std::fs::read(target_dir.path().join(".initialized")).unwrap(),
            b"new vault"
        );
        assert_eq!(
            std::fs::read(target_dir.path().join("new-vault-only")).unwrap(),
            b"preserve"
        );
        assert!(!target_dir.path().join("snapshot-only").exists());
        assert_eq!(state.pending_operation_count(), 0);
    }
}
