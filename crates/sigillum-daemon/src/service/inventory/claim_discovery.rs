use sigillum_api::{ClaimCandidateProbe, WalletAssetKind};
use sigillum_core::decode_quantity_hex;

use crate::service::{ServiceError, ServiceResult};

use super::super::evm::{encode_quantity_u256, normalize_address};
use super::support::quantity_hex_is_nonzero;

pub(super) const DISCOVERY_SOURCE_CLAIM_CANDIDATE_PREFIX: &str = "claim-candidate";
pub(super) const CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1: &str = "merkle-distributor-v1";

const DEFAULT_CLAIM_CANDIDATE_LIMIT: usize = 100;
const MAX_CLAIM_CANDIDATE_LIMIT: usize = 1_000;
const MAX_CLAIM_PROOF_WORDS: usize = 64;

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
    pub(super) claim_adapter: Option<String>,
    pub(super) claim_index_hex: Option<String>,
    pub(super) claim_proof: Vec<String>,
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
                claim_adapter: normalized_claim_adapter(probe.claim_adapter.as_deref())?,
                claim_index_hex: normalized_optional_quantity(
                    "claim candidate index",
                    probe.claim_index_hex.as_deref(),
                )?,
                claim_proof: normalized_claim_proof(&probe.claim_proof)?,
            },
        );
    }
    if candidates.is_empty() {
        return Err(ServiceError::bad_request(
            "claim_candidate_probes is required when claim candidate discovery is enabled",
        ));
    }
    for candidate in &candidates {
        validate_claim_adapter_evidence(candidate)?;
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
        value
            if value == WalletAssetKind::Airdrop.as_str()
                || value == WalletAssetKind::Reward.as_str() =>
        {
            Ok(value.to_string())
        }
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

fn normalized_optional_quantity(field: &str, value: Option<&str>) -> ServiceResult<Option<String>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| normalized_quantity(field, value))
        .transpose()
}

fn normalized_quantity(field: &str, value: &str) -> ServiceResult<String> {
    let decoded = decode_quantity_hex(value)
        .map_err(|_| ServiceError::bad_request(format!("{field} must be valid hex")))?;
    Ok(encode_quantity_u256(&decoded))
}

fn normalized_claim_adapter(value: Option<&str>) -> ServiceResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let value = normalized_label("claim adapter", value)?;
    match value.as_str() {
        CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1 => Ok(Some(value)),
        _ => Err(ServiceError::bad_request(format!(
            "claim adapter must be {CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1}"
        ))),
    }
}

fn normalized_claim_proof(values: &[String]) -> ServiceResult<Vec<String>> {
    if values.len() > MAX_CLAIM_PROOF_WORDS {
        return Err(ServiceError::bad_request(format!(
            "claim proof exceeds maximum length of {MAX_CLAIM_PROOF_WORDS} words"
        )));
    }
    values
        .iter()
        .map(|value| normalized_proof_word(value))
        .collect()
}

fn normalized_proof_word(value: &str) -> ServiceResult<String> {
    let raw = value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim());
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(
            "claim proof words must be 32-byte hex values",
        ));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn validate_claim_adapter_evidence(candidate: &ClaimCandidate) -> ServiceResult<()> {
    if candidate.claim_adapter.is_none() {
        if candidate.claim_index_hex.is_some() || !candidate.claim_proof.is_empty() {
            return Err(ServiceError::bad_request(
                "claim_adapter is required when claim index or proof is provided",
            ));
        }
        return Ok(());
    }

    if candidate.claim_index_hex.is_none() {
        return Err(ServiceError::bad_request(
            "claim_index_hex is required for Merkle claim candidates",
        ));
    }
    if candidate.claim_proof.is_empty() {
        return Err(ServiceError::bad_request(
            "claim_proof is required for Merkle claim candidates",
        ));
    }
    Ok(())
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
            && existing.claim_adapter == next.claim_adapter
            && existing.claim_index_hex == next.claim_index_hex
            && existing.claim_proof == next.claim_proof
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
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
        }
    }

    fn merkle_candidate(kind: &str) -> ClaimCandidateProbe {
        let mut candidate = candidate(kind);
        candidate.claim_adapter = Some("Merkle-Distributor-V1".into());
        candidate.claim_index_hex = Some("0X0007".into());
        candidate.claim_proof = vec![
            format!("0X{}", "11".repeat(32)),
            format!("0X{}", "22".repeat(32)),
        ];
        candidate
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
    fn normalizes_merkle_claim_evidence() {
        let config =
            claim_candidate_discovery_config(Some(true), &[merkle_candidate("reward")], None)
                .unwrap()
                .unwrap();
        let matches = config.candidates_for_address("0x9858effd232b4033e47d90003d41ec34ecaeda94");

        assert_eq!(
            matches[0].claim_adapter.as_deref(),
            Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1)
        );
        assert_eq!(matches[0].claim_index_hex.as_deref(), Some("0x7"));
        assert_eq!(matches[0].claim_proof[0], format!("0x{}", "11".repeat(32)));
        assert_eq!(matches[0].claim_proof[1], format!("0x{}", "22".repeat(32)));
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

        let mut proof_without_adapter = candidate("reward");
        proof_without_adapter.claim_proof = vec![format!("0x{}", "11".repeat(32))];
        assert!(
            claim_candidate_discovery_config(Some(true), &[proof_without_adapter], None).is_err()
        );

        let mut missing_proof = candidate("reward");
        missing_proof.claim_adapter = Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into());
        missing_proof.claim_index_hex = Some("0x1".into());
        assert!(claim_candidate_discovery_config(Some(true), &[missing_proof], None).is_err());

        let mut malformed_proof = merkle_candidate("reward");
        malformed_proof.claim_proof = vec!["0x1234".into()];
        assert!(claim_candidate_discovery_config(Some(true), &[malformed_proof], None).is_err());
    }
}
