use serde_json::json;

use crate::service::{ServiceError, ServiceResult};

use super::ProviderRpcClient;
use crate::service::evm::{normalize_address, normalize_hex_blob};

const ERC721_OWNER_OF_SELECTOR: &str = "6352211e";

impl ProviderRpcClient {
    pub(in crate::service::evm) async fn get_erc721_owner(
        &self,
        contract_address: &str,
        token_id_hex: &str,
        block_tag: &str,
    ) -> ServiceResult<String> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(contract_address)?,
                    "data": erc721_owner_of_call_data(token_id_hex)?,
                }, block_tag]),
            )
            .await?;
        parse_abi_address_result(&value)
    }
}

fn erc721_owner_of_call_data(token_id_hex: &str) -> ServiceResult<String> {
    let token_id = normalize_word_hex(token_id_hex, "ERC-721 token id")?;
    Ok(format!("0x{ERC721_OWNER_OF_SELECTOR}{token_id}"))
}

fn parse_abi_address_result(value: &serde_json::Value) -> ServiceResult<String> {
    let result = value
        .as_str()
        .ok_or_else(|| ServiceError::internal("Invalid provider response: expected owner hex"))?;
    let normalized = normalize_word_hex(result, "ERC-721 owner result")?;
    normalize_address(&format!("0x{}", &normalized[24..]))
}

fn normalize_word_hex(value: &str, label: &str) -> ServiceResult<String> {
    let normalized = normalize_hex_blob(value, label)?;
    let raw = &normalized[2..];
    if raw.len() != 64 {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} must be 32 bytes"
        )));
    }
    Ok(raw.to_string())
}
