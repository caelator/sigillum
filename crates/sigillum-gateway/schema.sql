-- Sigillum Gateway Schema
-- SQLite database for payment state management

CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY NOT NULL,       -- UUID
    name        TEXT NOT NULL UNIQUE,            -- human-readable project name
    api_key_hash TEXT NOT NULL,                  -- SHA-256 hash of the API key
    wallet_profile TEXT NOT NULL,                -- Sigillum stealth wallet profile name
    scopes_json TEXT NOT NULL DEFAULT '["payments:create","payments:read","payments:list","payments:cancel","webhooks:read"]',
    webhook_url TEXT,                            -- URL to POST webhook events
    webhook_secret TEXT,                         -- HMAC secret for signing webhooks
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS payments (
    id              TEXT PRIMARY KEY NOT NULL,   -- UUID
    project_id      TEXT NOT NULL REFERENCES projects(id),
    idempotency_key TEXT,                        -- client-supplied dedup key (unique per project)
    amount_wei      TEXT NOT NULL,               -- hex-encoded wei amount
    chain_id        INTEGER NOT NULL,            -- EVM chain ID
    token_address   TEXT,                        -- NULL for native ETH, address for ERC-20
    stealth_address TEXT NOT NULL,               -- generated stealth address
    ephemeral_pub   TEXT NOT NULL,               -- ephemeral public key hex
    view_tag        TEXT,                        -- view tag hex
    deposit_id      TEXT,                        -- Sigillum deposit tracking ID
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending|confirmed|sweeping|swept|expired|cancelled
    metadata_json   TEXT DEFAULT '{}',           -- arbitrary merchant metadata
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at      TEXT,                        -- payment expiry time
    confirmed_at    TEXT,
    swept_at        TEXT
);

CREATE INDEX IF NOT EXISTS idx_payments_project ON payments(project_id);
CREATE INDEX IF NOT EXISTS idx_payments_status ON payments(status);
CREATE INDEX IF NOT EXISTS idx_payments_deposit ON payments(deposit_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_payments_idempotency ON payments(project_id, idempotency_key);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    payment_id  TEXT NOT NULL REFERENCES payments(id),
    event       TEXT NOT NULL,                   -- payment.confirmed, payment.swept, etc.
    url         TEXT NOT NULL,
    status_code INTEGER,                         -- HTTP response status
    attempt     INTEGER NOT NULL DEFAULT 1,
    response_body TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    next_retry_at TEXT                           -- NULL if no more retries
);

CREATE INDEX IF NOT EXISTS idx_webhook_payment ON webhook_deliveries(payment_id);
CREATE INDEX IF NOT EXISTS idx_webhook_retry ON webhook_deliveries(next_retry_at);
