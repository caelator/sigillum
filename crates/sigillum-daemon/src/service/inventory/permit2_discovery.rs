use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::{Permit2AllowanceProbe, normalize_address};

pub(super) const DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE: &str = "permit2-allowance-probe";

const DEFAULT_PERMIT2_CONTRACT_ADDRESS: &str = "0x000000000022d473030f116ddee9f6b43ac78ba3";
const DEFAULT_PERMIT2_ALLOWANCE_LIMIT: usize = 250;
const MAX_PERMIT2_ALLOWANCE_LIMIT: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct Permit2AllowanceDiscoveryConfig {
    pub(super) contract_addresses: Vec<String>,
    pub(super) spender_addresses: Vec<String>,
    pub(super) limit: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Permit2AllowanceObservation {
    pub(super) permit2_address: String,
    pub(super) token_address: String,
    pub(super) spender_address: String,
    pub(super) amount_hex: String,
}

pub(super) fn permit2_allowance_discovery_config(
    enabled: Option<bool>,
    contract_addresses: &[String],
    spender_addresses: &[String],
    limit: Option<usize>,
) -> ServiceResult<Option<Permit2AllowanceDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let mut contracts = Vec::new();
    if contract_addresses.is_empty() {
        contracts.push(DEFAULT_PERMIT2_CONTRACT_ADDRESS.to_string());
    } else {
        for contract in contract_addresses {
            push_unique_address(&mut contracts, normalize_address(contract)?);
        }
    }
    let mut spenders = Vec::new();
    for spender in spender_addresses {
        push_unique_address(&mut spenders, normalize_address(spender)?);
    }
    if spenders.is_empty() {
        return Err(ServiceError::bad_request(
            "permit2_spender_addresses is required when Permit2 allowance discovery is enabled",
        ));
    }
    Ok(Some(Permit2AllowanceDiscoveryConfig {
        contract_addresses: contracts,
        spender_addresses: spenders,
        limit: validated_permit2_allowance_limit(limit)?,
    }))
}

impl SigillumService {
    pub(super) async fn discover_permit2_allowances_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        token_addresses: &[String],
        block_tag: &str,
        config: &Permit2AllowanceDiscoveryConfig,
    ) -> ServiceResult<Vec<Permit2AllowanceObservation>> {
        let mut observations = Vec::new();
        for permit2_address in &config.contract_addresses {
            for token_address in token_addresses {
                for spender_address in &config.spender_addresses {
                    if observations.len() >= config.limit {
                        return Ok(observations);
                    }
                    let (amount_hex, expiration_unix) = self
                        .evm_permit2_allowance_for_provider(
                            provider.compartment_id,
                            provider,
                            Permit2AllowanceProbe {
                                permit2_address,
                                owner_address,
                                token_address,
                                spender_address,
                                block_tag,
                            },
                        )
                        .await?;
                    observations.push(Permit2AllowanceObservation {
                        permit2_address: permit2_address.clone(),
                        token_address: token_address.clone(),
                        spender_address: spender_address.clone(),
                        amount_hex: if expiration_unix == 0 {
                            "0x0".into()
                        } else {
                            amount_hex
                        },
                    });
                }
            }
        }
        Ok(observations)
    }
}

fn validated_permit2_allowance_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_PERMIT2_ALLOWANCE_LIMIT);
    if limit == 0 || limit > MAX_PERMIT2_ALLOWANCE_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "permit2_allowance_limit must be between 1 and {MAX_PERMIT2_ALLOWANCE_LIMIT}"
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
    fn uses_canonical_contract_by_default() {
        let config = permit2_allowance_discovery_config(
            Some(true),
            &[],
            &["0x4444444444444444444444444444444444444444".to_string()],
            Some(8),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            config.contract_addresses,
            vec![DEFAULT_PERMIT2_CONTRACT_ADDRESS]
        );
        assert_eq!(config.limit, 8);
    }

    #[test]
    fn requires_spenders_when_enabled() {
        let error = permit2_allowance_discovery_config(Some(true), &[], &[], None).unwrap_err();
        assert!(error.to_string().contains("permit2_spender_addresses"));
        assert!(
            permit2_allowance_discovery_config(Some(false), &[], &[], None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn normalizes_and_deduplicates_contracts_and_spenders() {
        let contracts = vec![
            "0X000000000022D473030F116dDEE9F6B43aC78BA3".to_string(),
            DEFAULT_PERMIT2_CONTRACT_ADDRESS.to_string(),
        ];
        let spenders = vec![
            "0X4444444444444444444444444444444444444444".to_string(),
            "0x4444444444444444444444444444444444444444".to_string(),
        ];
        let config = permit2_allowance_discovery_config(Some(true), &contracts, &spenders, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            config.contract_addresses,
            vec![DEFAULT_PERMIT2_CONTRACT_ADDRESS]
        );
        assert_eq!(
            config.spender_addresses,
            vec!["0x4444444444444444444444444444444444444444"]
        );
    }

    #[test]
    fn validates_permit2_allowance_limit() {
        assert!(
            permit2_allowance_discovery_config(
                Some(true),
                &[],
                &["0x4444444444444444444444444444444444444444".to_string()],
                Some(MAX_PERMIT2_ALLOWANCE_LIMIT + 1)
            )
            .is_err()
        );
    }
}
