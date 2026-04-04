use std::path::Path;
use std::time::Duration;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use sigillum_api::AuditEvent as PublicAuditEvent;

use crate::audit::schema::AUDIT_SCHEMA_SQL;
use crate::audit_log::StoredAuditEvent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuditQuery {
    pub(crate) tail: usize,
    pub(crate) kind: Option<String>,
    pub(crate) since: Option<u64>,
    pub(crate) key: Option<String>,
}

pub(crate) fn open_database(path: &Path) -> Result<Connection, std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let connection = Connection::open(path).map_err(to_io_error)?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(to_io_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(to_io_error)?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(to_io_error)?;
    connection
        .execute_batch(AUDIT_SCHEMA_SQL)
        .map_err(to_io_error)?;
    Ok(connection)
}

pub(crate) fn append_event(path: &Path, event: &StoredAuditEvent) -> Result<(), std::io::Error> {
    let connection = open_database(path)?;
    insert_event(&connection, event, None, None)
}

pub(crate) fn query_events(
    path: &Path,
    query: &AuditQuery,
) -> Result<Vec<PublicAuditEvent>, std::io::Error> {
    let connection = open_database(path)?;
    let mut sql = String::from("SELECT event_json FROM audit_events");
    let mut filters = Vec::new();
    let mut params = Vec::<SqlValue>::new();

    if let Some(kind) = &query.kind {
        filters.push("kind = ?");
        params.push(SqlValue::from(kind.clone()));
    }
    if let Some(since) = query.since {
        filters.push("created_at_unix >= ?");
        params.push(SqlValue::from(since as i64));
    }
    if let Some(key) = &query.key {
        filters.push("key_name = ?");
        params.push(SqlValue::from(key.clone()));
    }

    if !filters.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&filters.join(" AND "));
    }
    sql.push_str(" ORDER BY created_at_unix DESC, id DESC LIMIT ?");
    params.push(SqlValue::from(query.tail.max(1) as i64));

    let mut statement = connection.prepare(&sql).map_err(to_io_error)?;
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| row.get::<_, String>(0))
        .map_err(to_io_error)?;

    let mut events = Vec::new();
    for row in rows {
        let json = row.map_err(to_io_error)?;
        let event = serde_json::from_str::<PublicAuditEvent>(&json).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to decode audit event from sqlite: {error}"),
            )
        })?;
        events.push(event);
    }
    Ok(events)
}

pub(crate) fn migration_complete(connection: &Connection) -> Result<bool, std::io::Error> {
    let value = connection
        .query_row(
            "SELECT value FROM audit_meta WHERE key = ?1",
            params![crate::audit::schema::JSONL_MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(to_io_error)?;
    Ok(matches!(value.as_deref(), Some("complete")))
}

pub(crate) fn mark_migration_complete(connection: &Connection) -> Result<(), std::io::Error> {
    connection
        .execute(
            "INSERT INTO audit_meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![crate::audit::schema::JSONL_MIGRATION_KEY, "complete"],
        )
        .map_err(to_io_error)?;
    Ok(())
}

pub(crate) fn insert_event(
    connection: &Connection,
    event: &StoredAuditEvent,
    source: Option<&str>,
    source_line: Option<i64>,
) -> Result<(), std::io::Error> {
    let public_event = event.to_public_event();
    let serialized_event = serde_json::to_string(&public_event).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to serialize audit event: {error}"),
        )
    })?;
    connection
        .execute(
            "INSERT OR IGNORE INTO audit_events(
                created_at_unix,
                kind,
                compartment_id,
                key_name,
                event_json,
                source,
                source_line
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                public_event.created_at_unix as i64,
                public_event.kind,
                public_event.compartment_id.map(|value| value as i64),
                event.spec.indexed_key(),
                serialized_event,
                source,
                source_line
            ],
        )
        .map_err(to_io_error)?;
    Ok(())
}

fn to_io_error(error: rusqlite::Error) -> std::io::Error {
    std::io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::audit_log::AuditEventSpec;

    #[test]
    fn query_events_supports_kind_key_and_since_filters() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("audit.db");

        append_event(
            &db_path,
            &StoredAuditEvent {
                created_at_unix: 10,
                compartment_id: Some(0),
                spec: AuditEventSpec::SecretSet {
                    key: "db_pass".into(),
                },
            },
        )
        .unwrap();
        append_event(
            &db_path,
            &StoredAuditEvent {
                created_at_unix: 20,
                compartment_id: None,
                spec: AuditEventSpec::LockAll,
            },
        )
        .unwrap();

        let events = query_events(
            &db_path,
            &AuditQuery {
                tail: 10,
                kind: Some("secret.set".into()),
                since: Some(9),
                key: Some("db_pass".into()),
            },
        )
        .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "secret.set");
        assert_eq!(events[0].details["key"], serde_json::json!("db_pass"));
    }
}
