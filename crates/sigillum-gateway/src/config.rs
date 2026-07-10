//! Gateway configuration loaded from environment variables.

use std::net::SocketAddr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid GATEWAY_BIND_ADDR `{raw}`: {source}")]
    InvalidBindAddr {
        raw: String,
        source: std::net::AddrParseError,
    },
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
    /// Explicit opt-in for the non-finality-proving payment preview flow.
    pub experimental_payments_enabled: bool,
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
            experimental_payments_enabled: std::env::var("GATEWAY_ENABLE_EXPERIMENTAL_PAYMENTS")
                .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true"))
                .unwrap_or(false),
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
        })
    }
}

fn parse_bind_addr(raw: &str) -> Result<SocketAddr, ConfigError> {
    raw.parse().map_err(|source| ConfigError::InvalidBindAddr {
        raw: raw.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, parse_bind_addr};

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
}
