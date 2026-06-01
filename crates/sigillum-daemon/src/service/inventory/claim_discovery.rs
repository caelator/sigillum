use sigillum_api::ClaimCandidateProbe;
use sigillum_core::decode_quantity_hex;

use crate::service::{ServiceError, ServiceResult};

use super::super::evm::{encode_quantity_u256, normalize_address};
use super::support::quantity_hex_is_nonzero;

pub(super) const DISCOVERY_SOURCE_CLAIM_CANDIDATE_PREFIX: &str = "claim-candidate";

const DEFAULT_CLAIM_CANDIDATE_LIMIT: usize = 100;
const MAX_CLAIM_CANDIDATE_LIMIT: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct ClaimCandidateDiscoveryConfig {
    candidates: Vec<ClaimCandidate>,
    limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClaimCandidate {
    pub(super) kind: String,
    pub(super) protocol: String,
    pub(super) claimant_address: String,
    pub(super) claim_contract_address: String,
    pub(super) asset_address: String,
    pub(super) amount_hex: String,
    pub(super) source_label: String,
}

pub(super) fn claim_candidate_discovery_config(
    enabled: Option<bool>,
    probes: &[ClaimCandidateProbe],
    limit: Option<usize>,
) -> ServiceResult<Option<ClaimCandidateDiscoveryConfig>> {
    if enabled != Some(true) {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for probe in probes {
        push_unique_candidate(
            &mut candidates,
            ClaimCandidate {
                kind: normalized_claim_kind(&probe.kind)?,
                protocol: normalized_label("claim protocol", &probe.protocol)?,
                claimant_address: normalize_address(&probe.claimant_address)?,
                claim_contract_address: normalize_address(&probe.claim_contract_address)?,
                asset_address: normalize_address(&probe.asset_address)?,
                amount_hex: normalized_claim_amount(&probe.amount_hex)?,
                source_label: normalized_label("claim source", &probe.source_label)?,
            },
        );
    }
    if candidates.is_empty() {
        return Err(ServiceError::bad_request(
            "claim_candidate_probes is required when claim candidate discovery is enabled",
        ));
    }

    Ok(Some(ClaimCandidateDiscoveryConfig {
        candidates,
        limit: validated_claim_candidate_limit(limit)?,
    }))
}

impl ClaimCandidateDiscoveryConfig {
    pub(super) fn candidates_for_address(&self, owner_address: &str) -> Vec<ClaimCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| {
                candidate
                    .claimant_address
                    .eq_ignore_ascii_case(owner_address)
            })
            .take(self.limit)
            .cloned()
            .collect()
    }
}

pub(super) fn claim_candidate_source(kind: &str, protocol: &str, source_label: &str) -> String {
    format!("{DISCOVERY_SOURCE_CLAIM_CANDIDATE_PREFIX}:{kind}:{protocol}:{source_label}")
}

fn validated_claim_candidate_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_CLAIM_CANDIDATE_LIMIT);
    if limit == 0 || limit > MAX_CLAIM_CANDIDATE_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "claim_candidate_limit must be between 1 and {MAX_CLAIM_CANDIDATE_LIMIT}"
        )));
    }
    Ok(limit)
}

fn normalized_claim_kind(value: &str) -> ServiceResult<String> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "airdrop" | "reward" => Ok(value),
        _ => Err(ServiceError::bad_request(
            "claim candidate kind must be either reward or airdrop",
        )),
    }
}

fn normalized_claim_amount(value: &str) -> ServiceResult<String> {
    let decoded = decode_quantity_hex(value)
        .map_err(|_| ServiceError::bad_request("claim candidate amount_hex must be valid hex"))?;
    let encoded = encode_quantity_u256(&decoded);
    if !quantity_hex_is_nonzero(&encoded) {
        return Err(ServiceError::bad_request(
            "claim candidate amount_hex must be non-zero",
        ));
    }
    Ok(encoded)
}

fn normalized_label(label: &str, value: &str) -> ServiceResult<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(ServiceError::bad_request(format!("{label} is required")));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ServiceError::bad_request(format!(
            "{label} may only contain ASCII letters, digits, hyphen, underscore, or dot"
        )));
    }
    Ok(value)
}

fn push_unique_candidate(candidates: &mut Vec<ClaimCandidate>, next: ClaimCandidate) {
    if !candidates.iter().any(|existing| {
        existing.kind == next.kind
            && existing.protocol == next.protocol
            && existing
                .claimant_address
                .eq_ignore_ascii_case(&next.claimant_address)
            && existing
                .claim_contract_address
                .eq_ignore_ascii_case(&next.claim_contract_address)
            && existing
                .asset_address
                .eq_ignore_ascii_case(&next.asset_address)
            && existing.amount_hex == next.amount_hex
            && existing.source_label == next.source_label
    }) {
        candidates.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: &str) -> ClaimCandidateProbe {
        ClaimCandidateProbe {
            kind: kind.into(),
            protocol: "Optimism".into(),
            claimant_address: "0X9858EFFD232B4033E47D90003D41EC34ECAEDA94".into(),
            claim_contract_address: "0X1111111111111111111111111111111111111111".into(),
            asset_address: "0X2222222222222222222222222222222222222222".into(),
            amount_hex: "0X000F4240".into(),
            source_label: "OP-Token-List".into(),
        }
    }

    #[test]
    fn requires_candidates_when_enabled() {
        let error = claim_candidate_discovery_config(Some(true), &[], None).unwrap_err();
        assert!(error.to_string().contains("claim_candidate_probes"));
        assert!(
            claim_candidate_discovery_config(Some(false), &[], None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn normalizes_deduplicates_and_filters_by_claimant() {
        let probes = vec![candidate("AIRDROP"), candidate("airdrop")];
        let config = claim_candidate_discovery_config(Some(true), &probes, Some(7))
            .unwrap()
            .unwrap();

        let matches = config.candidates_for_address("0x9858effd232b4033e47d90003d41ec34ecaeda94");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].kind, "airdrop");
        assert_eq!(matches[0].protocol, "optimism");
        assert_eq!(
            matches[0].claim_contract_address,
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(
            matches[0].asset_address,
            "0x2222222222222222222222222222222222222222"
        );
        assert_eq!(matches[0].amount_hex, "0xf4240");
        assert_eq!(matches[0].source_label, "op-token-list");
        assert!(
            config
                .candidates_for_address("0x3333333333333333333333333333333333333333")
                .is_empty()
        );
    }

    #[test]
    fn validates_kind_labels_amounts_and_limit() {
        assert!(claim_candidate_discovery_config(Some(true), &[candidate("claim")], None).is_err());

        let mut bad_label = candidate("reward");
        bad_label.source_label = "protocol api".into();
        assert!(claim_candidate_discovery_config(Some(true), &[bad_label], None).is_err());

        let mut zero_amount = candidate("reward");
        zero_amount.amount_hex = "0x0".into();
        assert!(claim_candidate_discovery_config(Some(true), &[zero_amount], None).is_err());

        assert!(
            claim_candidate_discovery_config(Some(true), &[candidate("reward")], Some(0)).is_err()
        );
    }
}
