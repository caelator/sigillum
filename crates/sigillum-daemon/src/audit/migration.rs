use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::audit::schema::JSONL_SOURCE_NAME;
use crate::audit_db;
use crate::audit_log::StoredAuditEvent;
use crate::json_store::decode_json_document;

pub(crate) fn migrate_jsonl_to_sqlite(base_dir: &Path) -> Result<(), std::io::Error> {
    let db_path = base_dir.join("audit.db");
    let legacy_path = base_dir.join("audit.log");
    let migrated_path = migrated_jsonl_path(&legacy_path);

    let mut connection = audit_db::open_database(&db_path)?;
    if audit_db::migration_complete(&connection)? {
        return Ok(());
    }

    if !legacy_path.exists() {
        return audit_db::mark_migration_complete(&connection);
    }

    let file = std::fs::File::open(&legacy_path)?;
    let transaction = connection.transaction().map_err(std::io::Error::other)?;
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let event = match decode_json_document::<StoredAuditEvent>(&legacy_path, line.as_bytes()) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    line = idx + 1,
                    path = %legacy_path.display(),
                    "skipping malformed legacy audit line during sqlite migration"
                );
                continue;
            }
        };

        audit_db::insert_event(
            &transaction,
            &event,
            Some(JSONL_SOURCE_NAME),
            Some((idx + 1) as i64),
        )?;
    }
    transaction.commit().map_err(std::io::Error::other)?;

    if migrated_path.exists() {
        std::fs::remove_file(&migrated_path)?;
    }
    std::fs::rename(&legacy_path, &migrated_path)?;
    audit_db::mark_migration_complete(&connection)?;
    Ok(())
}

fn migrated_jsonl_path(path: &Path) -> PathBuf {
    path.with_extension("log.migrated")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;
    use crate::audit_db::{AuditQuery, query_events};
    use crate::audit_log::{AuditEventSpec, StoredAuditEvent, append_audit_event};

    #[test]
    fn migration_imports_legacy_jsonl_and_marks_completion() {
        let dir = TempDir::new().unwrap();
        let legacy = dir.path().join("audit.log");

        append_audit_event(
            &legacy,
            &StoredAuditEvent {
                created_at_unix: 7,
                compartment_id: Some(0),
                spec: AuditEventSpec::SecretSet {
                    key: "db_pass".into(),
                },
            },
        )
        .unwrap();

        migrate_jsonl_to_sqlite(dir.path()).unwrap();
        assert!(!legacy.exists());
        assert!(dir.path().join("audit.log.migrated").exists());

        let events = query_events(
            &dir.path().join("audit.db"),
            &AuditQuery {
                tail: 10,
                kind: None,
                since: None,
                key: None,
            },
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "secret.set");

        migrate_jsonl_to_sqlite(dir.path()).unwrap();
        let events = query_events(
            &dir.path().join("audit.db"),
            &AuditQuery {
                tail: 10,
                kind: None,
                since: None,
                key: None,
            },
        )
        .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn migration_skips_corrupted_legacy_jsonl_lines() {
        let dir = TempDir::new().unwrap();
        let legacy = dir.path().join("audit.log");

        append_audit_event(
            &legacy,
            &StoredAuditEvent {
                created_at_unix: 7,
                compartment_id: Some(0),
                spec: AuditEventSpec::SecretSet {
                    key: "db_pass".into(),
                },
            },
        )
        .unwrap();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&legacy)
            .unwrap();
        writeln!(file, "{{ not valid json").unwrap();

        append_audit_event(
            &legacy,
            &StoredAuditEvent {
                created_at_unix: 9,
                compartment_id: None,
                spec: AuditEventSpec::LockAll,
            },
        )
        .unwrap();

        migrate_jsonl_to_sqlite(dir.path()).unwrap();

        let events = query_events(
            &dir.path().join("audit.db"),
            &AuditQuery {
                tail: 10,
                kind: None,
                since: None,
                key: None,
            },
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        let kinds = events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"secret.set"));
        assert!(kinds.contains(&"lock.all"));
    }
}
