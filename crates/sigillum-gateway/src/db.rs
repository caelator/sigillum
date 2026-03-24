//! SQLite database layer — connection pool, migrations, and typed queries.

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

use crate::error::GatewayError;

/// Row types returned from queries.
pub mod row {
    use serde::Serialize;

    #[derive(Debug, Clone, Serialize, sqlx::FromRow)]
    pub struct Project {
        pub id: String,
        pub name: String,
        pub api_key_hash: String,
        pub wallet_profile: String,
        pub webhook_url: Option<String>,
        pub webhook_secret: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    #[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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

    #[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    // R3: Enable WAL mode for concurrent read/write
    sqlx::raw_sql("PRAGMA journal_mode=WAL;")
        .execute(&pool)
        .await?;

    // Run schema — idempotent (CREATE TABLE IF NOT EXISTS)
    let schema = include_str!("../schema.sql");
    sqlx::raw_sql(schema).execute(&pool).await?;

    Ok(pool)
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
    sqlx::query(
        "INSERT INTO projects (id, name, api_key_hash, wallet_profile, webhook_url, webhook_secret) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(api_key_hash)
    .bind(wallet_profile)
    .bind(webhook_url)
    .bind(webhook_secret)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_project_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<row::Project>, GatewayError> {
    let project = sqlx::query_as::<_, row::Project>("SELECT * FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(project)
}

pub async fn list_projects(pool: &SqlitePool) -> Result<Vec<row::Project>, GatewayError> {
    let projects = sqlx::query_as::<_, row::Project>("SELECT * FROM projects ORDER BY created_at")
        .fetch_all(pool)
        .await?;
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
    sqlx::query(
        "INSERT INTO payments (id, project_id, idempotency_key, amount_wei, chain_id, token_address, stealth_address, ephemeral_pub, view_tag, deposit_id, metadata_json, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(project_id)
    .bind(idempotency_key)
    .bind(amount_wei)
    .bind(chain_id)
    .bind(token_address)
    .bind(stealth_address)
    .bind(ephemeral_pub)
    .bind(view_tag)
    .bind(deposit_id)
    .bind(metadata_json)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_payment_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<row::Payment>, GatewayError> {
    let payment = sqlx::query_as::<_, row::Payment>("SELECT * FROM payments WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(payment)
}

/// A4: Look up a payment by its idempotency key within a project scope.
pub async fn find_payment_by_idempotency_key(
    pool: &SqlitePool,
    project_id: &str,
    idempotency_key: &str,
) -> Result<Option<row::Payment>, GatewayError> {
    let payment = sqlx::query_as::<_, row::Payment>(
        "SELECT * FROM payments WHERE project_id = ? AND idempotency_key = ?",
    )
    .bind(project_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await?;
    Ok(payment)
}

pub async fn list_payments_by_project(
    pool: &SqlitePool,
    project_id: &str,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<row::Payment>, GatewayError> {
    let payments = if let Some(status) = status {
        sqlx::query_as::<_, row::Payment>(
            "SELECT * FROM payments WHERE project_id = ? AND status = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(project_id)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, row::Payment>(
            "SELECT * FROM payments WHERE project_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
        )
        .bind(project_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    };
    Ok(payments)
}

pub async fn list_pending_payments(pool: &SqlitePool) -> Result<Vec<row::Payment>, GatewayError> {
    let payments = sqlx::query_as::<_, row::Payment>(
        "SELECT * FROM payments WHERE status IN ('pending', 'confirmed', 'sweeping') ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
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

    if let Some(col) = timestamp_col {
        let query = format!("UPDATE payments SET status = ?, {col} = datetime('now') WHERE id = ?");
        sqlx::query(&query)
            .bind(status)
            .bind(id)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("UPDATE payments SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn expire_old_payments(pool: &SqlitePool) -> Result<u64, GatewayError> {
    let result = sqlx::query(
        "UPDATE payments SET status = 'expired' WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < datetime('now')",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

// ── Webhook delivery queries ───────────────────────────────────────

pub async fn insert_webhook_delivery(
    pool: &SqlitePool,
    delivery: NewWebhookDelivery<'_>,
) -> Result<(), GatewayError> {
    sqlx::query(
        "INSERT INTO webhook_deliveries (payment_id, event, url, attempt, status_code, response_body, next_retry_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(delivery.payment_id)
    .bind(delivery.event)
    .bind(delivery.url)
    .bind(delivery.attempt)
    .bind(delivery.status_code)
    .bind(delivery.response_body)
    .bind(delivery.next_retry_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_pending_webhook_retries(
    pool: &SqlitePool,
) -> Result<Vec<row::WebhookDelivery>, GatewayError> {
    let deliveries = sqlx::query_as::<_, row::WebhookDelivery>(
        "SELECT * FROM webhook_deliveries WHERE next_retry_at IS NOT NULL AND next_retry_at <= datetime('now') ORDER BY next_retry_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(deliveries)
}

/// Clear the retry marker on a webhook delivery (either succeeded or exhausted retries).
pub async fn clear_webhook_retry(pool: &SqlitePool, id: i64) -> Result<(), GatewayError> {
    sqlx::query("UPDATE webhook_deliveries SET next_retry_at = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
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
