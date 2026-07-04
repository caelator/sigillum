//! Queue execution failure classification and retry disposition.

use axum::http::StatusCode;

use crate::service::ServiceError;

pub(super) enum QueueFailureDisposition {
    Retryable {
        reason: String,
        retry_after_unix: u64,
        cause: QueueFailureCause,
    },
    FailedTerminal {
        reason: String,
        cause: QueueFailureCause,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QueueFailureCause {
    ProviderError,
    PolicyBlock,
    InsufficientGas,
    Validation,
    Unknown,
}

pub(super) fn classify_queue_error(
    error: ServiceError,
    attempts: u32,
    now: u64,
    policy: crate::policy::RuntimePolicy,
) -> QueueFailureDisposition {
    let cause = classify_service_error_cause(&error);
    match error.status() {
        StatusCode::INTERNAL_SERVER_ERROR | StatusCode::TOO_MANY_REQUESTS => {
            QueueFailureDisposition::Retryable {
                reason: error.message().to_string(),
                retry_after_unix: now + queue_retry_delay_secs(attempts, policy),
                cause,
            }
        }
        _ => QueueFailureDisposition::FailedTerminal {
            reason: error.message().to_string(),
            cause,
        },
    }
}

pub(super) fn classify_blocked_queue_reason(reason: &str) -> QueueFailureCause {
    classify_failure_message(None, reason)
}

fn classify_service_error_cause(error: &ServiceError) -> QueueFailureCause {
    if error.action().is_some() {
        return QueueFailureCause::PolicyBlock;
    }
    classify_failure_message(Some(error.status()), error.message())
}

fn classify_failure_message(status: Option<StatusCode>, message: &str) -> QueueFailureCause {
    let lower = message.to_ascii_lowercase();
    if contains_any(
        &lower,
        &[
            "policy",
            "guardrail",
            "not enabled",
            "not executable",
            "cross-party",
            "cross_party",
            "requires approval",
        ],
    ) {
        return QueueFailureCause::PolicyBlock;
    }
    if contains_any(
        &lower,
        &[
            "insufficient gas",
            "insufficient funds",
            "insufficient balance",
            "gas",
            "fee",
            "fees",
            "funds",
            "balance",
        ],
    ) {
        return QueueFailureCause::InsufficientGas;
    }
    if contains_any(
        &lower,
        &[
            "invalid",
            "malformed",
            "missing",
            "required",
            "decode",
            "parse",
            "validation",
            "bad payload",
            "bad request",
        ],
    ) {
        return QueueFailureCause::Validation;
    }
    if contains_any(
        &lower,
        &[
            "provider",
            "rpc",
            "http",
            "network",
            "timeout",
            "rate limit",
            "upstream",
            "transport",
            "temporarily unavailable",
        ],
    ) {
        return QueueFailureCause::ProviderError;
    }
    match status {
        Some(StatusCode::FORBIDDEN) => QueueFailureCause::PolicyBlock,
        Some(StatusCode::BAD_REQUEST | StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY) => {
            QueueFailureCause::Validation
        }
        Some(
            StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT,
        ) => QueueFailureCause::ProviderError,
        _ => QueueFailureCause::Unknown,
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn queue_retry_delay_secs(attempts: u32, policy: crate::policy::RuntimePolicy) -> u64 {
    policy.queue_retry_delay_secs(attempts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_uses_bounded_backoff() {
        let policy = crate::policy::RuntimePolicy::default();
        assert_eq!(queue_retry_delay_secs(0, policy), 5);
        assert_eq!(queue_retry_delay_secs(1, policy), 5);
        assert_eq!(queue_retry_delay_secs(2, policy), 10);
        assert_eq!(queue_retry_delay_secs(10, policy), 300);
    }

    #[test]
    fn queue_error_classification_distinguishes_retryable_failures() {
        let now = 100;
        let policy = crate::policy::RuntimePolicy::default();
        match classify_queue_error(ServiceError::internal("rpc down"), 0, now, policy) {
            QueueFailureDisposition::Retryable {
                reason,
                retry_after_unix,
                cause,
            } => {
                assert!(reason.contains("rpc down"));
                assert_eq!(retry_after_unix, 105);
                assert_eq!(cause, QueueFailureCause::ProviderError);
            }
            QueueFailureDisposition::FailedTerminal { .. } => panic!("expected retryable"),
        }

        match classify_queue_error(ServiceError::bad_request("bad payload"), 0, now, policy) {
            QueueFailureDisposition::FailedTerminal { reason, cause } => {
                assert!(reason.contains("bad payload"));
                assert_eq!(cause, QueueFailureCause::Validation);
            }
            QueueFailureDisposition::Retryable { .. } => panic!("expected terminal"),
        }
    }

    fn disposition_cause(disposition: &QueueFailureDisposition) -> QueueFailureCause {
        match disposition {
            QueueFailureDisposition::Retryable { cause, .. }
            | QueueFailureDisposition::FailedTerminal { cause, .. } => *cause,
        }
    }

    #[test]
    fn maintenance_failure_cause_provider_error() {
        let disposition = classify_queue_error(
            ServiceError::internal("provider rpc timeout"),
            0,
            100,
            crate::policy::RuntimePolicy::default(),
        );
        assert_eq!(
            disposition_cause(&disposition),
            QueueFailureCause::ProviderError
        );
    }

    #[test]
    fn maintenance_failure_cause_policy_block() {
        assert_eq!(
            classify_blocked_queue_reason("seed-wallet queue execution is not enabled yet"),
            QueueFailureCause::PolicyBlock
        );
        let disposition = classify_queue_error(
            ServiceError::policy_violation("cross-party transfer requires approval"),
            0,
            100,
            crate::policy::RuntimePolicy::default(),
        );
        assert_eq!(
            disposition_cause(&disposition),
            QueueFailureCause::PolicyBlock
        );
    }

    #[test]
    fn maintenance_failure_cause_insufficient_gas() {
        let disposition = classify_queue_error(
            ServiceError::bad_request("insufficient gas balance for sweep"),
            0,
            100,
            crate::policy::RuntimePolicy::default(),
        );
        assert_eq!(
            disposition_cause(&disposition),
            QueueFailureCause::InsufficientGas
        );
    }

    #[test]
    fn maintenance_failure_cause_validation() {
        let disposition = classify_queue_error(
            ServiceError::bad_request("invalid stealth address"),
            0,
            100,
            crate::policy::RuntimePolicy::default(),
        );
        assert_eq!(
            disposition_cause(&disposition),
            QueueFailureCause::Validation
        );
    }
}
