use serde_json::json;

use crate::service::{ServiceError, ServiceResult};

use super::ProviderRpcClient;
use crate::service::evm::{normalize_address, normalize_hex_blob};

const ERC721_OWNER_OF_SELECTOR: &str = "6352211e";
const NFT_IS_APPROVED_FOR_ALL_SELECTOR: &str = "e985e9c5";

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

    pub(in crate::service::evm) async fn get_nft_operator_approval(
        &self,
        contract_address: &str,
        owner_address: &str,
        operator_address: &str,
        block_tag: &str,
    ) -> ServiceResult<bool> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(contract_address)?,
                    "data": nft_operator_approval_call_data(owner_address, operator_address)?,
                }, block_tag]),
            )
            .await?;
        parse_abi_bool_result(&value)
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

fn nft_operator_approval_call_data(
    owner_address: &str,
    operator_address: &str,
) -> ServiceResult<String> {
    let owner = normalize_address(owner_address)?;
    let operator = normalize_address(operator_address)?;
    Ok(format!(
        "0x{NFT_IS_APPROVED_FOR_ALL_SELECTOR}{}{}{}{}",
        "0".repeat(24),
        &owner[2..],
        "0".repeat(24),
        &operator[2..]
    ))
}

fn parse_abi_bool_result(value: &serde_json::Value) -> ServiceResult<bool> {
    let result = value.as_str().ok_or_else(|| {
        ServiceError::internal("Invalid provider response: expected ABI bool hex")
    })?;
    let normalized = normalize_word_hex(result, "NFT operator approval result")?;
    if normalized.bytes().all(|byte| byte == b'0') {
        return Ok(false);
    }
    if normalized == format!("{}1", "0".repeat(63)) {
        return Ok(true);
    }
    Err(ServiceError::internal(
        "Invalid provider response: NFT operator approval result must be ABI bool",
    ))
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
