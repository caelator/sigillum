//! `eth_getTransactionReceipt` support (W7.4 receipt confirmation).
//!
//! A missing receipt (still pending) and a MALFORMED/unexpected response
//! shape are treated identically — `Ok(None)` — never a hard error. This
//! matters for W7.4's "never assume a broadcast tx failed" rule: a transient
//! or non-standard provider response must not be mistaken for a definitive
//! outcome, so the caller simply keeps waiting rather than erroring out.

use serde_json::{Value, json};

use crate::service::ServiceResult;

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
        Ok(parse_receipt(&value))
    }
}

fn parse_receipt(value: &Value) -> Option<EvmTransactionReceipt> {
    let object = value.as_object()?;
    let status_hex = object.get("status").and_then(Value::as_str)?;
    let block_number_hex = object.get("blockNumber").and_then(Value::as_str)?;
    let gas_used_hex = object.get("gasUsed").and_then(Value::as_str)?;
    let status_success = parse_quantity_u64(&Value::String(status_hex.to_string())).ok()? != 0;
    let block_number = parse_quantity_u64(&Value::String(block_number_hex.to_string())).ok()?;
    let gas_used_hex = normalize_hex_blob(gas_used_hex, "gas used").ok()?;
    Some(EvmTransactionReceipt {
        status_success,
        block_number,
        gas_used_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_successful_receipt() {
        let value = json!({
            "status": "0x1",
            "blockNumber": "0x2a",
            "gasUsed": "0x5208",
            "transactionHash": "0xaa",
        });
        let receipt = parse_receipt(&value).expect("valid receipt");
        assert!(receipt.status_success);
        assert_eq!(receipt.block_number, 42);
        assert_eq!(receipt.gas_used_hex, "0x5208");
    }

    #[test]
    fn parses_a_reverted_receipt() {
        let value = json!({
            "status": "0x0",
            "blockNumber": "0x2a",
            "gasUsed": "0x5208",
        });
        let receipt = parse_receipt(&value).expect("valid receipt");
        assert!(!receipt.status_success);
    }

    #[test]
    fn null_result_is_no_receipt_yet() {
        assert_eq!(parse_receipt(&Value::Null), None);
    }

    #[test]
    fn unrecognized_object_shape_is_treated_as_no_receipt_rather_than_an_error() {
        // Exactly the shape a mock/older provider without receipt support
        // would return for an unhandled method — must never be mistaken for
        // a definitive success or failure.
        let value = json!({ "unsupported": "eth_getTransactionReceipt" });
        assert_eq!(parse_receipt(&value), None);
    }
}
