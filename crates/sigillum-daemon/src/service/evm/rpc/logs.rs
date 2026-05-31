use serde_json::Value;

use crate::service::{ServiceError, ServiceResult};

use super::super::{normalize_address, normalize_hex_blob, normalize_hex_blob_allow_empty};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::service) struct EvmLogEntry {
    pub address: String,
    pub topics: Vec<String>,
    pub data: String,
    pub block_number: Option<String>,
    pub transaction_hash: Option<String>,
    pub log_index: Option<String>,
}

pub(super) fn parse_log_entry(value: &Value) -> ServiceResult<EvmLogEntry> {
    let object = value
        .as_object()
        .ok_or_else(|| ServiceError::internal("Invalid provider log response"))?;
    let address = object
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::internal("Provider log missing address"))
        .and_then(normalize_address)?;
    let topics = object
        .get("topics")
        .and_then(Value::as_array)
        .ok_or_else(|| ServiceError::internal("Provider log missing topics"))?
        .iter()
        .map(|topic| {
            topic
                .as_str()
                .ok_or_else(|| ServiceError::internal("Provider log topic must be a string"))
                .and_then(|topic| normalize_fixed_hex(topic, 32, "log topic"))
        })
        .collect::<ServiceResult<Vec<_>>>()?;
    let data = object
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| ServiceError::internal("Provider log missing data"))
        .and_then(|data| normalize_hex_blob_allow_empty(data, "log data"))?;
    let block_number = optional_normalized_hex(object.get("blockNumber"), "block number")?;
    let transaction_hash =
        optional_normalized_hex(object.get("transactionHash"), "transaction hash")?;
    let log_index = optional_normalized_hex(object.get("logIndex"), "log index")?;
    Ok(EvmLogEntry {
        address,
        topics,
        data,
        block_number,
        transaction_hash,
        log_index,
    })
}

fn optional_normalized_hex(value: Option<&Value>, label: &str) -> ServiceResult<Option<String>> {
    value
        .and_then(Value::as_str)
        .map(|value| normalize_hex_blob_allow_empty(value, label))
        .transpose()
}

fn normalize_fixed_hex(value: &str, bytes: usize, label: &str) -> ServiceResult<String> {
    let normalized = normalize_hex_blob(value, label)?;
    if normalized.len() != 2 + (bytes * 2) {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} has wrong length"
        )));
    }
    Ok(normalized)
}
