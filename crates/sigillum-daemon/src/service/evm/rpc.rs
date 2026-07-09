//! JSON-RPC transport and provider error classification.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::service::{ServiceError, ServiceResult};

use super::{
    normalize_address, normalize_hex_blob, normalize_hex_blob_allow_empty, parse_quantity_u64,
    parse_quantity_u256,
};

mod abi;
mod block;
mod erc1155;
mod erc20;
mod erc721;
mod errors;
mod logs;
mod permit2;
mod receipt;

use abi::{nft_token_uri_call_data, parse_abi_string_result};
use errors::{provider_http_error, provider_json_rpc_error};
pub(in crate::service) use logs::EvmLogEntry;
use logs::parse_log_entry;
pub(in crate::service) use receipt::EvmTransactionReceipt;

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
    // A JSON-RPC `null` result is semantically meaningful for some methods
    // (`eth_getTransactionReceipt` returns `null` for "not yet mined" — W7.4
    // receipt polling). Deserializing this as a required `Value` (defaulting
    // to `Value::Null` when the key is absent) collapses "explicit null" and
    // "missing key" into the SAME `Value::Null`, so `into_result` never
    // manufactures a spurious transport error for a valid null result;
    // per-method parsers (e.g. `parse_receipt`) decide whether `Null` is
    // acceptable for THEM.
    #[serde(default)]
    result: Value,
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

impl JsonRpcResponse {
    fn into_result(self, method: &str) -> ServiceResult<Value> {
        if let Some(error) = self.error {
            return Err(provider_json_rpc_error(method, error));
        }
        Ok(self.result)
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
