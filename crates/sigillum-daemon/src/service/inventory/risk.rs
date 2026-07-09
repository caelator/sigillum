use sigillum_api::{
    ChainProfile, DEFAULT_DORMANCY_BLOCK_WINDOW, RiskCatalogEntry, RiskFinding,
    WalletAddressClassification, WalletAssetHolding, WalletAssetKind, WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use super::nft_approval_discovery::DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE;
use super::permit2_discovery::DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE;
use super::support::quantity_hex_is_nonzero;

pub(super) fn derive_inventory_risk_findings(
    addresses: &[WalletInventoryAddress],
    holdings: &[WalletAssetHolding],
    risk_catalog: &[RiskCatalogEntry],
    chain_profiles: &[ChainProfile],
) -> Vec<RiskFinding> {
    let mut findings = Vec::new();
    for address in addresses {
        findings.extend(address_classification_findings(address, chain_profiles));
    }
    for holding in holdings
        .iter()
        .filter(|holding| quantity_hex_is_nonzero(&holding.amount_hex))
    {
        if holding.asset_kind == WalletAssetKind::Approval {
            let catalog_entry = holding
                .counterparty_address
                .as_deref()
                .and_then(|spender| risk_catalog_entry_for_address(risk_catalog, spender));
            findings.push(approval_finding(holding, catalog_entry));
            continue;
        }
        if is_claim_candidate_holding(holding) {
            let catalog_entry = holding
                .protocol_address
                .as_deref()
                .and_then(|claim_contract| {
                    risk_catalog_entry_for_address(risk_catalog, claim_contract)
                });
            findings.push(claim_candidate_finding(holding, catalog_entry));
        }
        if holding.asset_kind != WalletAssetKind::Native
            && native_balance_for_holding(addresses, holding)
                .is_none_or(|balance| !quantity_hex_is_nonzero(balance))
        {
            findings.push(RiskFinding {
                id: stable_finding_id("stranded_gas", holding),
                category: WalletAddressClassification::StrandedValue.as_str().into(),
                risk_level: "medium".into(),
                status: "open".into(),
                wallet_family: holding.wallet_family.clone(),
                wallet_profile: holding.wallet_profile.clone(),
                provider_profile: holding.provider_profile.clone(),
                chain_id: holding.chain_id,
                address: holding.address.clone(),
                subject_type: holding.asset_kind.to_string(),
                subject: holding
                    .asset_address
                    .clone()
                    .unwrap_or_else(|| WalletAssetKind::Native.as_str().into()),
                source: "local-risk-engine".into(),
                recommendation: "Fund gas or route through an approved sponsor before sweeping."
                    .into(),
                evidence: vec![
                    "Positive non-native holding detected".into(),
                    "No native gas balance detected for the same address".into(),
                ],
                first_seen_at_unix: holding.first_seen_at_unix,
                last_checked_at_unix: holding.last_checked_at_unix,
            });
        }
    }
    findings
}

fn is_claim_candidate_holding(holding: &WalletAssetHolding) -> bool {
    matches!(
        &holding.asset_kind,
        WalletAssetKind::Airdrop | WalletAssetKind::Reward
    )
}

fn address_classification_findings(
    address: &WalletInventoryAddress,
    chain_profiles: &[ChainProfile],
) -> Vec<RiskFinding> {
    let mut findings = Vec::new();
    if address_has_classification(address, &WalletAddressClassification::WatchOnly)
        && address_has_classification(address, &WalletAddressClassification::ValueDetected)
    {
        findings.push(address_finding(
            "watch_only_value",
            "medium",
            address,
            "Watch-only address has value. Import or connect the signer before planning any recovery action.",
            vec![
                "Address is visible but Sigillum cannot sign for it".into(),
                "Value was detected during inventory discovery".into(),
            ],
        ));
    }
    if address_has_classification(address, &WalletAddressClassification::DormantCandidate) {
        let window = dormancy_block_window_for_chain(chain_profiles, address.chain_id);
        let mut evidence = match address.last_activity_block {
            Some(block) => vec![format!("Last observed on-chain activity block: {block}")],
            None => vec![
                "No on-chain activity blocks observed; address has no outgoing transaction count"
                    .into(),
            ],
        };
        evidence.push(format!(
            "Dormancy block window: {window} blocks (chain {})",
            address.chain_id
        ));
        evidence.push(
            "Last-activity evidence is derived from observed transfer logs and stealth announcement scans; other activity may not be captured."
                .into(),
        );
        evidence.push(format!("Derivation path: {}", address.derivation_path));
        findings.push(address_finding(
            "dormant_wallet",
            "low",
            address,
            "Review this funded address as a dormant or historical receive address before consolidation.",
            evidence,
        ));
    }
    if address_has_classification(address, &WalletAddressClassification::StrandedValue) {
        findings.push(address_finding(
            WalletAddressClassification::StrandedValue.as_str(),
            "medium",
            address,
            "Fund gas or approve a gas sponsor before attempting token or NFT recovery from this address.",
            vec![
                "Non-native value was detected".into(),
                "No native gas balance was detected for the same address".into(),
            ],
        ));
    }
    findings
}

fn dormancy_block_window_for_chain(chain_profiles: &[ChainProfile], chain_id: u64) -> u64 {
    chain_profiles
        .iter()
        .find(|profile| profile.chain_id == Some(chain_id))
        .map(|profile| profile.dormancy_block_window)
        .filter(|window| *window > 0)
        .unwrap_or(DEFAULT_DORMANCY_BLOCK_WINDOW)
}

fn address_finding(
    category: &str,
    risk_level: &str,
    address: &WalletInventoryAddress,
    recommendation: &str,
    evidence: Vec<String>,
) -> RiskFinding {
    RiskFinding {
        id: stable_address_finding_id(category, address),
        category: category.into(),
        risk_level: risk_level.into(),
        status: "open".into(),
        wallet_family: address.wallet_family.clone(),
        wallet_profile: address.wallet_profile.clone(),
        provider_profile: address.provider_profile.clone(),
        chain_id: address.chain_id,
        address: address.address.clone(),
        subject_type: "address".into(),
        subject: address.address.clone(),
        source: "local-risk-engine".into(),
        recommendation: recommendation.into(),
        evidence,
        first_seen_at_unix: address.first_seen_at_unix,
        last_checked_at_unix: address.last_checked_at_unix,
    }
}

fn address_has_classification(
    address: &WalletInventoryAddress,
    classification: &WalletAddressClassification,
) -> bool {
    address
        .classifications
        .iter()
        .any(|value| value == classification)
}

fn claim_candidate_finding(
    holding: &WalletAssetHolding,
    catalog_entry: Option<&RiskCatalogEntry>,
) -> RiskFinding {
    let claim_contract = holding
        .protocol_address
        .clone()
        .unwrap_or_else(|| "unknown-claim-contract".into());
    let asset = holding
        .asset_address
        .clone()
        .unwrap_or_else(|| "unknown-asset".into());
    let mut risk_level = "medium".to_string();
    let (recommendation, mut evidence) = (
        "Review the claim source, contract, and future simulation evidence before signing any claim transaction.",
        vec![
            format!("Claim candidate kind: {}", holding.asset_kind),
            format!("Claim contract: {claim_contract}"),
            format!("Claim asset: {asset}"),
            format!("Claim amount: {}", holding.amount_hex),
            format!("Source: {}", holding.source),
            "Execution blocked: requires protocol-specific claim adapter".into(),
            "No blind claim signing or unknown claim contracts".into(),
        ],
    );
    if claim_contract == "unknown-claim-contract" {
        risk_level = "high".into();
        evidence.push("Claim contract is missing from inventory holding".into());
    }
    if let Some(entry) = catalog_entry {
        risk_level = risk_level_from_catalog_entry(entry);
        evidence.push(format!(
            "Risk catalog: {} ({})",
            entry.label, entry.risk_level
        ));
        evidence.push(format!("Catalog source: {}", entry.source));
        evidence.extend(
            entry
                .notes
                .iter()
                .map(|note| format!("Catalog note: {note}")),
        );
    }
    let recommendation = catalog_entry
        .map(|entry| claim_recommendation_with_catalog(entry, recommendation))
        .unwrap_or_else(|| recommendation.into());

    RiskFinding {
        id: stable_claim_finding_id(holding, &claim_contract),
        category: "claim_candidate".into(),
        risk_level,
        status: "open".into(),
        wallet_family: holding.wallet_family.clone(),
        wallet_profile: holding.wallet_profile.clone(),
        provider_profile: holding.provider_profile.clone(),
        chain_id: holding.chain_id,
        address: holding.address.clone(),
        subject_type: "claim_contract".into(),
        subject: claim_contract,
        source: "local-risk-engine".into(),
        recommendation,
        evidence,
        first_seen_at_unix: holding.first_seen_at_unix,
        last_checked_at_unix: holding.last_checked_at_unix,
    }
}

fn approval_finding(
    holding: &WalletAssetHolding,
    catalog_entry: Option<&RiskCatalogEntry>,
) -> RiskFinding {
    let spender = holding
        .counterparty_address
        .clone()
        .unwrap_or_else(|| "unknown-spender".into());
    let is_nft_operator_approval = holding.source == DISCOVERY_SOURCE_NFT_OPERATOR_APPROVAL_PROBE;
    let is_permit2_allowance = holding.source == DISCOVERY_SOURCE_PERMIT2_ALLOWANCE_PROBE;
    let base_risk_level = if is_nft_operator_approval || is_very_large_approval(&holding.amount_hex)
    {
        "high"
    } else {
        "medium"
    };
    let (recommendation, mut evidence) = if is_nft_operator_approval {
        (
            "Review the operator and revoke setApprovalForAll if it is no longer needed.",
            vec![
                format!(
                    "NFT collection {} has operator approval",
                    holding
                        .asset_address
                        .clone()
                        .unwrap_or_else(|| "unknown-collection".into())
                ),
                format!("Operator: {spender}"),
                "Approval: setApprovalForAll(true)".into(),
            ],
        )
    } else if is_permit2_allowance {
        (
            "Review the spender and revoke the Permit2 allowance if it is no longer needed.",
            vec![
                format!(
                    "Permit2 token {} has a non-zero allowance",
                    holding
                        .asset_address
                        .clone()
                        .unwrap_or_else(|| "unknown-token".into())
                ),
                format!("Spender: {spender}"),
                format!("Allowance: {}", holding.amount_hex),
                "Approval surface: Permit2 AllowanceTransfer".into(),
            ],
        )
    } else {
        (
            "Review the spender and revoke the allowance if it is no longer needed.",
            vec![
                format!(
                    "Token {} has a non-zero allowance",
                    holding
                        .asset_address
                        .clone()
                        .unwrap_or_else(|| "unknown-token".into())
                ),
                format!("Spender: {spender}"),
                format!("Allowance: {}", holding.amount_hex),
            ],
        )
    };
    let mut risk_level = base_risk_level.to_string();
    if let Some(entry) = catalog_entry {
        risk_level = risk_level_from_catalog_entry(entry);
        evidence.push(format!(
            "Risk catalog: {} ({})",
            entry.label, entry.risk_level
        ));
        evidence.push(format!("Catalog source: {}", entry.source));
        evidence.extend(
            entry
                .notes
                .iter()
                .map(|note| format!("Catalog note: {note}")),
        );
    }
    let recommendation = catalog_entry
        .map(|entry| recommendation_with_catalog(entry, recommendation))
        .unwrap_or_else(|| recommendation.into());

    RiskFinding {
        id: stable_approval_finding_id(holding, &spender),
        category: "risky_approval".into(),
        risk_level,
        status: "open".into(),
        wallet_family: holding.wallet_family.clone(),
        wallet_profile: holding.wallet_profile.clone(),
        provider_profile: holding.provider_profile.clone(),
        chain_id: holding.chain_id,
        address: holding.address.clone(),
        subject_type: WalletAssetKind::Approval.as_str().into(),
        subject: spender.clone(),
        source: "local-risk-engine".into(),
        recommendation,
        evidence,
        first_seen_at_unix: holding.first_seen_at_unix,
        last_checked_at_unix: holding.last_checked_at_unix,
    }
}

fn risk_catalog_entry_for_address<'a>(
    entries: &'a [RiskCatalogEntry],
    address: &str,
) -> Option<&'a RiskCatalogEntry> {
    entries
        .iter()
        .find(|entry| entry.address.eq_ignore_ascii_case(address))
}

fn risk_level_from_catalog_entry(entry: &RiskCatalogEntry) -> String {
    match entry.risk_level.as_str() {
        "trusted" => "low".into(),
        "low" | "medium" | "high" | "critical" => entry.risk_level.clone(),
        _ => "medium".into(),
    }
}

fn recommendation_with_catalog(entry: &RiskCatalogEntry, default: &str) -> String {
    match entry.risk_level.as_str() {
        "trusted" | "low" => format!(
            "Risk catalog marks {} as {}; keep this approval only if it is still intentional.",
            entry.label, entry.risk_level
        ),
        "high" | "critical" => format!(
            "Risk catalog marks {} as {}; revoke this approval unless there is an explicit operator exception.",
            entry.label, entry.risk_level
        ),
        "medium" => format!(
            "Risk catalog marks {} as medium risk; review this approval before consolidation.",
            entry.label
        ),
        _ => default.into(),
    }
}

fn claim_recommendation_with_catalog(entry: &RiskCatalogEntry, default: &str) -> String {
    match entry.risk_level.as_str() {
        "trusted" | "low" => format!(
            "Risk catalog marks {} as {}; still require protocol adapter verification and simulation before signing this claim.",
            entry.label, entry.risk_level
        ),
        "high" | "critical" => format!(
            "Risk catalog marks {} as {}; do not claim unless there is an explicit operator exception and verified adapter evidence.",
            entry.label, entry.risk_level
        ),
        "medium" => format!(
            "Risk catalog marks {} as medium risk; review this claim contract before any claim attempt.",
            entry.label
        ),
        _ => default.into(),
    }
}

fn native_balance_for_holding<'a>(
    addresses: &'a [WalletInventoryAddress],
    holding: &WalletAssetHolding,
) -> Option<&'a str> {
    addresses
        .iter()
        .find(|address| {
            address.wallet_family == holding.wallet_family
                && address.wallet_profile == holding.wallet_profile
                && address.provider_profile == holding.provider_profile
                && address.chain_id == holding.chain_id
                && address.address == holding.address
        })
        .map(|address| address.native_balance_wei_hex.as_str())
}

fn stable_finding_id(prefix: &str, holding: &WalletAssetHolding) -> String {
    format!(
        "{prefix}:{}:{}:{}:{}:{}",
        holding.wallet_family,
        holding.wallet_profile,
        holding.provider_profile,
        holding.chain_id,
        holding.address
    )
}

fn stable_address_finding_id(category: &str, address: &WalletInventoryAddress) -> String {
    format!(
        "{category}:{}:{}:{}:{}:{}",
        address.wallet_family,
        address.wallet_profile,
        address.provider_profile,
        address.chain_id,
        address.address
    )
}

fn stable_approval_finding_id(holding: &WalletAssetHolding, spender: &str) -> String {
    format!(
        "risky_approval:{}:{}:{}:{}:{}:{}:{}",
        holding.wallet_family,
        holding.wallet_profile,
        holding.provider_profile,
        holding.chain_id,
        holding.address,
        holding.asset_address.as_deref().unwrap_or("unknown-token"),
        spender
    )
}

fn stable_claim_finding_id(holding: &WalletAssetHolding, claim_contract: &str) -> String {
    format!(
        "claim_candidate:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        holding.wallet_family,
        holding.wallet_profile,
        holding.provider_profile,
        holding.chain_id,
        holding.address,
        holding.asset_kind,
        holding.asset_address.as_deref().unwrap_or("unknown-asset"),
        claim_contract,
        holding.source
    )
}

fn is_very_large_approval(amount_hex: &str) -> bool {
    decode_quantity_hex(amount_hex)
        .map(|amount| amount[0] >= 0x80)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_address(classifications: &[&str]) -> WalletInventoryAddress {
        WalletInventoryAddress {
            id: "addr_1".into(),
            wallet_family: "eth-xpub".into(),
            wallet_profile: "archive".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            derivation_pattern: Some("project".into()),
            account_index: Some(0),
            address_index: 0,
            activity_state: "funded".into(),
            native_balance_wei_hex: "0x0".into(),
            transaction_count: 0,
            last_activity_block: None,
            classifications: classifications
                .iter()
                .map(|value| (*value).into())
                .collect(),
            source: "local-rpc".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_claim_holding(kind: &str) -> WalletAssetHolding {
        WalletAssetHolding {
            id: "holding_1".into(),
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0x1111111111111111111111111111111111111111".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: kind.into(),
            asset_address: Some("0x4200000000000000000000000000000000000042".into()),
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: Some("0x2222222222222222222222222222222222222222".into()),
            claim_adapter: None,
            claim_index_hex: None,
            claim_proof: Vec::new(),
            metadata_uri: None,
            metadata_name: None,
            spam_label: None,
            amount_hex: "0xf4240".into(),
            source: format!("claim-candidate:{kind}:optimism:op-token-list"),
            status: "detected".into(),
            first_seen_at_unix: 1,
            last_checked_at_unix: 2,
        }
    }

    fn sample_catalog_entry(address: &str, risk_level: &str) -> RiskCatalogEntry {
        RiskCatalogEntry {
            address: address.into(),
            label: "Known claim contract".into(),
            risk_level: risk_level.into(),
            source: "operator".into(),
            notes: vec!["reviewed by operator".into()],
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    fn sample_chain_profile(window: u64) -> ChainProfile {
        ChainProfile {
            name: "ethereum".into(),
            chain_family: "evm".into(),
            chain_id: Some(1),
            provider_profile: None,
            native_symbol: "ETH".into(),
            native_decimals: 18,
            finality_blocks: 0,
            dormancy_block_window: window,
            permit2_address: None,
            explorer_url: None,
            capabilities: Vec::new(),
            enabled: true,
            source: "operator".into(),
            builtin: false,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    #[test]
    fn address_classifications_emit_watch_only_dormant_and_stranded_findings() {
        let findings = derive_inventory_risk_findings(
            &[sample_address(&[
                "watch_only",
                "value_detected",
                "stranded_value",
                "dormant_candidate",
            ])],
            &[],
            &[],
            &[],
        );

        assert!(findings.iter().any(|finding| {
            finding.category == "watch_only_value" && finding.risk_level == "medium"
        }));
        assert!(findings.iter().any(|finding| {
            finding.category == "dormant_wallet" && finding.risk_level == "low"
        }));
        assert!(findings.iter().any(|finding| {
            finding.category == "stranded_value" && finding.subject_type == "address"
        }));
    }

    #[test]
    fn dormant_findings_include_last_activity_block_and_window_evidence() {
        let mut address = sample_address(&["value_detected", "dormant_candidate"]);
        address.last_activity_block = Some(4711);
        let findings =
            derive_inventory_risk_findings(&[address], &[], &[], &[sample_chain_profile(123)]);

        let finding = findings
            .iter()
            .find(|finding| finding.category == "dormant_wallet")
            .expect("dormant finding");
        assert!(
            finding
                .evidence
                .iter()
                .any(|value| { value == "Last observed on-chain activity block: 4711" })
        );
        assert!(
            finding
                .evidence
                .iter()
                .any(|value| { value == "Dormancy block window: 123 blocks (chain 1)" })
        );
    }

    #[test]
    fn empty_address_classifications_do_not_emit_address_findings() {
        let findings = derive_inventory_risk_findings(
            &[sample_address(&["watch_only", "empty_candidate"])],
            &[],
            &[],
            &[],
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn claim_candidates_emit_review_findings() {
        let holding = sample_claim_holding("airdrop");
        let findings = derive_inventory_risk_findings(&[], &[holding], &[], &[]);

        let finding = findings
            .iter()
            .find(|finding| finding.category == "claim_candidate")
            .unwrap();
        assert_eq!(finding.risk_level, "medium");
        assert_eq!(finding.subject_type, "claim_contract");
        assert_eq!(
            finding.subject,
            "0x2222222222222222222222222222222222222222"
        );
        assert!(finding.evidence.iter().any(|value| {
            value == "Execution blocked: requires protocol-specific claim adapter"
        }));
        assert!(
            finding
                .recommendation
                .contains("before signing any claim transaction")
        );
    }

    #[test]
    fn claim_candidates_use_risk_catalog_for_claim_contracts() {
        let holding = sample_claim_holding("reward");
        let catalog = vec![sample_catalog_entry(
            "0x2222222222222222222222222222222222222222",
            "critical",
        )];
        let findings = derive_inventory_risk_findings(&[], &[holding], &catalog, &[]);

        let finding = findings
            .iter()
            .find(|finding| finding.category == "claim_candidate")
            .unwrap();
        assert_eq!(finding.risk_level, "critical");
        assert!(
            finding
                .evidence
                .iter()
                .any(|value| value == "Risk catalog: Known claim contract (critical)")
        );
        assert!(finding.recommendation.contains("do not claim"));
    }

    #[test]
    fn claim_candidates_without_claim_contract_are_high_risk() {
        let mut holding = sample_claim_holding("airdrop");
        holding.protocol_address = None;
        let findings = derive_inventory_risk_findings(&[], &[holding], &[], &[]);

        let finding = findings
            .iter()
            .find(|finding| finding.category == "claim_candidate")
            .unwrap();
        assert_eq!(finding.risk_level, "high");
        assert_eq!(finding.subject, "unknown-claim-contract");
        assert!(
            finding
                .evidence
                .iter()
                .any(|value| { value == "Claim contract is missing from inventory holding" })
        );
    }
}
