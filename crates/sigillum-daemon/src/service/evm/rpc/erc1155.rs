use serde_json::json;

use crate::service::ServiceResult;

use super::ProviderRpcClient;
use crate::service::evm::{normalize_address, normalize_hex_blob, parse_quantity_u256};

const ERC1155_BALANCE_OF_SELECTOR: &str = "00fdd58e";

impl ProviderRpcClient {
    pub(in crate::service::evm) async fn get_erc1155_balance(
        &self,
        contract_address: &str,
        owner_address: &str,
        token_id_hex: &str,
        block_tag: &str,
    ) -> ServiceResult<[u8; 32]> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(contract_address)?,
                    "data": erc1155_balance_of_call_data(owner_address, token_id_hex)?,
                }, block_tag]),
            )
            .await?;
        parse_quantity_u256(&value)
    }
}

fn erc1155_balance_of_call_data(owner_address: &str, token_id_hex: &str) -> ServiceResult<String> {
    let owner = normalize_address(owner_address)?;
    let token_id = normalize_word_hex(token_id_hex, "ERC-1155 token id")?;
    Ok(format!(
        "0x{ERC1155_BALANCE_OF_SELECTOR}{}{}{token_id}",
        "0".repeat(24),
        &owner[2..]
    ))
}

fn normalize_word_hex(value: &str, label: &str) -> ServiceResult<String> {
    let normalized = normalize_hex_blob(value, label)?;
    let raw = &normalized[2..];
    if raw.len() != 64 {
        return Err(crate::service::ServiceError::internal(format!(
            "Invalid provider response: {label} must be 32 bytes"
        )));
    }
    Ok(raw.to_string())
}
