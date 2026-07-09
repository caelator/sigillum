//! JSON-RPC transport and provider error classification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::service::{ServiceError, ServiceResult};

use super::{
    normalize_address, normalize_hex_blob, normalize_hex_blob_allow_empty, parse_quantity_u64,
    parse_quantity_u256,
};

const NFT_TOKEN_URI_MAX_BYTES: usize = 2048;
const ERC721_TOKEN_URI_SELECTOR: &str = "c87b56dd";
const ERC1155_URI_SELECTOR: &str = "0e89341c";

mod block;
mod erc1155;
mod erc20;
mod erc721;
mod logs;
mod permit2;

pub(in crate::service) use logs::EvmLogEntry;
use logs::parse_log_entry;

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
pub(in crate::service) struct ProviderRpcClient {
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

    /// Chain id reported by the provider (`eth_chainId`), for verifying that
    /// an endpoint actually serves the network its profile claims.
    pub(super) async fn get_chain_id(&self) -> ServiceResult<u64> {
        let value = self.request(1, "eth_chainId", json!([])).await?;
        parse_quantity_u64(&value)
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

    pub(super) async fn latest_base_fee_per_gas(&self) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(1, "eth_feeHistory", json!(["0x1", "latest", []]))
            .await?;
        let base_fee = value
            .get("baseFeePerGas")
            .and_then(Value::as_array)
            .and_then(|fees| fees.last())
            .ok_or_else(|| {
                ServiceError::internal("Invalid provider response: missing baseFeePerGas")
            })?;
        parse_quantity_u256(base_fee)
    }

    pub(super) async fn max_priority_fee_per_gas(&self) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(1, "eth_maxPriorityFeePerGas", json!([]))
            .await?;
        parse_quantity_u256(&value)
    }

    pub(super) async fn get_logs(
        &self,
        address: &str,
        topics: &[String],
        from_block: &str,
        to_block: &str,
    ) -> ServiceResult<Vec<EvmLogEntry>> {
        let topics = topics.iter().cloned().map(Some).collect::<Vec<_>>();
        self.get_filtered_logs(Some(address), &topics, from_block, to_block)
            .await
    }

    pub(super) async fn get_filtered_logs(
        &self,
        address: Option<&str>,
        topics: &[Option<String>],
        from_block: &str,
        to_block: &str,
    ) -> ServiceResult<Vec<EvmLogEntry>> {
        let mut filter = Map::new();
        if let Some(address) = address {
            filter.insert("address".into(), json!(normalize_address(address)?));
        }
        filter.insert("fromBlock".into(), json!(from_block));
        filter.insert("toBlock".into(), json!(to_block));
        filter.insert(
            "topics".into(),
            Value::Array(
                topics
                    .iter()
                    .map(|topic| topic.clone().map(Value::String).unwrap_or(Value::Null))
                    .collect(),
            ),
        );
        let value = self
            .request(1, "eth_getLogs", Value::Array(vec![Value::Object(filter)]))
            .await?;
        let logs = value.as_array().ok_or_else(|| {
            ServiceError::internal("Invalid provider response: expected log array")
        })?;
        logs.iter().map(parse_log_entry).collect()
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

    pub(super) async fn simulate_contract_call(
        &self,
        from_address: &str,
        target_address: &str,
        data_hex: &str,
        value_hex: Option<&str>,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let mut transaction = Map::new();
        transaction.insert("from".into(), json!(normalize_address(from_address)?));
        transaction.insert("to".into(), json!(normalize_address(target_address)?));
        transaction.insert(
            "data".into(),
            json!(normalize_hex_blob_allow_empty(
                data_hex,
                "contract call data"
            )?),
        );
        if let Some(value_hex) = value_hex {
            transaction.insert(
                "value".into(),
                json!(normalize_hex_blob(value_hex, "call value")?),
            );
        }
        let value = self
            .request(
                1,
                "eth_call",
                Value::Array(vec![Value::Object(transaction), json!(block_tag)]),
            )
            .await?;
        let result = value.as_str().ok_or_else(|| {
            ServiceError::internal("Invalid provider response: expected eth_call result hex")
        })?;
        normalize_hex_blob_allow_empty(result, "eth_call result")
    }

    pub(super) async fn get_nft_token_uri(
        &self,
        contract_address: &str,
        token_id_hex: &str,
        erc1155: bool,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(contract_address)?,
                    "data": nft_token_uri_call_data(token_id_hex, erc1155)?,
                }, block_tag]),
            )
            .await?;
        parse_abi_string_result(&value)
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

fn nft_token_uri_call_data(token_id_hex: &str, erc1155: bool) -> ServiceResult<String> {
    let token_id = normalize_word_hex_arg(token_id_hex, "NFT token id")?;
    let selector = if erc1155 {
        ERC1155_URI_SELECTOR
    } else {
        ERC721_TOKEN_URI_SELECTOR
    };
    Ok(format!("0x{selector}{token_id}"))
}

fn normalize_word_hex_arg(value: &str, label: &str) -> ServiceResult<String> {
    let normalized = normalize_hex_blob(value, label)?;
    let raw = &normalized[2..];
    if raw.len() > 64 {
        return Err(ServiceError::bad_request(format!(
            "{label} must fit in 32 bytes"
        )));
    }
    Ok(format!("{}{}", "0".repeat(64 - raw.len()), raw))
}

fn parse_abi_string_result(value: &Value) -> ServiceResult<String> {
    let result = value.as_str().ok_or_else(|| {
        ServiceError::internal("Invalid provider response: expected ABI string hex")
    })?;
    let raw = result
        .strip_prefix("0x")
        .or_else(|| result.strip_prefix("0X"))
        .unwrap_or(result);
    if raw.is_empty() || raw.len() % 2 != 0 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string result must be hex",
        ));
    }
    let bytes = hex::decode(raw).map_err(|error| {
        ServiceError::internal(format!(
            "Invalid provider response: ABI string hex decode failed: {error}"
        ))
    })?;
    if bytes.len() < 64 {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string result is too short",
        ));
    }
    let offset = abi_word_to_usize(&bytes[0..32])?;
    if offset != 32 {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string offset must be 0x20",
        ));
    }
    let length_word_end = offset.checked_add(32).ok_or_else(|| {
        ServiceError::internal("Invalid provider response: ABI string offset overflow")
    })?;
    if bytes.len() < length_word_end {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string length word is missing",
        ));
    }
    let decoded_len = abi_word_to_usize(&bytes[offset..length_word_end])?;
    if decoded_len > NFT_TOKEN_URI_MAX_BYTES {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string exceeds 2048 bytes",
        ));
    }
    let data_start = length_word_end;
    let data_end = data_start.checked_add(decoded_len).ok_or_else(|| {
        ServiceError::internal("Invalid provider response: ABI string length overflow")
    })?;
    if bytes.len() < data_end {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI string data is truncated",
        ));
    }
    String::from_utf8(bytes[data_start..data_end].to_vec()).map_err(|error| {
        ServiceError::internal(format!(
            "Invalid provider response: ABI string is not valid UTF-8: {error}"
        ))
    })
}

fn abi_word_to_usize(word: &[u8]) -> ServiceResult<usize> {
    if word.len() != 32 {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI word must be 32 bytes",
        ));
    }
    if word[..24].iter().any(|byte| *byte != 0) {
        return Err(ServiceError::internal(
            "Invalid provider response: ABI word exceeds usize",
        ));
    }
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&word[24..32]);
    Ok(u64::from_be_bytes(raw) as usize)
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
    fn parses_valid_abi_string_result() {
        let value = json!(abi_string_hex("ipfs://bafy/token.json"));

        let parsed = parse_abi_string_result(&value).unwrap();

        assert_eq!(parsed, "ipfs://bafy/token.json");
    }

    #[test]
    fn rejects_abi_string_with_bad_offset() {
        let value = json!(format!(
            "0x{}{}",
            abi_word_hex(64),
            abi_word_hex("ignored".len())
        ));

        let error = parse_abi_string_result(&value).unwrap_err();

        assert!(error.message().contains("offset must be 0x20"));
    }

    #[test]
    fn rejects_oversize_abi_string_result() {
        let value = json!(format!("0x{}{}", abi_word_hex(32), abi_word_hex(2049)));

        let error = parse_abi_string_result(&value).unwrap_err();

        assert!(error.message().contains("exceeds 2048 bytes"));
    }

    #[test]
    fn provider_rate_limit_detection_catches_common_signals() {
        assert!(provider_error_is_rate_limited(-32005, "request limit"));
        assert!(provider_error_is_rate_limited(0, "Too many requests"));
        assert!(provider_error_is_rate_limited(0, "provider throttle"));
        assert!(!provider_error_is_rate_limited(-32602, "invalid params"));
    }

    fn abi_string_hex(value: &str) -> String {
        let mut data = hex::encode(value.as_bytes());
        while data.len() % 64 != 0 {
            data.push('0');
        }
        format!(
            "0x{}{}{}",
            abi_word_hex(32),
            abi_word_hex(value.len()),
            data
        )
    }

    fn abi_word_hex(value: usize) -> String {
        format!("{value:064x}")
    }
}
