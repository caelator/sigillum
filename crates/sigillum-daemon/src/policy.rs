//! Runtime policy configuration with environment variable overrides and clamping.
//!
//! ## Runtime Policy Rationale
//!
//! The daemon enforces operational limits (queue pagination, audit log pagination, retry delays)
//! to prevent resource exhaustion. These limits are configurable at runtime via environment
//! variables (`SIGILLUM_*`) to support different deployment profiles:
//! - Development: Higher limits (less pagination, faster retries)
//! - Production: Conservative limits to prevent OOM or hanging requests
//!
//! Why configurable? Different deployments have different scale requirements. A Kubernetes
//! pod with 8GB RAM can handle larger batch operations than an embedded deployment.
//!
//! ## Clamping Strategy
//!
//! Invalid overrides (negative, zero, or conflicting) are silently clamped to safe ranges:
//! - `default_limit` is clamped between `1` and `max_limit`
//! - `max_limit` is clamped to at least `1`
//! - Requested values (at query time) are clamped between default and max
//!
//! This prevents configuration errors from breaking the daemon. An operator who sets
//! `SIGILLUM_QUEUE_DEFAULT_PROCESS_LIMIT=999` (higher than the current max of 500)
//! will silently get the effective max instead of a startup error. This is intentional:
//! runtime policy is best-effort configuration, not critical state.

use sigillum_api::response::RuntimePolicyResponse;

const DEFAULT_QUEUE_DEFAULT_PROCESS_LIMIT: usize = 50;
const DEFAULT_QUEUE_MAX_PROCESS_LIMIT: usize = 500;
const DEFAULT_DEPOSIT_DEFAULT_REFRESH_LIMIT: usize = 100;
const DEFAULT_DEPOSIT_MAX_REFRESH_LIMIT: usize = 500;
const DEFAULT_AUDIT_DEFAULT_LIMIT: usize = 25;
const DEFAULT_AUDIT_MAX_LIMIT: usize = 200;
const DEFAULT_QUEUE_RETRY_BASE_DELAY_SECS: u64 = 5;
const DEFAULT_QUEUE_RETRY_MAX_DELAY_SECS: u64 = 300;
const DEFAULT_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY: usize = 8;
const MAX_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY: usize = 64;
const MAX_QUEUE_RETRY_EXPONENT: u32 = 16;
const DEFAULT_IDLE_LOCK_SECS: u64 = 900;
const DEFAULT_IDLE_LOCK_DRAIN_SECS: u64 = 60;
const MAX_IDLE_LOCK_DRAIN_SECS: u64 = 300;

/// Runtime policy configuration with validated limits.
///
/// All fields are guaranteed to satisfy their invariants (see field docs).
/// Use `from_env()` to load from environment variables with automatic clamping,
/// or construct directly if you're confident about the values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimePolicy {
    /// Default number of queue jobs to process per request. Clamped to [1, max].
    pub queue_default_process_limit: usize,
    /// Maximum number of queue jobs a client can request. Clamped to >= 1.
    pub queue_max_process_limit: usize,
    /// Default number of deposits to refresh per request. Clamped to [1, max].
    pub deposit_default_refresh_limit: usize,
    /// Maximum number of deposits a client can request. Clamped to >= 1.
    pub deposit_max_refresh_limit: usize,
    /// Default number of audit events to return per request. Clamped to [1, max].
    pub audit_default_limit: usize,
    /// Maximum number of audit events a client can request. Clamped to >= 1.
    pub audit_max_limit: usize,
    /// Base delay for exponential backoff on queue job retries. Clamped to >= 1.
    pub queue_retry_base_delay_secs: u64,
    /// Maximum delay for exponential backoff. Clamped to >= base_delay.
    pub queue_retry_max_delay_secs: u64,
    /// Maximum number of concurrent balance observation requests. Clamped to [1, 64].
    pub provider_balance_observation_concurrency: usize,
    /// Idle session duration before unlocked custody state drains and locks.
    pub idle_lock_secs: u64,
    /// Observability deadline while waiting for in-flight guarded operations.
    pub idle_lock_drain_secs: u64,
    /// Optional force-lock deadline. Zero means never force-zeroize in-flight work.
    pub idle_lock_force_after_secs: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RuntimePolicyOverrides {
    queue_default_process_limit: Option<usize>,
    queue_max_process_limit: Option<usize>,
    deposit_default_refresh_limit: Option<usize>,
    deposit_max_refresh_limit: Option<usize>,
    audit_default_limit: Option<usize>,
    audit_max_limit: Option<usize>,
    queue_retry_base_delay_secs: Option<u64>,
    queue_retry_max_delay_secs: Option<u64>,
    provider_balance_observation_concurrency: Option<usize>,
    idle_lock_secs: Option<u64>,
    idle_lock_drain_secs: Option<u64>,
    idle_lock_force_after_secs: Option<u64>,
}

impl Default for RuntimePolicy {
    fn default() -> Self {
        Self::from_overrides(RuntimePolicyOverrides::default())
    }
}

impl RuntimePolicy {
    pub fn from_env() -> Self {
        Self::from_pairs(std::env::vars())
    }

    fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut overrides = RuntimePolicyOverrides::default();
        for (key, value) in pairs {
            let key = key.as_ref();
            let value = value.as_ref().trim();
            match key {
                "SIGILLUM_QUEUE_DEFAULT_PROCESS_LIMIT" => {
                    overrides.queue_default_process_limit = value.parse().ok();
                }
                "SIGILLUM_QUEUE_MAX_PROCESS_LIMIT" => {
                    overrides.queue_max_process_limit = value.parse().ok();
                }
                "SIGILLUM_DEPOSIT_DEFAULT_REFRESH_LIMIT" => {
                    overrides.deposit_default_refresh_limit = value.parse().ok();
                }
                "SIGILLUM_DEPOSIT_MAX_REFRESH_LIMIT" => {
                    overrides.deposit_max_refresh_limit = value.parse().ok();
                }
                "SIGILLUM_AUDIT_DEFAULT_LIMIT" => {
                    overrides.audit_default_limit = value.parse().ok();
                }
                "SIGILLUM_AUDIT_MAX_LIMIT" => {
                    overrides.audit_max_limit = value.parse().ok();
                }
                "SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS" => {
                    overrides.queue_retry_base_delay_secs = value.parse().ok();
                }
                "SIGILLUM_QUEUE_RETRY_MAX_DELAY_SECS" => {
                    overrides.queue_retry_max_delay_secs = value.parse().ok();
                }
                "SIGILLUM_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY" => {
                    overrides.provider_balance_observation_concurrency = value.parse().ok();
                }
                "SIGILLUM_IDLE_LOCK_SECS" => {
                    overrides.idle_lock_secs = value.parse().ok();
                }
                "SIGILLUM_IDLE_LOCK_DRAIN_SECS" => {
                    overrides.idle_lock_drain_secs = value.parse().ok();
                }
                "SIGILLUM_IDLE_LOCK_FORCE_AFTER_SECS" => {
                    overrides.idle_lock_force_after_secs = value.parse().ok();
                }
                _ => {}
            }
        }
        Self::from_overrides(overrides)
    }

    fn from_overrides(overrides: RuntimePolicyOverrides) -> Self {
        let queue_max_process_limit = overrides
            .queue_max_process_limit
            .unwrap_or(DEFAULT_QUEUE_MAX_PROCESS_LIMIT)
            .max(1);
        let queue_default_process_limit = overrides
            .queue_default_process_limit
            .unwrap_or(DEFAULT_QUEUE_DEFAULT_PROCESS_LIMIT)
            .clamp(1, queue_max_process_limit);

        let deposit_max_refresh_limit = overrides
            .deposit_max_refresh_limit
            .unwrap_or(DEFAULT_DEPOSIT_MAX_REFRESH_LIMIT)
            .max(1);
        let deposit_default_refresh_limit = overrides
            .deposit_default_refresh_limit
            .unwrap_or(DEFAULT_DEPOSIT_DEFAULT_REFRESH_LIMIT)
            .clamp(1, deposit_max_refresh_limit);

        let audit_max_limit = overrides
            .audit_max_limit
            .unwrap_or(DEFAULT_AUDIT_MAX_LIMIT)
            .max(1);
        let audit_default_limit = overrides
            .audit_default_limit
            .unwrap_or(DEFAULT_AUDIT_DEFAULT_LIMIT)
            .clamp(1, audit_max_limit);

        let queue_retry_base_delay_secs = overrides
            .queue_retry_base_delay_secs
            .unwrap_or(DEFAULT_QUEUE_RETRY_BASE_DELAY_SECS)
            .max(1);
        let queue_retry_max_delay_secs = overrides
            .queue_retry_max_delay_secs
            .unwrap_or(DEFAULT_QUEUE_RETRY_MAX_DELAY_SECS)
            .max(queue_retry_base_delay_secs);

        let provider_balance_observation_concurrency = overrides
            .provider_balance_observation_concurrency
            .unwrap_or(DEFAULT_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY)
            .clamp(1, MAX_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY);
        let idle_lock_secs = overrides
            .idle_lock_secs
            .unwrap_or(DEFAULT_IDLE_LOCK_SECS)
            .max(1);
        let idle_lock_drain_secs = overrides
            .idle_lock_drain_secs
            .unwrap_or(DEFAULT_IDLE_LOCK_DRAIN_SECS)
            .clamp(1, MAX_IDLE_LOCK_DRAIN_SECS);
        let idle_lock_force_after_secs = overrides.idle_lock_force_after_secs.unwrap_or(0);

        Self {
            queue_default_process_limit,
            queue_max_process_limit,
            deposit_default_refresh_limit,
            deposit_max_refresh_limit,
            audit_default_limit,
            audit_max_limit,
            queue_retry_base_delay_secs,
            queue_retry_max_delay_secs,
            provider_balance_observation_concurrency,
            idle_lock_secs,
            idle_lock_drain_secs,
            idle_lock_force_after_secs,
        }
    }

    pub fn queue_process_limit(&self, requested: Option<usize>) -> usize {
        requested
            .unwrap_or(self.queue_default_process_limit)
            .clamp(1, self.queue_max_process_limit)
    }

    pub fn deposit_refresh_limit(&self, requested: Option<usize>) -> usize {
        requested
            .unwrap_or(self.deposit_default_refresh_limit)
            .clamp(1, self.deposit_max_refresh_limit)
    }

    pub fn audit_limit(&self, requested: Option<usize>) -> usize {
        requested
            .unwrap_or(self.audit_default_limit)
            .clamp(1, self.audit_max_limit)
    }

    pub fn queue_retry_delay_secs(&self, attempts: u32) -> u64 {
        let exponent = attempts.saturating_sub(1).min(MAX_QUEUE_RETRY_EXPONENT);
        self.queue_retry_base_delay_secs
            .saturating_mul(2u64.saturating_pow(exponent))
            .min(self.queue_retry_max_delay_secs)
    }

    pub fn as_response(&self) -> RuntimePolicyResponse {
        RuntimePolicyResponse {
            queue_default_process_limit: self.queue_default_process_limit,
            queue_max_process_limit: self.queue_max_process_limit,
            deposit_default_refresh_limit: self.deposit_default_refresh_limit,
            deposit_max_refresh_limit: self.deposit_max_refresh_limit,
            audit_default_limit: self.audit_default_limit,
            audit_max_limit: self.audit_max_limit,
            queue_retry_base_delay_secs: self.queue_retry_base_delay_secs,
            queue_retry_max_delay_secs: self.queue_retry_max_delay_secs,
            provider_balance_observation_concurrency: self.provider_balance_observation_concurrency,
            idle_lock_secs: self.idle_lock_secs,
            idle_lock_drain_secs: self.idle_lock_drain_secs,
            idle_lock_force_after_secs: self.idle_lock_force_after_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_policy_defaults_match_expected_baseline() {
        let policy = RuntimePolicy::default();

        assert_eq!(policy.queue_default_process_limit, 50);
        assert_eq!(policy.queue_max_process_limit, 500);
        assert_eq!(policy.deposit_default_refresh_limit, 100);
        assert_eq!(policy.deposit_max_refresh_limit, 500);
        assert_eq!(policy.audit_default_limit, 25);
        assert_eq!(policy.audit_max_limit, 200);
        assert_eq!(policy.queue_retry_base_delay_secs, 5);
        assert_eq!(policy.queue_retry_max_delay_secs, 300);
        assert_eq!(policy.provider_balance_observation_concurrency, 8);
        assert_eq!(policy.idle_lock_secs, 900);
        assert_eq!(policy.idle_lock_drain_secs, 60);
        assert_eq!(policy.idle_lock_force_after_secs, 0);
    }

    #[test]
    fn runtime_policy_sanitizes_invalid_overrides() {
        let policy = RuntimePolicy::from_pairs([
            ("SIGILLUM_QUEUE_DEFAULT_PROCESS_LIMIT", "999"),
            ("SIGILLUM_QUEUE_MAX_PROCESS_LIMIT", "20"),
            ("SIGILLUM_DEPOSIT_DEFAULT_REFRESH_LIMIT", "0"),
            ("SIGILLUM_DEPOSIT_MAX_REFRESH_LIMIT", "10"),
            ("SIGILLUM_AUDIT_DEFAULT_LIMIT", "999"),
            ("SIGILLUM_AUDIT_MAX_LIMIT", "5"),
            ("SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS", "30"),
            ("SIGILLUM_QUEUE_RETRY_MAX_DELAY_SECS", "10"),
            ("SIGILLUM_PROVIDER_BALANCE_OBSERVATION_CONCURRENCY", "999"),
            ("SIGILLUM_IDLE_LOCK_SECS", "0"),
            ("SIGILLUM_IDLE_LOCK_DRAIN_SECS", "999"),
            ("SIGILLUM_IDLE_LOCK_FORCE_AFTER_SECS", "45"),
        ]);

        assert_eq!(policy.queue_default_process_limit, 20);
        assert_eq!(policy.queue_max_process_limit, 20);
        assert_eq!(policy.deposit_default_refresh_limit, 1);
        assert_eq!(policy.deposit_max_refresh_limit, 10);
        assert_eq!(policy.audit_default_limit, 5);
        assert_eq!(policy.audit_max_limit, 5);
        assert_eq!(policy.queue_retry_base_delay_secs, 30);
        assert_eq!(policy.queue_retry_max_delay_secs, 30);
        assert_eq!(policy.idle_lock_secs, 1);
        assert_eq!(policy.idle_lock_drain_secs, 300);
        assert_eq!(policy.idle_lock_force_after_secs, 45);
        assert_eq!(policy.provider_balance_observation_concurrency, 64);
    }

    #[test]
    fn runtime_policy_clamps_requested_limits_and_retry_backoff() {
        let policy = RuntimePolicy::from_pairs([
            ("SIGILLUM_QUEUE_DEFAULT_PROCESS_LIMIT", "12"),
            ("SIGILLUM_QUEUE_MAX_PROCESS_LIMIT", "24"),
            ("SIGILLUM_DEPOSIT_DEFAULT_REFRESH_LIMIT", "15"),
            ("SIGILLUM_DEPOSIT_MAX_REFRESH_LIMIT", "30"),
            ("SIGILLUM_AUDIT_DEFAULT_LIMIT", "9"),
            ("SIGILLUM_AUDIT_MAX_LIMIT", "11"),
            ("SIGILLUM_QUEUE_RETRY_BASE_DELAY_SECS", "3"),
            ("SIGILLUM_QUEUE_RETRY_MAX_DELAY_SECS", "20"),
        ]);

        assert_eq!(policy.queue_process_limit(None), 12);
        assert_eq!(policy.queue_process_limit(Some(999)), 24);
        assert_eq!(policy.deposit_refresh_limit(None), 15);
        assert_eq!(policy.deposit_refresh_limit(Some(999)), 30);
        assert_eq!(policy.audit_limit(None), 9);
        assert_eq!(policy.audit_limit(Some(999)), 11);
        assert_eq!(policy.queue_retry_delay_secs(1), 3);
        assert_eq!(policy.queue_retry_delay_secs(2), 6);
        assert_eq!(policy.queue_retry_delay_secs(4), 20);
    }
}
