//! `eth_getTransactionReceipt` support (W7.4 receipt confirmation).
//!
//! A missing receipt (still pending) and a MALFORMED/unexpected response
//! shape are treated identically — `Ok(None)` — never a hard error. This
//! matters for W7.4's "never assume a broadcast tx failed" rule: a transient
//! or non-standard provider response must not be mistaken for a definitive
//! outcome, so the caller simply keeps waiting rather than erroring out.

use serde_json::{Value, json};

use crate::service::{ServiceError, ServiceResult};

use super::super::{normalize_hex_blob, parse_quantity_u64};
use super::ProviderRpcClient;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::service) struct EvmTransactionReceipt {
    pub(in crate::service) status_success: bool,
    pub(in crate::service) block_number: u64,
    pub(in crate::service) gas_used_hex: String,
}

impl ProviderRpcClient {
    /// `None` covers BOTH "not yet mined" (JSON-RPC `null` result) and any
    /// response shape this parser cannot recognize as a receipt — callers
    /// must treat both the same way (keep waiting), never as a failure.
    pub(in crate::service) async fn get_transaction_receipt(
        &self,
        transaction_hash_hex: &str,
    ) -> ServiceResult<Option<EvmTransactionReceipt>> {
        let hash = normalize_hex_blob(transaction_hash_hex, "transaction hash")?;
        let value = self
            .request(1, "eth_getTransactionReceipt", json!([hash]))
            .await?;
        parse_receipt(&value, &hash)
    }
}

fn parse_receipt(
    value: &Value,
    expected_transaction_hash_hex: &str,
) -> ServiceResult<Option<EvmTransactionReceipt>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(status_hex) = object.get("status").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(block_number_hex) = object.get("blockNumber").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(gas_used_hex) = object.get("gasUsed").and_then(Value::as_str) else {
        return Ok(None);
    };
    let transaction_hash_hex = object
        .get("transactionHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ServiceError::internal(
                "Invalid provider receipt: missing transactionHash identity binding.",
            )
        })?;
    let transaction_hash_hex = normalize_hex_blob(transaction_hash_hex, "transaction hash")
        .map_err(|_| ServiceError::internal("Invalid provider receipt transactionHash."))?;
    if transaction_hash_hex.len() != 66
        || !transaction_hash_hex.eq_ignore_ascii_case(expected_transaction_hash_hex)
    {
        return Err(ServiceError::internal(format!(
            "Provider receipt transactionHash mismatch: expected {expected_transaction_hash_hex}, received {transaction_hash_hex}."
        )));
    }
    let status_success = parse_quantity_u64(&Value::String(status_hex.to_string()))? != 0;
    let block_number = parse_quantity_u64(&Value::String(block_number_hex.to_string()))?;
    let gas_used_hex = normalize_hex_blob(gas_used_hex, "gas used")?;
    Ok(Some(EvmTransactionReceipt {
        status_success,
        block_number,
        gas_used_hex,
    }))
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
