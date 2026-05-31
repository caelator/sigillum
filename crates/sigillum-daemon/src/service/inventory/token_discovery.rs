use sha3::{Digest, Keccak256};
use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::normalize_address;

pub(super) const DISCOVERY_SOURCE_ERC20_TRANSFER_LOG: &str = "erc20-transfer-log";

const DEFAULT_TOKEN_DISCOVERY_LIMIT: usize = 250;
const MAX_TOKEN_DISCOVERY_LIMIT: usize = 2_000;
const ERC20_TRANSFER_EVENT: &str = "Transfer(address,address,uint256)";

#[derive(Clone, Debug)]
pub(super) struct Erc20TransferDiscoveryConfig {
    pub(super) from_block: String,
    pub(super) to_block: String,
    pub(super) limit: usize,
}

pub(super) fn erc20_transfer_discovery_config(
    enabled: Option<bool>,
    from_block: Option<&str>,
    to_block: Option<&str>,
    limit: Option<usize>,
) -> ServiceResult<Option<Erc20TransferDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let from_block = from_block
        .ok_or_else(|| {
            ServiceError::bad_request(
                "token_discovery_from_block is required when ERC-20 transfer discovery is enabled",
            )
        })
        .and_then(|value| normalize_log_block_tag(value, "token_discovery_from_block"))?;
    let to_block = to_block
        .map(|value| normalize_log_block_tag(value, "token_discovery_to_block"))
        .transpose()?
        .unwrap_or_else(|| "latest".into());
    let limit = validated_token_discovery_limit(limit)?;
    Ok(Some(Erc20TransferDiscoveryConfig {
        from_block,
        to_block,
        limit,
    }))
}

impl SigillumService {
    pub(super) async fn discover_erc20_transfer_tokens_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        config: &Erc20TransferDiscoveryConfig,
    ) -> ServiceResult<Vec<String>> {
        let owner_topic = padded_address_topic(owner_address)?;
        let transfer_topic = erc20_transfer_topic();
        let outgoing_topics = vec![Some(transfer_topic.clone()), Some(owner_topic.clone())];
        let incoming_topics = vec![Some(transfer_topic), None, Some(owner_topic)];
        let mut tokens = Vec::new();

        for topics in [&outgoing_topics, &incoming_topics] {
            if tokens.len() >= config.limit {
                break;
            }
            let logs = self
                .evm_filtered_logs_for_provider(
                    provider.compartment_id,
                    provider,
                    None,
                    topics,
                    &config.from_block,
                    &config.to_block,
                )
                .await?;
            for log in logs {
                push_unique_token(&mut tokens, log.address);
                if tokens.len() >= config.limit {
                    break;
                }
            }
        }

        Ok(tokens)
    }
}

pub(super) fn push_unique_token(tokens: &mut Vec<String>, token: String) {
    if !tokens
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&token))
    {
        tokens.push(token);
    }
}

fn validated_token_discovery_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_TOKEN_DISCOVERY_LIMIT);
    if limit == 0 || limit > MAX_TOKEN_DISCOVERY_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "token_discovery_limit must be between 1 and {MAX_TOKEN_DISCOVERY_LIMIT}"
        )));
    }
    Ok(limit)
}

fn normalize_log_block_tag(value: &str, label: &str) -> ServiceResult<String> {
    let trimmed = value.trim();
    if matches!(
        trimmed,
        "earliest" | "latest" | "pending" | "safe" | "finalized"
    ) {
        return Ok(trimmed.into());
    }
    let raw = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .ok_or_else(|| {
            ServiceError::bad_request(format!("{label} must be a block tag or 0x quantity"))
        })?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(format!(
            "{label} must be a block tag or 0x quantity"
        )));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn erc20_transfer_topic() -> String {
    format!(
        "0x{}",
        hex::encode(Keccak256::digest(ERC20_TRANSFER_EVENT.as_bytes()))
    )
}

fn padded_address_topic(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("0x{}{}", "0".repeat(24), &normalized[2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_bounded_from_block_when_enabled() {
        let error = erc20_transfer_discovery_config(Some(true), None, None, None).unwrap_err();
        assert!(error.to_string().contains("token_discovery_from_block"));
        assert!(
            erc20_transfer_discovery_config(Some(false), None, None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn validates_transfer_discovery_bounds() {
        let config = erc20_transfer_discovery_config(Some(true), Some("0X000abc"), None, Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(config.from_block, "0x000abc");
        assert_eq!(config.to_block, "latest");
        assert_eq!(config.limit, 5);
        assert!(erc20_transfer_discovery_config(Some(true), Some("1"), None, Some(5)).is_err());
        assert!(
            erc20_transfer_discovery_config(
                Some(true),
                Some("0x1"),
                None,
                Some(MAX_TOKEN_DISCOVERY_LIMIT + 1)
            )
            .is_err()
        );
    }

    #[test]
    fn pads_owner_address_topic() {
        assert_eq!(
            padded_address_topic("0x1111111111111111111111111111111111111111").unwrap(),
            "0x0000000000000000000000001111111111111111111111111111111111111111"
        );
    }
}
