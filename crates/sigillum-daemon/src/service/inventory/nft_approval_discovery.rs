use sigillum_api::EvmProviderProfile;

use crate::service::{ServiceError, ServiceResult, SigillumService};

use super::super::evm::normalize_address;

pub(super) const DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE: &str = "nft-operator-approval-probe";

const DEFAULT_NFT_OPERATOR_APPROVAL_LIMIT: usize = 250;
const MAX_NFT_OPERATOR_APPROVAL_LIMIT: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct NftOperatorApprovalDiscoveryConfig {
    pub(super) operator_addresses: Vec<String>,
    pub(super) limit: usize,
}

#[derive(Clone, Debug)]
pub(super) struct NftOperatorApprovalObservation {
    pub(super) contract_address: String,
    pub(super) operator_address: String,
    pub(super) amount_hex: String,
}

pub(super) fn nft_operator_approval_discovery_config(
    enabled: Option<bool>,
    operator_addresses: &[String],
    limit: Option<usize>,
) -> ServiceResult<Option<NftOperatorApprovalDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }
    let mut operators = Vec::new();
    for operator in operator_addresses {
        push_unique_address(&mut operators, normalize_address(operator)?);
    }
    if operators.is_empty() {
        return Err(ServiceError::bad_request(
            "nft_operator_addresses is required when NFT operator approval discovery is enabled",
        ));
    }
    Ok(Some(NftOperatorApprovalDiscoveryConfig {
        operator_addresses: operators,
        limit: validated_nft_operator_approval_limit(limit)?,
    }))
}

impl SigillumService {
    pub(super) async fn discover_nft_operator_approvals_for_address(
        &self,
        provider: &EvmProviderProfile,
        owner_address: &str,
        nft_contract_addresses: &[String],
        block_tag: &str,
        config: &NftOperatorApprovalDiscoveryConfig,
    ) -> ServiceResult<Vec<NftOperatorApprovalObservation>> {
        let mut contracts = Vec::new();
        for contract in nft_contract_addresses {
            push_unique_address(&mut contracts, normalize_address(contract)?);
        }

        let mut observations = Vec::new();
        for contract_address in contracts {
            for operator_address in &config.operator_addresses {
                if observations.len() >= config.limit {
                    return Ok(observations);
                }
                let approved = self
                    .evm_nft_operator_approval_for_provider(
                        provider.compartment_id,
                        provider,
                        &contract_address,
                        owner_address,
                        operator_address,
                        block_tag,
                    )
                    .await?;
                observations.push(NftOperatorApprovalObservation {
                    contract_address: contract_address.clone(),
                    operator_address: operator_address.clone(),
                    amount_hex: if approved { "0x1" } else { "0x0" }.into(),
                });
            }
        }
        Ok(observations)
    }
}

fn validated_nft_operator_approval_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_NFT_OPERATOR_APPROVAL_LIMIT);
    if limit == 0 || limit > MAX_NFT_OPERATOR_APPROVAL_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "nft_operator_approval_limit must be between 1 and {MAX_NFT_OPERATOR_APPROVAL_LIMIT}"
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
    fn requires_operators_when_enabled() {
        let error = nft_operator_approval_discovery_config(Some(true), &[], None).unwrap_err();
        assert!(error.to_string().contains("nft_operator_addresses"));
        assert!(
            nft_operator_approval_discovery_config(Some(false), &[], None)
                .unwrap()
                .is_none()
        );
        assert!(
            nft_operator_approval_discovery_config(
                None,
                &["0x3333333333333333333333333333333333333333".to_string()],
                None
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn normalizes_and_deduplicates_operators() {
        let operators = vec![
            "0X3333333333333333333333333333333333333333".to_string(),
            "0x3333333333333333333333333333333333333333".to_string(),
        ];
        let config = nft_operator_approval_discovery_config(Some(true), &operators, Some(7))
            .unwrap()
            .unwrap();
        assert_eq!(
            config.operator_addresses,
            vec!["0x3333333333333333333333333333333333333333"]
        );
        assert_eq!(config.limit, 7);
    }

    #[test]
    fn validates_operator_approval_limit() {
        assert!(
            nft_operator_approval_discovery_config(
                Some(true),
                &["0x3333333333333333333333333333333333333333".to_string()],
                Some(MAX_NFT_OPERATOR_APPROVAL_LIMIT + 1)
            )
            .is_err()
        );
    }
}
