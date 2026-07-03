//! Gateway configuration loaded from environment variables.

use std::net::SocketAddr;

use crate::validate;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid GATEWAY_BIND_ADDR `{raw}`: {source}")]
    InvalidBindAddr {
        raw: String,
        source: std::net::AddrParseError,
    },
    #[error("invalid GATEWAY_BESATAS_WEBHOOK_URL `{url}`: {reason}")]
    InvalidBesatasWebhookUrl { url: String, reason: String },
    #[error(
        "GATEWAY_BESATAS_WEBHOOK_PRIVATE_KEY is required when GATEWAY_BESATAS_WEBHOOK_URL is set"
    )]
    BesatasWebhookUrlWithoutPrivateKey,
    #[error(
        "GATEWAY_BESATAS_WEBHOOK_URL is required when GATEWAY_BESATAS_WEBHOOK_PRIVATE_KEY is set"
    )]
    BesatasWebhookPrivateKeyWithoutUrl,
}

/// Gateway configuration.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Sigillum daemon base URL (default: `http://127.0.0.1:9743`).
    pub daemon_url: String,
    /// Pre-established Sigillum daemon bearer session token for authenticated calls.
    pub daemon_session_token: Option<String>,
    /// Preferred scoped daemon capability token for gateway calls.
    pub daemon_capability_token: Option<String>,
    /// SQLite database path (default: `gateway.db`).
    pub database_url: String,
    /// Gateway bind address (default: `127.0.0.1:8443`).
    pub bind_addr: SocketAddr,
    /// Deposit polling interval in seconds (default: 30).
    pub poll_interval_secs: u64,
    /// Payment expiry in minutes (default: 60).
    pub payment_expiry_minutes: i64,
    /// Admin API key for project management (required in production).
    pub admin_key_hash: Option<String>,
    /// Allowed CORS origins (comma-separated, default: none).
    pub cors_origins: Vec<String>,
    /// Rate limit: max requests per second per IP (default: 20, 0 = disabled).
    pub rate_limit_rps: u64,
    /// Auth cache TTL in seconds (default: 30).
    pub auth_cache_ttl_secs: u64,
    /// Structured JSON logging (default: false, set GATEWAY_LOG_JSON=1).
    pub log_json: bool,
    /// Optional besatas payment-confirmation webhook URL.
    pub besatas_webhook_url: Option<String>,
    /// Ed25519 PKCS#8 private key PEM used to sign besatas callbacks.
    pub besatas_webhook_private_key: Option<String>,
}

impl GatewayConfig {
    /// Load configuration from environment variables with sensible defaults.
    pub fn from_env() -> Result<Self, ConfigError> {
        let admin_key = std::env::var("GATEWAY_ADMIN_KEY").ok();
        let admin_key_hash = admin_key.as_deref().map(|k| {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(k.as_bytes()))
        });

        if admin_key_hash.is_none() {
            tracing::warn!(
                "GATEWAY_ADMIN_KEY not set — project creation is disabled until an admin key is configured"
            );
        }

        let cors_origins: Vec<String> = std::env::var("GATEWAY_CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let besatas_webhook_url = std::env::var("GATEWAY_BESATAS_WEBHOOK_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let besatas_webhook_private_key = std::env::var("GATEWAY_BESATAS_WEBHOOK_PRIVATE_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        validate_besatas_webhook_config(
            besatas_webhook_url.as_deref(),
            besatas_webhook_private_key.as_deref(),
        )?;

        let bind_addr = parse_bind_addr(
            &std::env::var("GATEWAY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8443".into()),
        )?;

        Ok(Self {
            daemon_url: std::env::var("SIGILLUM_DAEMON_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:9743".into()),
            daemon_session_token: std::env::var("SIGILLUM_DAEMON_SESSION_TOKEN")
                .ok()
                .or_else(|| std::env::var("SIGILLUM_SESSION_TOKEN").ok())
                .filter(|value| !value.trim().is_empty()),
            daemon_capability_token: std::env::var("SIGILLUM_DAEMON_CAPABILITY_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            database_url: std::env::var("GATEWAY_DATABASE_URL")
                .unwrap_or_else(|_| "sqlite:gateway.db".into()),
            bind_addr,
            poll_interval_secs: std::env::var("GATEWAY_POLL_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            payment_expiry_minutes: std::env::var("GATEWAY_PAYMENT_EXPIRY_MINUTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            admin_key_hash,
            cors_origins,
            rate_limit_rps: std::env::var("GATEWAY_RATE_LIMIT_RPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            auth_cache_ttl_secs: std::env::var("GATEWAY_AUTH_CACHE_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            log_json: std::env::var("GATEWAY_LOG_JSON")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false),
            besatas_webhook_url,
            besatas_webhook_private_key,
        })
    }
}

fn parse_bind_addr(raw: &str) -> Result<SocketAddr, ConfigError> {
    raw.parse().map_err(|source| ConfigError::InvalidBindAddr {
        raw: raw.to_string(),
        source,
    })
}

fn validate_besatas_webhook_config(
    url: Option<&str>,
    key: Option<&str>,
) -> Result<(), ConfigError> {
    match (url, key) {
        (Some(url), Some(_)) => validate::validate_webhook_url(url).map_err(|reason| {
            ConfigError::InvalidBesatasWebhookUrl {
                url: url.to_string(),
                reason,
            }
        }),
        (Some(_), None) => Err(ConfigError::BesatasWebhookUrlWithoutPrivateKey),
        (None, Some(_)) => Err(ConfigError::BesatasWebhookPrivateKeyWithoutUrl),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, parse_bind_addr, validate_besatas_webhook_config};

    #[test]
    fn parse_bind_addr_rejects_invalid_address() {
        assert!(matches!(
            parse_bind_addr("not-an-addr"),
            Err(ConfigError::InvalidBindAddr { .. })
        ));
    }

    #[test]
    fn parse_bind_addr_accepts_valid_socket_address() {
        let addr = parse_bind_addr("127.0.0.1:8443").expect("valid bind address");

        assert_eq!(addr.to_string(), "127.0.0.1:8443");
    }

    #[test]
    fn validate_besatas_webhook_config_rejects_url_without_key() {
        assert!(matches!(
            validate_besatas_webhook_config(Some("https://example.com/hook"), None),
            Err(ConfigError::BesatasWebhookUrlWithoutPrivateKey)
        ));
    }

    #[test]
    fn validate_besatas_webhook_config_rejects_key_without_url() {
        assert!(matches!(
            validate_besatas_webhook_config(None, Some("private-key")),
            Err(ConfigError::BesatasWebhookPrivateKeyWithoutUrl)
        ));
    }

    #[test]
    fn validate_besatas_webhook_config_rejects_invalid_url_with_key() {
        assert!(matches!(
            validate_besatas_webhook_config(Some("not a url"), Some("private-key")),
            Err(ConfigError::InvalidBesatasWebhookUrl { .. })
        ));
    }

    #[test]
    fn validate_besatas_webhook_config_allows_missing_webhook_config() {
        assert!(validate_besatas_webhook_config(None, None).is_ok());
    }
}
