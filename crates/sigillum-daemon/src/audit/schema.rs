pub(crate) const AUDIT_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at_unix INTEGER NOT NULL,
    kind TEXT NOT NULL,
    compartment_id INTEGER,
    key_name TEXT,
    event_json TEXT NOT NULL,
    source TEXT,
    source_line INTEGER,
    chain_scope TEXT,
    prev_mac TEXT,
    mac TEXT,
    verification_status TEXT NOT NULL DEFAULT 'legacy',
    UNIQUE(source, source_line)
);

CREATE INDEX IF NOT EXISTS idx_audit_events_created_at
    ON audit_events(created_at_unix DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_audit_events_kind
    ON audit_events(kind);
CREATE INDEX IF NOT EXISTS idx_audit_events_key_name
    ON audit_events(key_name);
CREATE INDEX IF NOT EXISTS idx_audit_events_chain_scope
    ON audit_events(chain_scope, id);

CREATE TABLE IF NOT EXISTS audit_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub(crate) const JSONL_MIGRATION_KEY: &str = "jsonl_migration_complete";
pub(crate) const JSONL_SOURCE_NAME: &str = "jsonl";
