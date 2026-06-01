use sha3::{Digest, Keccak256};
use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::normalize_address;

pub(super) const DISCOVERY_SOURCE_ERC721_TRANSFER_LOG: &str = "erc721-transfer-log";
pub(super) const DISCOVERY_SOURCE_ERC1155_TRANSFER_LOG: &str = "erc1155-transfer-log";

const DEFAULT_NFT_DISCOVERY_LIMIT: usize = 100;
const MAX_NFT_DISCOVERY_LIMIT: usize = 1_000;
const ERC721_TRANSFER_EVENT: &str = "Transfer(address,address,uint256)";
const ERC1155_TRANSFER_SINGLE_EVENT: &str =
    "TransferSingle(address,address,address,uint256,uint256)";
const ERC1155_TRANSFER_BATCH_EVENT: &str =
    "TransferBatch(address,address,address,uint256[],uint256[])";

#[derive(Clone, Debug)]
pub(super) struct Erc721TransferDiscoveryConfig {
    pub(super) from_block: String,
    pub(super) to_block: String,
    pub(super) limit: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Erc1155TransferDiscoveryConfig {
    pub(super) from_block: String,
    pub(super) to_block: String,
    pub(super) limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Erc721HoldingObservation {
    pub(super) contract_address: String,
    pub(super) token_id_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Erc1155HoldingObservation {
    pub(super) contract_address: String,
    pub(super) token_id_hex: String,
    pub(super) amount_hex: String,
}

pub(super) fn erc721_transfer_discovery_config(
    enabled: Option<bool>,
    from_block: Option<&str>,
    to_block: Option<&str>,
    limit: Option<usize>,
) -> ServiceResult<Option<Erc721TransferDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let from_block = from_block
        .ok_or_else(|| {
            ServiceError::bad_request(
                "nft_discovery_from_block is required when ERC-721 transfer discovery is enabled",
            )
        })
        .and_then(|value| normalize_log_block_tag(value, "nft_discovery_from_block"))?;
    let to_block = to_block
        .map(|value| normalize_log_block_tag(value, "nft_discovery_to_block"))
        .transpose()?
        .unwrap_or_else(|| "latest".into());
    Ok(Some(Erc721TransferDiscoveryConfig {
        from_block,
        to_block,
        limit: validated_nft_discovery_limit(limit)?,
    }))
}

pub(super) fn erc1155_transfer_discovery_config(
    enabled: Option<bool>,
    from_block: Option<&str>,
    to_block: Option<&str>,
    limit: Option<usize>,
) -> ServiceResult<Option<Erc1155TransferDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let from_block = from_block
        .ok_or_else(|| {
            ServiceError::bad_request(
                "nft_discovery_from_block is required when ERC-1155 transfer discovery is enabled",
            )
        })
        .and_then(|value| normalize_log_block_tag(value, "nft_discovery_from_block"))?;
    let to_block = to_block
        .map(|value| normalize_log_block_tag(value, "nft_discovery_to_block"))
        .transpose()?
        .unwrap_or_else(|| "latest".into());
    Ok(Some(Erc1155TransferDiscoveryConfig {
        from_block,
        to_block,
        limit: validated_nft_discovery_limit(limit)?,
    }))
}

impl SigillumService {
    pub(super) async fn discover_erc721_transfer_holdings_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        config: &Erc721TransferDiscoveryConfig,
    ) -> ServiceResult<Vec<Erc721HoldingObservation>> {
        let owner_address = normalize_address(owner_address)?;
        let owner_topic = padded_address_topic(&owner_address)?;
        let transfer_topic = erc721_transfer_topic();
        let outgoing_topics = vec![
            Some(transfer_topic.clone()),
            Some(owner_topic.clone()),
            None,
            None,
        ];
        let incoming_topics = vec![Some(transfer_topic), None, Some(owner_topic), None];
        let mut candidates = Vec::new();

        for topics in [&outgoing_topics, &incoming_topics] {
            if candidates.len() >= config.limit {
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
            for log in logs.into_iter().filter(|log| log.topics.len() >= 4) {
                push_unique_candidate(
                    &mut candidates,
                    Erc721HoldingObservation {
                        contract_address: log.address,
                        token_id_hex: log.topics[3].clone(),
                    },
                );
                if candidates.len() >= config.limit {
                    break;
                }
            }
        }

        let mut confirmed = Vec::new();
        for candidate in candidates {
            let owner = self
                .evm_erc721_owner_for_provider(
                    provider.compartment_id,
                    provider,
                    &candidate.contract_address,
                    &candidate.token_id_hex,
                    &config.to_block,
                )
                .await;
            if owner.is_ok_and(|owner| owner == owner_address) {
                confirmed.push(candidate);
            }
        }
        Ok(confirmed)
    }

    pub(super) async fn discover_erc1155_transfer_holdings_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        config: &Erc1155TransferDiscoveryConfig,
    ) -> ServiceResult<Vec<Erc1155HoldingObservation>> {
        let owner_address = normalize_address(owner_address)?;
        let mut candidates = Vec::new();
        self.collect_erc1155_candidates_for_event(
            provider,
            &owner_address,
            &erc1155_transfer_single_topic(),
            config,
            &mut candidates,
            erc1155_single_token_ids,
        )
        .await?;
        self.collect_erc1155_candidates_for_event(
            provider,
            &owner_address,
            &erc1155_transfer_batch_topic(),
            config,
            &mut candidates,
            erc1155_batch_token_ids,
        )
        .await?;

        let mut confirmed = Vec::new();
        for candidate in candidates {
            let amount_hex = self
                .evm_erc1155_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    &candidate.contract_address,
                    &owner_address,
                    &candidate.token_id_hex,
                    &config.to_block,
                )
                .await?;
            if super::support::quantity_hex_is_nonzero(&amount_hex) {
                confirmed.push(Erc1155HoldingObservation {
                    amount_hex,
                    ..candidate
                });
            }
        }
        Ok(confirmed)
    }

    async fn collect_erc1155_candidates_for_event(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        transfer_topic: &str,
        config: &Erc1155TransferDiscoveryConfig,
        candidates: &mut Vec<Erc1155HoldingObservation>,
        token_ids_from_log: fn(&str) -> ServiceResult<Vec<String>>,
    ) -> ServiceResult<()> {
        let owner_topic = padded_address_topic(owner_address)?;
        let outgoing_topics = vec![
            Some(transfer_topic.to_string()),
            None,
            Some(owner_topic.clone()),
            None,
        ];
        let incoming_topics = vec![
            Some(transfer_topic.to_string()),
            None,
            None,
            Some(owner_topic),
        ];

        for topics in [&outgoing_topics, &incoming_topics] {
            if candidates.len() >= config.limit {
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
            for log in logs.into_iter().filter(|log| log.topics.len() >= 4) {
                for token_id_hex in token_ids_from_log(&log.data)? {
                    push_unique_erc1155_candidate(
                        candidates,
                        Erc1155HoldingObservation {
                            contract_address: log.address.clone(),
                            token_id_hex,
                            amount_hex: "0x0".into(),
                        },
                    );
                    if candidates.len() >= config.limit {
                        break;
                    }
                }
                if candidates.len() >= config.limit {
                    break;
                }
            }
        }
        Ok(())
    }
}

fn validated_nft_discovery_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_NFT_DISCOVERY_LIMIT);
    if limit == 0 || limit > MAX_NFT_DISCOVERY_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "nft_discovery_limit must be between 1 and {MAX_NFT_DISCOVERY_LIMIT}"
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

fn erc721_transfer_topic() -> String {
    event_topic(ERC721_TRANSFER_EVENT)
}

fn erc1155_transfer_single_topic() -> String {
    event_topic(ERC1155_TRANSFER_SINGLE_EVENT)
}

fn erc1155_transfer_batch_topic() -> String {
    event_topic(ERC1155_TRANSFER_BATCH_EVENT)
}

fn event_topic(signature: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(signature.as_bytes())))
}

fn padded_address_topic(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("0x{}{}", "0".repeat(24), &normalized[2..]))
}

fn push_unique_candidate(
    candidates: &mut Vec<Erc721HoldingObservation>,
    next: Erc721HoldingObservation,
) {
    if !candidates.iter().any(|existing| {
        existing
            .contract_address
            .eq_ignore_ascii_case(&next.contract_address)
            && existing
                .token_id_hex
                .eq_ignore_ascii_case(&next.token_id_hex)
    }) {
        candidates.push(next);
    }
}

fn push_unique_erc1155_candidate(
    candidates: &mut Vec<Erc1155HoldingObservation>,
    next: Erc1155HoldingObservation,
) {
    if !candidates.iter().any(|existing| {
        existing
            .contract_address
            .eq_ignore_ascii_case(&next.contract_address)
            && existing
                .token_id_hex
                .eq_ignore_ascii_case(&next.token_id_hex)
    }) {
        candidates.push(next);
    }
}

fn erc1155_single_token_ids(data: &str) -> ServiceResult<Vec<String>> {
    Ok(vec![abi_word(data, 0, "ERC-1155 TransferSingle id")?])
}

fn erc1155_batch_token_ids(data: &str) -> ServiceResult<Vec<String>> {
    let ids_offset = abi_word_usize(data, 0, "ERC-1155 ids offset")?;
    let values_offset = abi_word_usize(data, 1, "ERC-1155 values offset")?;
    let ids = abi_array_words(data, ids_offset, "ERC-1155 ids")?;
    let values = abi_array_words(data, values_offset, "ERC-1155 values")?;
    if ids.len() != values.len() {
        return Err(ServiceError::internal(
            "Invalid provider response: ERC-1155 batch ids and values length mismatch",
        ));
    }
    Ok(ids)
}

fn abi_array_words(data: &str, offset_bytes: usize, label: &str) -> ServiceResult<Vec<String>> {
    if !offset_bytes.is_multiple_of(32) {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} offset is not word-aligned"
        )));
    }
    let length_word = offset_bytes / 32;
    let length = abi_word_usize(data, length_word, label)?;
    if length > MAX_NFT_DISCOVERY_LIMIT {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} length exceeds {MAX_NFT_DISCOVERY_LIMIT}"
        )));
    }
    let mut words = Vec::with_capacity(length);
    for index in 0..length {
        words.push(abi_word(data, length_word + 1 + index, label)?);
    }
    Ok(words)
}

fn abi_word_usize(data: &str, word_index: usize, label: &str) -> ServiceResult<usize> {
    let word = abi_word(data, word_index, label)?;
    let raw = word[2..].trim_start_matches('0');
    if raw.is_empty() {
        return Ok(0);
    }
    if raw.len() > std::mem::size_of::<usize>() * 2 {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} is too large"
        )));
    }
    usize::from_str_radix(raw, 16).map_err(|error| {
        ServiceError::internal(format!("Invalid provider response: {label}: {error}"))
    })
}

fn abi_word(data: &str, word_index: usize, label: &str) -> ServiceResult<String> {
    let raw = data
        .strip_prefix("0x")
        .or_else(|| data.strip_prefix("0X"))
        .ok_or_else(|| ServiceError::internal(format!("Invalid provider response: {label}")))?;
    if !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} is not hex"
        )));
    }
    let start = word_index
        .checked_mul(64)
        .ok_or_else(|| ServiceError::internal(format!("Invalid provider response: {label}")))?;
    let end = start
        .checked_add(64)
        .ok_or_else(|| ServiceError::internal(format!("Invalid provider response: {label}")))?;
    if raw.len() < end {
        return Err(ServiceError::internal(format!(
            "Invalid provider response: {label} missing ABI word"
        )));
    }
    Ok(format!("0x{}", raw[start..end].to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_bounded_from_block_when_enabled() {
        let error = erc721_transfer_discovery_config(Some(true), None, None, None).unwrap_err();
        assert!(error.to_string().contains("nft_discovery_from_block"));
        assert!(
            erc721_transfer_discovery_config(Some(false), None, None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn validates_nft_discovery_bounds() {
        let config = erc721_transfer_discovery_config(Some(true), Some("0X000abc"), None, Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(config.from_block, "0x000abc");
        assert_eq!(config.to_block, "latest");
        assert_eq!(config.limit, 5);
        assert!(erc721_transfer_discovery_config(Some(true), Some("1"), None, Some(5)).is_err());
        assert!(
            erc721_transfer_discovery_config(
                Some(true),
                Some("0x1"),
                None,
                Some(MAX_NFT_DISCOVERY_LIMIT + 1)
            )
            .is_err()
        );
    }

    #[test]
    fn erc1155_single_ids_decode_first_data_word() {
        let data = format!("0x{}7b{}2a", "0".repeat(62), "0".repeat(62));
        assert_eq!(
            erc1155_single_token_ids(&data).unwrap(),
            vec!["0x000000000000000000000000000000000000000000000000000000000000007b"]
        );
    }

    #[test]
    fn erc1155_batch_ids_decode_dynamic_array() {
        let data = format!(
            "0x{}40{}a0{}02{}7b{}7c{}02{}01{}02",
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
            "0".repeat(62),
        );
        assert_eq!(
            erc1155_batch_token_ids(&data).unwrap(),
            vec![
                "0x000000000000000000000000000000000000000000000000000000000000007b",
                "0x000000000000000000000000000000000000000000000000000000000000007c",
            ]
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
