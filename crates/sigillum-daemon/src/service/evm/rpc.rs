//! JSON-RPC transport and provider error classification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::service::{ServiceError, ServiceResult};

use super::{
    erc20_balance_call_data, normalize_address, normalize_hex_blob, parse_quantity_u64,
    parse_quantity_u256,
};

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    id: u64,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Clone)]
pub(super) struct ProviderRpcClient {
    http: reqwest::Client,
    rpc_url: String,
    auth_token: Option<String>,
}

impl ProviderRpcClient {
    pub(super) fn new(http: &reqwest::Client, rpc_url: String, auth_token: Option<String>) -> Self {
        Self {
            http: http.clone(),
            rpc_url,
            auth_token,
        }
    }

    pub(super) async fn get_transaction_count(
        &self,
        address: &str,
        block_tag: &str,
    ) -> ServiceResult<u64> {
        let value = self
            .request(
                1,
                "eth_getTransactionCount",
                json!([normalize_address(address)?, block_tag]),
            )
            .await?;
        parse_quantity_u64(&value)
    }

    pub(super) async fn get_balance(
        &self,
        address: &str,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(
                1,
                "eth_getBalance",
                json!([normalize_address(address)?, block_tag]),
            )
            .await?;
        parse_quantity_u256(&value)
    }

    pub(super) async fn get_erc20_balance(
        &self,
        token_address: &str,
        owner_address: &str,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(token_address)?,
                    "data": erc20_balance_call_data(owner_address)?,
                }, block_tag]),
            )
            .await?;
        parse_quantity_u256(&value)
    }

    pub(super) async fn get_native_and_erc20_balance(
        &self,
        owner_address: &str,
        token_address: &str,
        block_tag: &str,
    ) -> ServiceResult<([u8; 32], [u8; 32])> {
        let responses = self
            .request_batch(&[
                JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 1,
                    method: "eth_getBalance",
                    params: json!([normalize_address(owner_address)?, block_tag]),
                },
                JsonRpcRequest {
                    jsonrpc: "2.0",
                    id: 2,
                    method: "eth_call",
                    params: json!([{
                        "to": normalize_address(token_address)?,
                        "data": erc20_balance_call_data(owner_address)?,
                    }, block_tag]),
                },
            ])
            .await?;

        let mut by_id: HashMap<u64, JsonRpcResponse> = HashMap::with_capacity(responses.len());
        for response in responses {
            by_id.insert(response.id, response);
        }

        let native_balance = parse_quantity_u256(&batch_result(&mut by_id, 1, "eth_getBalance")?)?;
        let token_balance = parse_quantity_u256(&batch_result(&mut by_id, 2, "eth_call")?)?;
        Ok((native_balance, token_balance))
    }

    pub(super) async fn send_raw_transaction(
        &self,
        raw_transaction_hex: &str,
    ) -> ServiceResult<String> {
        let raw = normalize_hex_blob(raw_transaction_hex, "raw transaction")?;
        let value = self
            .request(1, "eth_sendRawTransaction", json!([raw]))
            .await?;
        let hash = value
            .as_str()
            .ok_or_else(|| ServiceError::internal("Invalid provider response: expected tx hash"))?;
        let normalized = normalize_hex_blob(hash, "transaction hash")?;
        if normalized.len() != 66 {
            return Err(ServiceError::internal(
                "Invalid provider response: transaction hash has wrong length",
            ));
        }
        Ok(normalized[2..].to_string())
    }

    async fn request(&self, id: u64, method: &'static str, params: Value) -> ServiceResult<Value> {
        let response = self
            .request_batch(&[JsonRpcRequest {
                jsonrpc: "2.0",
                id,
                method,
                params,
            }])
            .await?;
        response
            .into_iter()
            .next()
            .ok_or_else(|| {
                ServiceError::internal(format!("Provider response missing result for {method}"))
            })?
            .into_result(method)
    }

    async fn request_batch(
        &self,
        requests: &[JsonRpcRequest<'_>],
    ) -> ServiceResult<Vec<JsonRpcResponse>> {
        let mut builder = self.http.post(&self.rpc_url).json(requests);
        if let Some(auth_token) = self.auth_token.as_deref() {
            builder = builder.bearer_auth(auth_token);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| ServiceError::internal(format!("Provider request failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            let method = requests
                .first()
                .map(|request| request.method)
                .unwrap_or("batch");
            return Err(provider_http_error(method, status));
        }

        let payload: Vec<JsonRpcResponse> = response.json().await.map_err(|error| {
            ServiceError::internal(format!("Provider response decode failed: {error}"))
        })?;

        Ok(payload)
    }
}

impl JsonRpcResponse {
    fn into_result(self, method: &str) -> ServiceResult<Value> {
        if let Some(error) = self.error {
            return Err(provider_json_rpc_error(method, error));
        }

        self.result.ok_or_else(|| {
            ServiceError::internal(format!("Provider response missing result for {method}"))
        })
    }
}

fn batch_result(
    responses: &mut HashMap<u64, JsonRpcResponse>,
    id: u64,
    method: &str,
) -> ServiceResult<Value> {
    responses
        .remove(&id)
        .ok_or_else(|| {
            ServiceError::internal(format!("Provider batch response missing item for {method}"))
        })?
        .into_result(method)
}

fn provider_http_error(method: &str, status: reqwest::StatusCode) -> ServiceError {
    let message = format!("Provider request failed for {method}: http {status}");
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        ServiceError::too_many_requests(message)
    } else if status.is_client_error() {
        ServiceError::bad_request(message)
    } else {
        ServiceError::internal(message)
    }
}

fn provider_json_rpc_error(method: &str, error: JsonRpcError) -> ServiceError {
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
