use sha3::{Digest, Keccak256};
use sigillum_api::{DefiTokenProbe, EvmProviderProfile};

use crate::service::evm::EvmContractCallPreflight;
use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::normalize_address;
use super::defi_adapters::{DEFI_EXIT_ADAPTER_ERC4626_REDEEM, adapter_for_protocol};

pub(super) const DISCOVERY_SOURCE_DEFI_TOKEN_PROBE_PREFIX: &str = "defi-token-probe";

const DEFAULT_DEFI_POSITION_LIMIT: usize = 100;
const MAX_DEFI_POSITION_LIMIT: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct DefiTokenPositionDiscoveryConfig {
    pub(super) probes: Vec<DefiTokenPositionProbe>,
    pub(super) limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefiTokenPositionProbe {
    pub(super) protocol: String,
    pub(super) token_address: String,
    pub(super) protocol_address: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefiTokenPositionObservation {
    pub(super) protocol: String,
    pub(super) token_address: String,
    pub(super) protocol_address: Option<String>,
    pub(super) claim_adapter: Option<String>,
    pub(super) amount_hex: String,
}

pub(super) fn defi_token_position_discovery_config(
    enabled: Option<bool>,
    probes: &[DefiTokenProbe],
    limit: Option<usize>,
) -> ServiceResult<Option<DefiTokenPositionDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }

    let mut normalized_probes = Vec::new();
    for probe in probes {
        push_unique_probe(
            &mut normalized_probes,
            DefiTokenPositionProbe {
                protocol: normalized_protocol_label(&probe.protocol)?,
                token_address: normalize_address(&probe.token_address)?,
                protocol_address: probe
                    .protocol_address
                    .as_deref()
                    .map(normalize_address)
                    .transpose()?,
            },
        );
    }
    if normalized_probes.is_empty() {
        return Err(ServiceError::bad_request(
            "defi_token_probes is required when DeFi token position discovery is enabled",
        ));
    }

    Ok(Some(DefiTokenPositionDiscoveryConfig {
        probes: normalized_probes,
        limit: validated_defi_position_limit(limit)?,
    }))
}

impl SigillumService {
    pub(super) async fn discover_defi_token_positions_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        block_tag: &str,
        config: &DefiTokenPositionDiscoveryConfig,
    ) -> ServiceResult<Vec<DefiTokenPositionObservation>> {
        let mut observations = Vec::new();
        for probe in &config.probes {
            if observations.len() >= config.limit {
                break;
            }
            let amount_hex = self
                .evm_erc20_balance_for_provider(
                    provider.compartment_id,
                    provider,
                    &probe.token_address,
                    owner_address,
                    block_tag,
                )
                .await?;
            let mut protocol_address = probe.protocol_address.clone();
            let (claim_adapter, amount_hex) = if probe.protocol == "erc4626" {
                match self
                    .verified_erc4626_redeem_amount(
                        provider,
                        owner_address,
                        block_tag,
                        &probe.token_address,
                    )
                    .await
                {
                    Some(amount_hex) => {
                        if protocol_address.is_none() {
                            protocol_address = Some(probe.token_address.clone());
                        }
                        (
                            Some(DEFI_EXIT_ADAPTER_ERC4626_REDEEM.to_string()),
                            amount_hex,
                        )
                    }
                    None => (None, amount_hex),
                }
            } else {
                (
                    adapter_for_protocol(&probe.protocol).map(str::to_string),
                    amount_hex,
                )
            };
            observations.push(DefiTokenPositionObservation {
                protocol: probe.protocol.clone(),
                token_address: probe.token_address.clone(),
                protocol_address,
                claim_adapter,
                amount_hex,
            });
        }
        Ok(observations)
    }

    async fn verified_erc4626_redeem_amount(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        block_tag: &str,
        vault_address: &str,
    ) -> Option<String> {
        let max_redeem_data = erc4626_max_redeem_call_data(owner_address).ok()?;
        let max_redeem_result = self
            .evm_contract_call_preflight_for_provider(
                provider.compartment_id,
                provider,
                EvmContractCallPreflight {
                    from_address: owner_address,
                    target_address: vault_address,
                    data_hex: &max_redeem_data,
                    value_hex: None,
                    block_tag,
                },
            )
            .await
            .ok()?;
        let max_redeem_word = strict_single_word_hex(&max_redeem_result)?;

        let convert_to_assets_data = erc4626_convert_to_assets_call_data(&max_redeem_word);
        let convert_result = self
            .evm_contract_call_preflight_for_provider(
                provider.compartment_id,
                provider,
                EvmContractCallPreflight {
                    from_address: owner_address,
                    target_address: vault_address,
                    data_hex: &convert_to_assets_data,
                    value_hex: None,
                    block_tag,
                },
            )
            .await
            .ok()?;
        strict_single_word_hex(&convert_result)?;

        Some(canonical_quantity_hex_from_word(&max_redeem_word))
    }
}

pub(super) fn defi_token_probe_source(protocol: &str) -> String {
    format!("{DISCOVERY_SOURCE_DEFI_TOKEN_PROBE_PREFIX}:{protocol}")
}

fn validated_defi_position_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_DEFI_POSITION_LIMIT);
    if limit == 0 || limit > MAX_DEFI_POSITION_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "defi_position_limit must be between 1 and {MAX_DEFI_POSITION_LIMIT}"
        )));
    }
    Ok(limit)
}

fn normalized_protocol_label(value: &str) -> ServiceResult<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(ServiceError::bad_request("defi protocol is required"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ServiceError::bad_request(
            "defi protocol may only contain ASCII letters, digits, hyphen, underscore, or dot",
        ));
    }
    Ok(value)
}

fn push_unique_probe(probes: &mut Vec<DefiTokenPositionProbe>, next: DefiTokenPositionProbe) {
    if !probes.iter().any(|existing| {
        existing.protocol == next.protocol
            && existing
                .token_address
                .eq_ignore_ascii_case(&next.token_address)
            && existing.protocol_address == next.protocol_address
    }) {
        probes.push(next);
    }
}

fn erc4626_max_redeem_call_data(owner_address: &str) -> ServiceResult<String> {
    Ok(format!(
        "0x{}{}",
        function_selector_hex("maxRedeem(address)"),
        encoded_address_arg(owner_address)?
    ))
}

fn erc4626_convert_to_assets_call_data(shares_word: &str) -> String {
    format!(
        "0x{}{}",
        function_selector_hex("convertToAssets(uint256)"),
        shares_word
    )
}

fn encoded_address_arg(address: &str) -> ServiceResult<String> {
    let normalized = normalize_address(address)?;
    Ok(format!("{}{}", "0".repeat(24), &normalized[2..]))
}

fn strict_single_word_hex(value: &str) -> Option<String> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(raw.to_ascii_lowercase())
}

fn canonical_quantity_hex_from_word(word: &str) -> String {
    let raw = word.trim_start_matches('0');
    if raw.is_empty() {
        "0x0".into()
    } else {
        format!("0x{raw}")
    }
}

fn function_selector_hex(signature: &str) -> String {
    let digest = Keccak256::digest(signature.as_bytes());
    hex::encode(&digest[..4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_probes_when_enabled() {
        let error = defi_token_position_discovery_config(Some(true), &[], None).unwrap_err();
        assert!(error.to_string().contains("defi_token_probes"));
        assert!(
            defi_token_position_discovery_config(Some(false), &[], None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn normalizes_and_deduplicates_probes() {
        let probes = vec![
            DefiTokenProbe {
                protocol: "Aave-V3".into(),
                token_address: "0X4D5F47FA6A74757F35C14FD3A6EF8E3C9BC514E8".into(),
                protocol_address: Some("0X87870BCA3F3FD6335C3F4CE8392D69350B4FA4E2".into()),
            },
            DefiTokenProbe {
                protocol: "aave-v3".into(),
                token_address: "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".into(),
                protocol_address: Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2".into()),
            },
        ];

        let config = defi_token_position_discovery_config(Some(true), &probes, Some(7))
            .unwrap()
            .unwrap();

        assert_eq!(config.probes.len(), 1);
        assert_eq!(config.probes[0].protocol, "aave-v3");
        assert_eq!(
            config.probes[0].token_address,
            "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8"
        );
        assert_eq!(
            config.probes[0].protocol_address.as_deref(),
            Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2")
        );
        assert_eq!(config.limit, 7);
    }

    #[test]
    fn validates_protocol_labels_and_limit() {
        let bad_protocol = vec![DefiTokenProbe {
            protocol: "Aave V3".into(),
            token_address: "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".into(),
            protocol_address: None,
        }];
        assert!(defi_token_position_discovery_config(Some(true), &bad_protocol, None).is_err());

        let probe = vec![DefiTokenProbe {
            protocol: "aave-v3".into(),
            token_address: "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".into(),
            protocol_address: None,
        }];
        assert!(defi_token_position_discovery_config(Some(true), &probe, Some(0)).is_err());
    }
}
