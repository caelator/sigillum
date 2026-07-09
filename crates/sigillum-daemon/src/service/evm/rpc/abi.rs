use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::service::evm::normalize_hex_blob;
use crate::service::{ServiceError, ServiceResult};

const NFT_TOKEN_URI_MAX_BYTES: usize = 2048;
const ERC721_TOKEN_URI_SELECTOR: &str = "c87b56dd";
const ERC1155_URI_SELECTOR: &str = "0e89341c";

pub(super) fn nft_token_uri_call_data(token_id_hex: &str, erc1155: bool) -> ServiceResult<String> {
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

pub(super) fn parse_abi_string_result(value: &Value) -> ServiceResult<String> {
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
