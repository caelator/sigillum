use serde_json::json;
use sha3::{Digest, Keccak256};

use crate::service::{ServiceError, ServiceResult};

use super::ProviderRpcClient;
use crate::service::evm::{normalize_address, normalize_hex_blob, parse_quantity_u256};

pub(in crate::service::evm) struct Permit2Allowance {
    pub(in crate::service::evm) amount: [u8; 32],
    pub(in crate::service::evm) expiration_unix: u64,
}

impl ProviderRpcClient {
    pub(in crate::service::evm) async fn get_permit2_allowance(
        &self,
        permit2_address: &str,
        owner_address: &str,
        token_address: &str,
        spender_address: &str,
        block_tag: &str,
    ) -> ServiceResult<Permit2Allowance> {
        let value = self
            .request(
                1,
                "eth_call",
                json!([{
                    "to": normalize_address(permit2_address)?,
                    "data": permit2_allowance_call_data(owner_address, token_address, spender_address)?,
                }, block_tag]),
            )
            .await?;
        parse_permit2_allowance_result(&value)
    }
}

fn permit2_allowance_call_data(
    owner_address: &str,
    token_address: &str,
    spender_address: &str,
) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}{}{}",
        function_selector_hex("allowance(address,address,address)"),
        encoded_address_arg(owner_address)?,
        encoded_address_arg(token_address)?,
        encoded_address_arg(spender_address)?
    ))
}

fn parse_permit2_allowance_result(value: &serde_json::Value) -> ServiceResult<Permit2Allowance> {
    let result = value.as_str().ok_or_else(|| {
        ServiceError::internal("Invalid provider response: expected Permit2 allowance hex")
    })?;
    let normalized = normalize_hex_blob(result, "Permit2 allowance result")?;
    let raw = &normalized[2..];
    if raw.len() != 64 * 3 {
        return Err(ServiceError::internal(
            "Invalid provider response: Permit2 allowance result must be three ABI words",
        ));
    }
    Ok(Permit2Allowance {
        amount: parse_quantity_u256(&serde_json::Value::String(format!("0x{}", &raw[0..64])))?,
        expiration_unix: parse_word_u64(&raw[64..128], "Permit2 allowance expiration")?,
    })
}

fn encoded_address_arg(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("{}{}", "0".repeat(24), &normalized[2..]))
}

fn function_selector_hex(signature: &str) -> String {
    let digest = Keccak256::digest(signature.as_bytes());
    hex::encode(&digest[..4])
}

fn parse_word_u64(word: &str, label: &str) -> ServiceResult<u64> {
    let trimmed = word.trim_start_matches('0');
    if trimmed.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(trimmed, 16)
        .map_err(|error| ServiceError::internal(format!("Invalid {label}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_standard_function_selector() {
        assert_eq!(
            function_selector_hex("transfer(address,uint256)"),
            "a9059cbb"
        );
    }

    #[test]
    fn parses_permit2_allowance_tuple() {
        let value = serde_json::Value::String(format!(
            "0x{:064x}{:064x}{:064x}",
            1_000_000u64, 1_879_048_191u64, 7u64
        ));
        let parsed = parse_permit2_allowance_result(&value).unwrap();
        assert_eq!(parsed.amount[28..], [0x00, 0x0f, 0x42, 0x40]);
        assert_eq!(parsed.expiration_unix, 1_879_048_191);
    }
}
