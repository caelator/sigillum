use crate::service::ServiceError;

use super::JsonRpcError;

pub(super) fn provider_http_error(method: &str, status: reqwest::StatusCode) -> ServiceError {
    let message = format!("Provider request failed for {method}: http {status}");
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        ServiceError::too_many_requests(message)
    } else if status.is_client_error() {
        ServiceError::bad_request(message)
    } else {
        ServiceError::internal(message)
    }
}

pub(super) fn provider_json_rpc_error(method: &str, error: JsonRpcError) -> ServiceError {
    let message = format!(
        "Provider error for {method}: {} ({})",
        error.message, error.code
    );
    if provider_error_is_rate_limited(error.code, &error.message) {
        ServiceError::too_many_requests(message)
    } else if matches!(error.code, -32700 | -32600 | -32601 | -32602) {
        ServiceError::bad_request(message)
    } else {
        ServiceError::internal(message)
    }
}

fn provider_error_is_rate_limited(code: i64, message: &str) -> bool {
    if code == -32005 {
        return true;
    }
    let message = message.to_ascii_lowercase();
    message.contains("rate limit")
        || message.contains("too many requests")
        || message.contains("throttle")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_rate_limit_detection_catches_common_signals() {
        assert!(provider_error_is_rate_limited(-32005, "request limit"));
        assert!(provider_error_is_rate_limited(0, "Too many requests"));
        assert!(provider_error_is_rate_limited(0, "provider throttle"));
        assert!(!provider_error_is_rate_limited(-32602, "invalid params"));
    }
}
