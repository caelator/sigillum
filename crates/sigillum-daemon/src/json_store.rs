//! JSON document storage with schema versioning, backup recovery, and corruption resilience.
//!
//! ## Schema Versioning Strategy
//!
//! All documents are wrapped in an envelope with schema name and version:
//! ```json
//! {
//!   "schema": "sigillum.audit-event",
//!   "schema_version": 1,
//!   "data": { ... }
//! }
//! ```
//!
//! This enables safe schema evolution: new code can read old versions by implementing
//! migration logic in `JsonDocument::from_enveloped_json()`. Legacy unwrapped documents
//! (from versions before versioning was introduced) still load via `from_legacy_json()`.
//!
//! ## Backup/Recovery Protocol
//!
//! For every live document, a `.bak` copy is maintained:
//! 1. Write new version to live file atomically
//! 2. Sync backup to match (if they differ)
//! 3. On read: try live first; if corrupted, restore from backup
//! 4. Corrupt live files are quarantined with timestamp suffix
//!
//! This "dual write" pattern ensures that brief I/O errors or process crashes during
//! writes don't lose data. If both files are corrupted (extremely unlikely), the load
//! fails with a clear error indicating both paths were tried.
//!
//! ## Data Durability Rationale
//!
//! Why backups matter: Daemon crashes, power loss, or filesystem errors can partially
//! write a JSON file, leaving it syntactically invalid. The backup allows recovery
//! without losing state. The operation journal (in `operations` module) provides
//! crash-safe semantics for multi-step operations that depend on this file durability.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sigillum_core::utils::atomic_write;

/// Schema definition for a JSON document type.
///
/// The schema name must be a stable identifier (e.g., "sigillum.audit-event")
/// and the version must be incremented when the data structure changes incompatibly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JsonSchema {
    pub name: &'static str,
    pub version: u32,
}

impl JsonSchema {
    pub const fn new(name: &'static str, version: u32) -> Self {
        Self { name, version }
    }
}

/// Trait for JSON documents that implement schema versioning and legacy migration.
///
/// Implementors define a constant SCHEMA and handle deserialization of both
/// envelope-wrapped (versioned) and legacy unwrapped documents.
pub(crate) trait JsonDocument: Serialize + DeserializeOwned + Sized {
    const SCHEMA: JsonSchema;

    /// Deserialize a versioned envelope. Return an error if the schema version
    /// is not supported by this code. Most implementations will pass the data
    /// directly to serde if the version matches, or return a migration error.
    fn from_enveloped_json(
        path: &Path,
        version: u32,
        data: serde_json::Value,
    ) -> Result<Self, std::io::Error> {
        if version != Self::SCHEMA.version {
            return Err(invalid_data(format!(
                "unsupported {} schema version {} in {}; expected {}",
                Self::SCHEMA.name,
                version,
                path.display(),
                Self::SCHEMA.version
            )));
        }

        serde_json::from_value(data).map_err(|error| {
            invalid_data(format!(
                "failed to parse {} schema payload {}: {error}",
                Self::SCHEMA.name,
                path.display()
            ))
        })
    }

    /// Deserialize a legacy (pre-envelope) JSON value. This allows reading documents
    /// created before the versioning envelope was introduced. Implementations typically
    /// just call serde_json::from_value, or perform a migration if the legacy format
    /// was substantially different.
    fn from_legacy_json(path: &Path, value: serde_json::Value) -> Result<Self, std::io::Error> {
        serde_json::from_value(value).map_err(|error| {
            invalid_data(format!(
                "failed to parse legacy {} document {}: {error}",
                Self::SCHEMA.name,
                path.display()
            ))
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JsonStoreEnvelope<T> {
    schema: String,
    schema_version: u32,
    data: T,
}

pub(crate) fn load_json_document<T>(path: &Path) -> Result<Option<T>, std::io::Error>
where
    T: JsonDocument,
{
    load_with_parser(path, decode_json_document::<T>)
}

pub(crate) fn save_json_document<T>(path: &Path, value: &T) -> Result<(), std::io::Error>
where
    T: JsonDocument,
{
    let body = encode_json_document(value)?;
    save_json_bytes(path, &body)
}

pub(crate) fn encode_json_document<T>(value: &T) -> Result<Vec<u8>, std::io::Error>
where
    T: JsonDocument,
{
    encode_json_document_with(value, true)
}

pub(crate) fn encode_json_document_compact<T>(value: &T) -> Result<Vec<u8>, std::io::Error>
where
    T: JsonDocument,
{
    encode_json_document_with(value, false)
}

fn encode_json_document_with<T>(value: &T, pretty: bool) -> Result<Vec<u8>, std::io::Error>
where
    T: JsonDocument,
{
    let envelope = JsonStoreEnvelope {
        schema: T::SCHEMA.name.to_string(),
        schema_version: T::SCHEMA.version,
        data: value,
    };

    let encoded = if pretty {
        serde_json::to_vec_pretty(&envelope)
    } else {
        serde_json::to_vec(&envelope)
    };

    encoded.map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

pub(crate) fn decode_json_document<T>(path: &Path, bytes: &[u8]) -> Result<T, std::io::Error>
where
    T: JsonDocument,
{
    parse_json_document(path, bytes)
}

fn load_with_parser<T>(
    path: &Path,
    parser: impl Fn(&Path, &[u8]) -> Result<T, std::io::Error>,
) -> Result<Option<T>, std::io::Error> {
    match std::fs::read(path) {
        Ok(live_bytes) => match parser(path, &live_bytes) {
            Ok(value) => {
                sync_backup(path, &live_bytes);
                Ok(Some(value))
            }
            Err(live_error) => match read_backup(path)? {
                Some(backup_bytes) => match parser(&backup_path(path), &backup_bytes) {
                    Ok(value) => {
                        quarantine_corrupt_file(path)?;
                        atomic_write(path, &backup_bytes)?;
                        Ok(Some(value))
                    }
                    Err(backup_error) => Err(invalid_data(format!(
                        "failed to parse {} and recovery backup {}: {}; backup error: {}",
                        path.display(),
                        backup_path(path).display(),
                        live_error,
                        backup_error
                    ))),
                },
                None => Err(live_error),
            },
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match read_backup(path)? {
            Some(backup_bytes) => {
                let value = parser(&backup_path(path), &backup_bytes)?;
                atomic_write(path, &backup_bytes)?;
                Ok(Some(value))
            }
            None => Ok(None),
        },
        Err(error) => Err(error),
    }
}

fn save_json_bytes(path: &Path, body: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, body)?;
    sync_backup(path, body);
    Ok(())
}

pub(crate) fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.bak",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state.json")
    ))
}

fn read_backup(path: &Path) -> Result<Option<Vec<u8>>, std::io::Error> {
    let backup = backup_path(path);
    match std::fs::read(&backup) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_json_document<T>(path: &Path, bytes: &[u8]) -> Result<T, std::io::Error>
where
    T: JsonDocument,
{
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_data(format!("failed to parse {}: {error}", path.display())))?;

    if looks_like_envelope(&value) {
        let envelope: JsonStoreEnvelope<serde_json::Value> = serde_json::from_value(value)
            .map_err(|error| {
                invalid_data(format!(
                    "failed to parse versioned json document {}: {error}",
                    path.display()
                ))
            })?;

        if envelope.schema != T::SCHEMA.name {
            return Err(invalid_data(format!(
                "unexpected schema {} in {}; expected {}",
                envelope.schema,
                path.display(),
                T::SCHEMA.name
            )));
        }

        return T::from_enveloped_json(path, envelope.schema_version, envelope.data);
    }

    T::from_legacy_json(path, value)
}

fn looks_like_envelope(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.contains_key("schema") || object.contains_key("schema_version")
    })
}

fn sync_backup(path: &Path, bytes: &[u8]) {
    let backup = backup_path(path);
    match std::fs::read(&backup) {
        Ok(existing) if existing == bytes => return,
        Ok(_) | Err(_) => {}
    }

    if let Err(error) = atomic_write(&backup, bytes) {
        tracing::warn!(
            path = %path.display(),
            backup = %backup.display(),
            %error,
            "failed to refresh json store backup"
        );
    }
}

fn quarantine_corrupt_file(path: &Path) -> Result<PathBuf, std::io::Error> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut candidate = path.with_file_name(format!("{file_name}.corrupt-{stamp}"));
    let mut suffix = 0usize;
    while candidate.exists() {
        suffix += 1;
        candidate = path.with_file_name(format!("{file_name}.corrupt-{stamp}-{suffix}"));
    }
    std::fs::rename(path, &candidate)?;
    Ok(candidate)
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Fixture {
        value: String,
    }

    impl JsonDocument for Fixture {
        const SCHEMA: JsonSchema = JsonSchema::new("sigillum.test.fixture", 1);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");

        assert_eq!(load_json_document::<Fixture>(&path).unwrap(), None);
    }

    #[test]
    fn save_roundtrip_creates_backup() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        save_json_document(&path, &fixture).unwrap();

        assert_eq!(
            load_json_document::<Fixture>(&path).unwrap(),
            Some(fixture.clone())
        );
        assert!(backup_path(&path).exists());
    }

    #[test]
    fn load_restores_missing_live_from_backup() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        save_json_document(&path, &fixture).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            load_json_document::<Fixture>(&path).unwrap(),
            Some(fixture.clone())
        );
        assert!(path.exists());
    }

    #[test]
    fn load_quarantines_corrupt_live_and_restores_backup() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        save_json_document(&path, &fixture).unwrap();
        std::fs::write(&path, b"not valid json {{{").unwrap();

        assert_eq!(
            load_json_document::<Fixture>(&path).unwrap(),
            Some(fixture.clone())
        );

        let corrupt_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .count();
        assert_eq!(corrupt_files, 1);
        assert_eq!(load_json_document::<Fixture>(&path).unwrap(), Some(fixture));
    }

    #[test]
    fn valid_live_rewrites_invalid_backup() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        save_json_document(&path, &fixture).unwrap();
        std::fs::write(backup_path(&path), b"not valid json {{{").unwrap();

        assert_eq!(
            load_json_document::<Fixture>(&path).unwrap(),
            Some(fixture.clone())
        );
        assert_eq!(
            load_json_document::<Fixture>(&backup_path(&path)).unwrap(),
            Some(fixture.clone())
        );
    }

    #[test]
    fn corrupted_live_and_backup_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");

        std::fs::write(&path, b"not valid json {{{").unwrap();
        std::fs::write(backup_path(&path), b"still not valid {{{").unwrap();

        let error = load_json_document::<Fixture>(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("recovery backup"));
    }

    #[test]
    fn legacy_unwrapped_documents_still_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        std::fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();

        assert_eq!(
            load_json_document::<Fixture>(&path).unwrap(),
            Some(fixture.clone())
        );
    }

    #[test]
    fn save_wraps_documents_in_schema_envelope() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        let fixture = Fixture {
            value: "hello".into(),
        };

        save_json_document(&path, &fixture).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["schema"], json!("sigillum.test.fixture"));
        assert_eq!(saved["schema_version"], json!(1));
        assert_eq!(saved["data"]["value"], json!("hello"));
    }

    #[test]
    fn mismatched_schema_is_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.other",
                "schema_version": 1,
                "data": { "value": "hello" }
            }))
            .unwrap(),
        )
        .unwrap();

        let error = load_json_document::<Fixture>(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unexpected schema"));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fixture.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema": "sigillum.test.fixture",
                "schema_version": 9,
                "data": { "value": "hello" }
            }))
            .unwrap(),
        )
        .unwrap();

        let error = load_json_document::<Fixture>(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unsupported"));
    }
}
