use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::normalize_address;

pub(super) const DISCOVERY_SOURCE_ERC20_ALLOWANCE_PROBE: &str = "erc20-allowance-probe";

const DEFAULT_ALLOWANCE_DISCOVERY_LIMIT: usize = 250;
const MAX_ALLOWANCE_DISCOVERY_LIMIT: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct Erc20AllowanceDiscoveryConfig {
    pub(super) spender_addresses: Vec<String>,
    pub(super) limit: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Erc20AllowanceObservation {
    pub(super) token_address: String,
    pub(super) spender_address: String,
    pub(super) amount_hex: String,
}

pub(super) fn erc20_allowance_discovery_config(
    enabled: Option<bool>,
    spender_addresses: &[String],
    limit: Option<usize>,
) -> ServiceResult<Option<Erc20AllowanceDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let mut spenders = Vec::new();
    for spender in spender_addresses {
        push_unique_address(&mut spenders, normalize_address(spender)?);
    }
    if spenders.is_empty() {
        return Err(ServiceError::bad_request(
            "allowance_spender_addresses is required when ERC-20 allowance discovery is enabled",
        ));
    }
    Ok(Some(Erc20AllowanceDiscoveryConfig {
        spender_addresses: spenders,
        limit: validated_allowance_discovery_limit(limit)?,
    }))
}

impl SigillumService {
    pub(super) async fn discover_erc20_allowances_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        token_addresses: &[String],
        block_tag: &str,
        config: &Erc20AllowanceDiscoveryConfig,
    ) -> ServiceResult<Vec<Erc20AllowanceObservation>> {
        let mut observations = Vec::new();
        for token_address in token_addresses {
            for spender_address in &config.spender_addresses {
                if observations.len() >= config.limit {
                    return Ok(observations);
                }
                let amount_hex = self
                    .evm_erc20_allowance_for_provider(
                        provider.compartment_id,
                        provider,
                        token_address,
                        owner_address,
                        spender_address,
                        block_tag,
                    )
                    .await?;
                observations.push(Erc20AllowanceObservation {
                    token_address: token_address.clone(),
                    spender_address: spender_address.clone(),
                    amount_hex,
                });
            }
        }
        Ok(observations)
    }
}

fn validated_allowance_discovery_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_ALLOWANCE_DISCOVERY_LIMIT);
    if limit == 0 || limit > MAX_ALLOWANCE_DISCOVERY_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "allowance_discovery_limit must be between 1 and {MAX_ALLOWANCE_DISCOVERY_LIMIT}"
        )));
    }
    Ok(limit)
}

fn push_unique_address(addresses: &mut Vec<String>, address: String) {
    if !addresses
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&address))
    {
        addresses.push(address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_spenders_when_enabled() {
        let error = erc20_allowance_discovery_config(Some(true), &[], None).unwrap_err();
        assert!(error.to_string().contains("allowance_spender_addresses"));
        assert!(
            erc20_allowance_discovery_config(Some(false), &[], None)
                .unwrap()
                .is_none()
        );
        assert!(
            erc20_allowance_discovery_config(
                None,
                &["0x2222222222222222222222222222222222222222".to_string()],
                None
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn normalizes_and_deduplicates_spenders() {
        let spenders = vec![
            "0X2222222222222222222222222222222222222222".to_string(),
            "0x2222222222222222222222222222222222222222".to_string(),
        ];
        let config = erc20_allowance_discovery_config(Some(true), &spenders, Some(7))
            .unwrap()
            .unwrap();
        assert_eq!(
            config.spender_addresses,
            vec!["0x2222222222222222222222222222222222222222"]
        );
        assert_eq!(config.limit, 7);
    }

    #[test]
    fn validates_allowance_discovery_limit() {
        assert!(
            erc20_allowance_discovery_config(
                Some(true),
                &["0x2222222222222222222222222222222222222222".to_string()],
                Some(MAX_ALLOWANCE_DISCOVERY_LIMIT + 1)
            )
            .is_err()
        );
    }
}
