//! SQLite database layer — local connection handle, migrations, and typed queries.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::GatewayError;

pub const DEFAULT_PROJECT_SCOPES: &[&str] = &[
    "payments:create",
    "payments:read",
    "payments:list",
    "payments:cancel",
    "webhooks:read",
];

pub fn default_project_scopes() -> Vec<String> {
    DEFAULT_PROJECT_SCOPES
        .iter()
        .map(|scope| (*scope).to_string())
        .collect()
}

fn default_project_scopes_json() -> String {
    serde_json::to_string(DEFAULT_PROJECT_SCOPES).expect("default scopes serialize")
}

/// Lightweight cloneable database handle for the local sidecar.
#[derive(Clone)]
pub struct SqlitePool {
    inner: Arc<Mutex<Connection>>,
}

impl SqlitePool {
    fn new(connection: Connection) -> Self {
        Self {
            inner: Arc::new(Mutex::new(connection)),
        }
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Row types returned from queries.
pub mod row {
    use rusqlite::Row;
    use serde::Serialize;

    #[derive(Debug, Clone, Serialize)]
    pub struct Project {
        pub id: String,
        pub name: String,
        pub api_key_hash: String,
        pub wallet_profile: String,
        pub scopes: Vec<String>,
        pub webhook_url: Option<String>,
        pub webhook_secret: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    impl Project {
        pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
            let scopes_json: String = row.get("scopes_json")?;
            let scopes = serde_json::from_str(&scopes_json)
                .unwrap_or_else(|_| super::default_project_scopes());
            Ok(Self {
                id: row.get("id")?,
                name: row.get("name")?,
                api_key_hash: row.get("api_key_hash")?,
                wallet_profile: row.get("wallet_profile")?,
                scopes,
                webhook_url: row.get("webhook_url")?,
                webhook_secret: row.get("webhook_secret")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        }
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct Payment {
        pub id: String,
        pub project_id: String,
        pub idempotency_key: Option<String>,
        pub amount_wei: String,
        pub chain_id: i64,
        pub token_address: Option<String>,
        pub stealth_address: String,
        pub ephemeral_pub: String,
        pub view_tag: Option<String>,
        pub deposit_id: Option<String>,
        pub status: String,
        pub metadata_json: String,
        pub created_at: String,
        pub expires_at: Option<String>,
        pub confirmed_at: Option<String>,
        pub swept_at: Option<String>,
    }

    impl Payment {
        pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
            Ok(Self {
                id: row.get("id")?,
                project_id: row.get("project_id")?,
                idempotency_key: row.get("idempotency_key")?,
                amount_wei: row.get("amount_wei")?,
                chain_id: row.get("chain_id")?,
                token_address: row.get("token_address")?,
                stealth_address: row.get("stealth_address")?,
                ephemeral_pub: row.get("ephemeral_pub")?,
                view_tag: row.get("view_tag")?,
                deposit_id: row.get("deposit_id")?,
                status: row.get("status")?,
                metadata_json: row.get("metadata_json")?,
                created_at: row.get("created_at")?,
                expires_at: row.get("expires_at")?,
                confirmed_at: row.get("confirmed_at")?,
                swept_at: row.get("swept_at")?,
            })
        }
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct WebhookDelivery {
        pub id: i64,
        pub payment_id: String,
        pub event: String,
        pub url: String,
        pub status_code: Option<i32>,
        pub attempt: i32,
        pub response_body: Option<String>,
        pub created_at: String,
        pub next_retry_at: Option<String>,
    }

    impl WebhookDelivery {
        pub(super) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
            Ok(Self {
                id: row.get("id")?,
                payment_id: row.get("payment_id")?,
                event: row.get("event")?,
                url: row.get("url")?,
                status_code: row.get("status_code")?,
                attempt: row.get("attempt")?,
                response_body: row.get("response_body")?,
                created_at: row.get("created_at")?,
                next_retry_at: row.get("next_retry_at")?,
            })
        }
    }
}

pub struct NewWebhookDelivery<'a> {
    pub payment_id: &'a str,
    pub event: &'a str,
    pub url: &'a str,
    pub attempt: i32,
    pub status_code: Option<i32>,
    pub response_body: Option<&'a str>,
    pub next_retry_at: Option<&'a str>,
}

/// Open (or create) the SQLite database, enable WAL, and run the schema.
pub async fn connect(database_url: &str) -> Result<SqlitePool, GatewayError> {
    let connection = match sqlite_database_path(database_url) {
        Some(path) => Connection::open(path)?,
        None => Connection::open_in_memory()?,
    };

    connection.execute_batch("PRAGMA journal_mode=WAL;")?;
    connection.execute_batch(include_str!("../schema.sql"))?;
    migrate_project_scopes(&connection)?;

    Ok(SqlitePool::new(connection))
}

fn migrate_project_scopes(connection: &Connection) -> Result<(), rusqlite::Error> {
    let has_scopes = {
        let mut statement = connection.prepare("PRAGMA table_info(projects)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        columns
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|column| column == "scopes_json")
    };
    if !has_scopes {
        connection.execute(
            "ALTER TABLE projects ADD COLUMN scopes_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )?;
    }
    connection.execute(
        "UPDATE projects SET scopes_json = ? WHERE scopes_json IS NULL OR scopes_json = '' OR scopes_json = '[]'",
        params![default_project_scopes_json()],
    )?;
    Ok(())
}

fn sqlite_database_path(database_url: &str) -> Option<PathBuf> {
    let trimmed = database_url.trim();
    if trimmed == ":memory:" || trimmed == "sqlite::memory:" {
        return None;
    }

    let without_scheme = trimmed
        .strip_prefix("sqlite://")
        .or_else(|| trimmed.strip_prefix("sqlite:"))
        .unwrap_or(trimmed);
    let path = without_scheme
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(without_scheme);

    Some(PathBuf::from(if path.is_empty() {
        "gateway.db"
    } else {
        path
    }))
}

pub fn is_unique_constraint(error: &GatewayError) -> bool {
    matches!(
        error,
        GatewayError::Database(rusqlite::Error::SqliteFailure(code, _))
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

// ── Project queries ────────────────────────────────────────────────

pub async fn insert_project(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    api_key_hash: &str,
    wallet_profile: &str,
    webhook_url: Option<&str>,
    webhook_secret: Option<&str>,
) -> Result<(), GatewayError> {
    let connection = pool.connection();
    connection.execute(
        "INSERT INTO projects (id, name, api_key_hash, wallet_profile, scopes_json, webhook_url, webhook_secret) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            name,
            api_key_hash,
            wallet_profile,
            default_project_scopes_json(),
            webhook_url,
            webhook_secret
        ],
    )?;
    Ok(())
}

pub async fn update_project_scopes(
    pool: &SqlitePool,
    id: &str,
    scopes: &[String],
) -> Result<Option<row::Project>, GatewayError> {
    let scopes_json = serde_json::to_string(scopes)
        .map_err(|error| GatewayError::BadRequest(format!("invalid scopes: {error}")))?;
    let connection = pool.connection();
    let updated = connection.execute(
        "UPDATE projects SET scopes_json = ?, updated_at = datetime('now') WHERE id = ?",
        params![scopes_json, id],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    let project = connection
        .query_row("SELECT * FROM projects WHERE id = ?", params![id], |row| {
            row::Project::from_row(row)
        })
        .optional()?;
    Ok(project)
}

pub async fn find_project_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<row::Project>, GatewayError> {
    let connection = pool.connection();
    let project = connection
        .query_row("SELECT * FROM projects WHERE id = ?", params![id], |row| {
            row::Project::from_row(row)
        })
        .optional()?;
    Ok(project)
}

pub async fn list_projects(pool: &SqlitePool) -> Result<Vec<row::Project>, GatewayError> {
    let connection = pool.connection();
    let mut statement = connection.prepare("SELECT * FROM projects ORDER BY created_at")?;
    let rows = statement.query_map([], row::Project::from_row)?;
    let projects = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(projects)
}

// ── Payment queries ────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn insert_payment(
    pool: &SqlitePool,
    id: &str,
    project_id: &str,
    idempotency_key: Option<&str>,
    amount_wei: &str,
    chain_id: i64,
    token_address: Option<&str>,
    stealth_address: &str,
    ephemeral_pub: &str,
    view_tag: Option<&str>,
    deposit_id: Option<&str>,
    metadata_json: &str,
    expires_at: Option<&str>,
) -> Result<(), GatewayError> {
    let connection = pool.connection();
    connection.execute(
        "INSERT INTO payments (id, project_id, idempotency_key, amount_wei, chain_id, token_address, stealth_address, ephemeral_pub, view_tag, deposit_id, metadata_json, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            project_id,
            idempotency_key,
            amount_wei,
            chain_id,
            token_address,
            stealth_address,
            ephemeral_pub,
            view_tag,
            deposit_id,
            metadata_json,
            expires_at,
        ],
    )?;
    Ok(())
}

pub async fn find_payment_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<row::Payment>, GatewayError> {
    let connection = pool.connection();
    let payment = connection
        .query_row("SELECT * FROM payments WHERE id = ?", params![id], |row| {
            row::Payment::from_row(row)
        })
        .optional()?;
    Ok(payment)
}

/// A4: Look up a payment by its idempotency key within a project scope.
pub async fn find_payment_by_idempotency_key(
    pool: &SqlitePool,
    project_id: &str,
    idempotency_key: &str,
) -> Result<Option<row::Payment>, GatewayError> {
    let connection = pool.connection();
    let payment = connection
        .query_row(
            "SELECT * FROM payments WHERE project_id = ? AND idempotency_key = ?",
            params![project_id, idempotency_key],
            row::Payment::from_row,
        )
        .optional()?;
    Ok(payment)
}

pub async fn list_payments_by_project(
    pool: &SqlitePool,
    project_id: &str,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<row::Payment>, GatewayError> {
    let connection = pool.connection();
    let payments = if let Some(status) = status {
        let mut statement = connection.prepare(
            "SELECT * FROM payments WHERE project_id = ? AND status = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )?;
        let rows = statement.query_map(params![project_id, status, limit, offset], |row| {
            row::Payment::from_row(row)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let mut statement = connection.prepare(
            "SELECT * FROM payments WHERE project_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )?;
        let rows = statement.query_map(params![project_id, limit, offset], |row| {
            row::Payment::from_row(row)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(payments)
}

pub async fn list_pending_payments(pool: &SqlitePool) -> Result<Vec<row::Payment>, GatewayError> {
    let connection = pool.connection();
    let mut statement = connection.prepare(
        "SELECT * FROM payments WHERE status IN ('pending', 'confirmed', 'sweeping') ORDER BY created_at",
    )?;
    let rows = statement.query_map([], row::Payment::from_row)?;
    let payments = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(payments)
}

pub async fn update_payment_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
) -> Result<(), GatewayError> {
    let timestamp_col = match status {
        "confirmed" => Some("confirmed_at"),
        "swept" => Some("swept_at"),
        _ => None,
    };

    let connection = pool.connection();
    if let Some(col) = timestamp_col {
        let query = format!("UPDATE payments SET status = ?, {col} = datetime('now') WHERE id = ?");
        connection.execute(&query, params![status, id])?;
    } else {
        connection.execute(
            "UPDATE payments SET status = ? WHERE id = ?",
            params![status, id],
        )?;
    }
    Ok(())
}

pub async fn expire_old_payments(pool: &SqlitePool) -> Result<u64, GatewayError> {
    let connection = pool.connection();
    let affected = connection.execute(
        "UPDATE payments SET status = 'expired' WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    )?;
    Ok(affected as u64)
}

// ── Webhook delivery queries ───────────────────────────────────────

pub async fn insert_webhook_delivery(
    pool: &SqlitePool,
    delivery: NewWebhookDelivery<'_>,
) -> Result<(), GatewayError> {
    let connection = pool.connection();
    connection.execute(
        "INSERT INTO webhook_deliveries (payment_id, event, url, attempt, status_code, response_body, next_retry_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            delivery.payment_id,
            delivery.event,
            delivery.url,
            delivery.attempt,
            delivery.status_code,
            delivery.response_body,
            delivery.next_retry_at,
        ],
    )?;
    Ok(())
}

pub async fn list_pending_webhook_retries(
    pool: &SqlitePool,
) -> Result<Vec<row::WebhookDelivery>, GatewayError> {
    let connection = pool.connection();
    let mut statement = connection.prepare(
        "SELECT * FROM webhook_deliveries WHERE next_retry_at IS NOT NULL AND next_retry_at <= datetime('now') ORDER BY next_retry_at",
    )?;
    let rows = statement.query_map([], row::WebhookDelivery::from_row)?;
    let deliveries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(deliveries)
}

/// Clear the retry marker on a webhook delivery (either succeeded or exhausted retries).
pub async fn clear_webhook_retry(pool: &SqlitePool, id: i64) -> Result<(), GatewayError> {
    let connection = pool.connection();
    connection.execute(
        "UPDATE webhook_deliveries SET next_retry_at = NULL WHERE id = ?",
        params![id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    async fn test_pool() -> Result<(TempDir, SqlitePool), Box<dyn std::error::Error + Send + Sync>>
    {
        let dir = TempDir::new()?;
        let db_url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("gateway.db").display()
        );
        let pool = connect(&db_url).await?;
        Ok((dir, pool))
    }

    #[tokio::test]
    async fn webhook_delivery_persists_attempt_count()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (_dir, pool) = test_pool().await?;

        insert_project(
            &pool,
            "project-1",
            "merchant-a",
            "hash",
            "payments-mainnet",
            None,
            None,
        )
        .await?;
        insert_payment(
            &pool,
            "payment-1",
            "project-1",
            Some("idem-1"),
            "0x1",
            1,
            None,
            "st:address:stub",
            &"33".repeat(32),
            Some("aa"),
            Some("deposit-1"),
            "{}",
            Some("2099-01-01 00:00:00"),
        )
        .await?;

        insert_webhook_delivery(
            &pool,
            NewWebhookDelivery {
                payment_id: "payment-1",
                event: "payment.confirmed",
                url: "https://example.com/hook",
                attempt: 2,
                status_code: Some(500),
                response_body: Some("upstream error"),
                next_retry_at: Some("2000-01-01 00:00:00"),
            },
        )
        .await?;

        let pending = list_pending_webhook_retries(&pool).await?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt, 2);

        Ok(())
    }
}
