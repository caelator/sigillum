//! Operation journal pattern for crash-safe multi-step operations.
//!
//! ## Operation Journal Design
//!
//! Long-running operations (snapshot restore, compartment init, FIDO2 registration) are
//! recorded to disk before execution. The `OperationGuard` RAII type ensures completion
//! is recorded: if the operation panics or crashes, the journal file remains on disk.
//! On daemon restart, `service::recover_runtime_state()` scans for incomplete operations
//! and either completes them (if safe) or reports them for manual recovery.
//!
//! Journal entries contain:
//! - `operation_id`: Unique identifier (timestamp + random nonce) for deduplication
//! - `subject`: Human-readable label (compartment ID, key label, etc.)
//! - `spec`: Typed operation kind (SnapshotRestore, Fido2Setup, etc.)
//! - `started_at_unix`: Timestamp for age tracking
//!
//! ## OperationGuard: RAII for Crash Safety
//!
//! When an operation begins, `begin_operation()` writes the journal file and returns
//! an `OperationGuard`. Callers MUST call `guard.complete()` when the operation finishes.
//! If `complete()` is never called, the guard's `Drop` impl logs a warning — the journal
//! file persists, signaling an incomplete operation.
//!
//! This pattern prevents silent failures: even if the operation succeeded but we crashed
//! before cleanup, the next startup will detect it. Contrast this with ad-hoc error
//! handling: a forgotten error branch leaves the operation journal in an ambiguous state.
//!
//! ## Versioning and Legacy Migration
//!
//! Old operation journals (before schema versioning) are automatically migrated by
//! `PendingOperationSpec::from_legacy_details()`. This allows graceful rollout of
//! new operation kinds without breaking existing deployments.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::json_store::{JsonDocument, JsonSchema, decode_json_document, encode_json_document};

/// A pending operation recorded in the operation journal.
///
/// Persisted to `.ops/{operation_id}.json` and used to track in-progress multi-step
/// operations. On daemon startup, recovered operations trigger recovery logic.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOperation {
    /// Globally unique operation ID for deduplication and log correlation.
    pub operation_id: String,
    /// Optional human-readable label (compartment ID, key label, etc.) for logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Unix timestamp when the operation started, for age tracking and debugging.
    pub started_at_unix: u64,
    /// The operation kind and parameters, stored as a tagged enum.
    #[serde(flatten)]
    pub spec: PendingOperationSpec,
}

impl JsonDocument for PendingOperation {
    const SCHEMA: JsonSchema = JsonSchema::new("sigillum.pending-operation", 1);

    fn from_legacy_json(path: &Path, value: serde_json::Value) -> Result<Self, std::io::Error> {
        let legacy: LegacyPendingOperation = serde_json::from_value(value).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "failed to parse legacy {} document {}: {error}",
                    Self::SCHEMA.name,
                    path.display()
                ),
            )
        })?;

        Ok(Self {
            operation_id: legacy.operation_id,
            subject: legacy.subject,
            started_at_unix: legacy.started_at_unix,
            spec: PendingOperationSpec::from_legacy_details(path, legacy.kind, legacy.details)?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "details")]
pub enum PendingOperationSpec {
    #[serde(rename = "snapshot.restore")]
    SnapshotRestore { snapshot_bytes: usize },
    #[serde(rename = "compartment.add")]
    CompartmentAdd { label: String, threshold: usize },
    #[serde(rename = "compartment.remove")]
    CompartmentRemove { id: usize },
    #[serde(rename = "compartment.init")]
    CompartmentInit {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        threshold: Option<usize>,
    },
    #[serde(rename = "fido2.setup")]
    Fido2Setup {
        label: String,
        compartment_count: usize,
    },
    #[serde(rename = "fido2.register")]
    Fido2Register {
        #[serde(skip_serializing_if = "Option::is_none")]
        poison: Option<bool>,
    },
    #[serde(rename = "fido2.remove")]
    Fido2Remove { skip_keys: Vec<String> },
}

impl PendingOperationSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SnapshotRestore { .. } => "snapshot.restore",
            Self::CompartmentAdd { .. } => "compartment.add",
            Self::CompartmentRemove { .. } => "compartment.remove",
            Self::CompartmentInit { .. } => "compartment.init",
            Self::Fido2Setup { .. } => "fido2.setup",
            Self::Fido2Register { .. } => "fido2.register",
            Self::Fido2Remove { .. } => "fido2.remove",
        }
    }

    pub fn snapshot_restore(snapshot_bytes: usize) -> Self {
        Self::SnapshotRestore { snapshot_bytes }
    }

    pub fn compartment_add(label: impl Into<String>, threshold: usize) -> Self {
        Self::CompartmentAdd {
            label: label.into(),
            threshold,
        }
    }

    pub fn compartment_remove(id: usize) -> Self {
        Self::CompartmentRemove { id }
    }

    pub fn compartment_init(label: Option<String>, threshold: Option<usize>) -> Self {
        Self::CompartmentInit { label, threshold }
    }

    pub fn fido2_setup(label: impl Into<String>, compartment_count: usize) -> Self {
        Self::Fido2Setup {
            label: label.into(),
            compartment_count,
        }
    }

    pub fn fido2_register(poison: Option<bool>) -> Self {
        Self::Fido2Register { poison }
    }

    pub fn fido2_remove(skip_keys: Vec<String>) -> Self {
        Self::Fido2Remove { skip_keys }
    }

    fn from_legacy_details(
        path: &Path,
        kind: String,
        details: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        match kind.as_str() {
            "snapshot.restore" => {
                let details = parse_legacy_details::<SnapshotRestoreDetails>(path, &kind, details)?;
                Ok(Self::SnapshotRestore {
                    snapshot_bytes: details.snapshot_bytes,
                })
            }
            "compartment.add" => {
                let details = parse_legacy_details::<CompartmentAddDetails>(path, &kind, details)?;
                Ok(Self::CompartmentAdd {
                    label: details.label,
                    threshold: details.threshold,
                })
            }
            "compartment.remove" => {
                let details =
                    parse_legacy_details::<CompartmentRemoveDetails>(path, &kind, details)?;
                Ok(Self::CompartmentRemove { id: details.id })
            }
            "compartment.init" => {
                let details = parse_legacy_details::<CompartmentInitDetails>(path, &kind, details)?;
                Ok(Self::CompartmentInit {
                    label: details.label,
                    threshold: details.threshold,
                })
            }
            "fido2.setup" => {
                let details = parse_legacy_details::<Fido2SetupDetails>(path, &kind, details)?;
                Ok(Self::Fido2Setup {
                    label: details.label,
                    compartment_count: details.compartments,
                })
            }
            "fido2.register" => {
                let details = parse_legacy_details::<Fido2RegisterDetails>(path, &kind, details)?;
                Ok(Self::Fido2Register {
                    poison: details.poison,
                })
            }
            "fido2.remove" => {
                let details = parse_legacy_details::<Fido2RemoveDetails>(path, &kind, details)?;
                Ok(Self::Fido2Remove {
                    skip_keys: details.skip_keys,
                })
            }
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "unsupported pending operation kind {} in {}",
                    other,
                    path.display()
                ),
            )),
        }
    }
}

/// RAII guard that ensures operation journal cleanup on completion.
///
/// When an operation finishes successfully, `complete()` must be called to delete
/// the journal file. If the guard is dropped without calling `complete()`, a warning
/// is logged and the journal file persists, signaling an incomplete operation.
///
/// This design ensures crash-safety: even if the process dies immediately after
/// the operation succeeds but before cleanup, the next startup will find the journal
/// and can safely retry cleanup or validate the operation's effects.
pub struct OperationGuard {
    path: PathBuf,
    operation_id: String,
    completed: bool,
}

impl OperationGuard {
    /// Return the operation ID persisted in this guard's journal.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Mark the operation as completed and delete the journal file.
    ///
    /// Returns `Ok(())` if the file was deleted or already doesn't exist.
    /// Returns `Err` only for real I/O errors (permission denied, disk full, etc).
    pub fn complete(mut self) -> Result<(), std::io::Error> {
        remove_operation_journal(&self.path)?;
        self.completed = true;
        Ok(())
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        if !self.completed {
            tracing::warn!(
                path = %self.path.display(),
                "OperationGuard dropped without calling complete()"
            );
        }
    }
}

pub fn begin_operation(
    base_dir: &Path,
    spec: PendingOperationSpec,
    subject: Option<String>,
) -> Result<OperationGuard, std::io::Error> {
    let ops_dir = operations_dir(base_dir);
    std::fs::create_dir_all(&ops_dir)?;

    let started_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let operation_id = new_operation_id(started_at_unix);
    let record = PendingOperation {
        operation_id: operation_id.clone(),
        subject,
        started_at_unix,
        spec,
    };
    let path = ops_dir.join(format!("{operation_id}.json"));
    let body = encode_json_document(&record)?;
    sigillum_core::utils::atomic_write(&path, &body)?;

    Ok(OperationGuard {
        path,
        operation_id,
        completed: false,
    })
}

pub fn list_pending_operations(base_dir: &Path) -> Result<Vec<PendingOperation>, std::io::Error> {
    let ops_dir = operations_dir(base_dir);
    let mut entries = match std::fs::read_dir(&ops_dir) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    entries.sort_by_key(|entry| entry.file_name());

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let record = decode_json_document(&path, &bytes)?;
        records.push(record);
    }
    Ok(records)
}

pub fn clear_pending_operation(base_dir: &Path, operation_id: &str) -> Result<(), std::io::Error> {
    let path = operations_dir(base_dir).join(format!("{operation_id}.json"));
    remove_operation_journal(&path)
}

fn remove_operation_journal(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => sync_parent_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sync_parent_directory(path: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

fn new_operation_id(started_at_unix: u64) -> String {
    let mut random = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut random);
    format!("{started_at_unix:016x}-{}", hex::encode(random))
}

fn operations_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(".ops")
}

fn parse_legacy_details<T>(
    path: &Path,
    kind: &str,
    details: serde_json::Value,
) -> Result<T, std::io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(details).map_err(|error| invalid_operation_data(path, kind, error))
}

fn invalid_operation_data(path: &Path, kind: &str, error: serde_json::Error) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "failed to parse pending operation {} in {}: {error}",
            kind,
            path.display()
        ),
    )
}

#[derive(Clone, Debug, Deserialize)]
struct LegacyPendingOperation {
    operation_id: String,
    kind: String,
    #[serde(default)]
    subject: Option<String>,
    started_at_unix: u64,
    #[serde(default)]
    details: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct SnapshotRestoreDetails {
    snapshot_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct CompartmentAddDetails {
    label: String,
    threshold: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct CompartmentRemoveDetails {
    id: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct CompartmentInitDetails {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    threshold: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
struct Fido2SetupDetails {
    label: String,
    compartments: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct Fido2RegisterDetails {
    #[serde(default)]
    poison: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
struct Fido2RemoveDetails {
    #[serde(default)]
    skip_keys: Vec<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn operation_journal_roundtrip() {
        let dir = TempDir::new().unwrap();
        let guard = begin_operation(
            dir.path(),
            PendingOperationSpec::snapshot_restore(4),
            Some("vault".into()),
        )
        .unwrap();

        let pending = list_pending_operations(dir.path()).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].spec.kind(), "snapshot.restore");
        assert_eq!(pending[0].subject.as_deref(), Some("vault"));
        assert_eq!(
            pending[0].spec,
            PendingOperationSpec::SnapshotRestore { snapshot_bytes: 4 }
        );

        guard.complete().unwrap();
        assert!(list_pending_operations(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn save_writes_versioned_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let _guard = begin_operation(
            dir.path(),
            PendingOperationSpec::compartment_add("default", 1),
            Some("compartment/0".into()),
        )
        .unwrap();

        let op_dir = operations_dir(dir.path());
        let entry = std::fs::read_dir(op_dir)
            .unwrap()
            .find_map(Result::ok)
            .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();

        assert_eq!(saved["schema"], json!("sigillum.pending-operation"));
        assert_eq!(saved["schema_version"], json!(1));
        assert_eq!(saved["data"]["kind"], json!("compartment.add"));
        assert_eq!(saved["data"]["details"]["label"], json!("default"));
    }

    #[test]
    fn legacy_unwrapped_records_still_load() {
        let dir = TempDir::new().unwrap();
        let op_dir = operations_dir(dir.path());
        std::fs::create_dir_all(&op_dir).unwrap();
        let path = op_dir.join("legacy.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "operation_id": "op-1",
                "kind": "fido2.setup",
                "subject": "fido2",
                "started_at_unix": 7,
                "details": {
                    "label": "primary",
                    "compartments": 3
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let pending = list_pending_operations(dir.path()).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation_id, "op-1");
        assert_eq!(pending[0].spec.kind(), "fido2.setup");
        assert_eq!(
            pending[0].spec,
            PendingOperationSpec::Fido2Setup {
                label: "primary".into(),
                compartment_count: 3,
            }
        );
    }

    #[test]
    fn unsupported_legacy_kind_returns_error() {
        let dir = TempDir::new().unwrap();
        let op_dir = operations_dir(dir.path());
        std::fs::create_dir_all(&op_dir).unwrap();
        let path = op_dir.join("legacy.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "operation_id": "op-1",
                "kind": "something.unknown",
                "started_at_unix": 7,
                "details": {}
            }))
            .unwrap(),
        )
        .unwrap();

        let error = list_pending_operations(dir.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("unsupported pending operation kind")
        );
    }
}
