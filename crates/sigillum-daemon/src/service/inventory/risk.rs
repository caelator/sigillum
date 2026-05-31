use sigillum_api::{RiskFinding, WalletAssetHolding, WalletInventoryAddress};
use sigillum_core::decode_quantity_hex;

use super::support::quantity_hex_is_nonzero;

pub(super) fn derive_inventory_risk_findings(
    addresses: &[WalletInventoryAddress],
    holdings: &[WalletAssetHolding],
) -> Vec<RiskFinding> {
    let mut findings = Vec::new();
    for holding in holdings
        .iter()
        .filter(|holding| quantity_hex_is_nonzero(&holding.amount_hex))
    {
        if holding.asset_kind == "approval" {
            findings.push(approval_finding(holding));
            continue;
        }
        if holding.asset_kind != "native"
            && native_balance_for_holding(addresses, holding)
                .is_none_or(|balance| !quantity_hex_is_nonzero(balance))
        {
            findings.push(RiskFinding {
                id: stable_finding_id("stranded_gas", holding),
                category: "stranded_value".into(),
                risk_level: "medium".into(),
                status: "open".into(),
                wallet_family: holding.wallet_family.clone(),
                wallet_profile: holding.wallet_profile.clone(),
                provider_profile: holding.provider_profile.clone(),
                chain_id: holding.chain_id,
                address: holding.address.clone(),
                subject_type: holding.asset_kind.clone(),
                subject: holding
                    .asset_address
                    .clone()
                    .unwrap_or_else(|| "native".into()),
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

fn approval_finding(holding: &WalletAssetHolding) -> RiskFinding {
    let spender = holding
        .counterparty_address
        .clone()
        .unwrap_or_else(|| "unknown-spender".into());
    let risk_level = if is_very_large_approval(&holding.amount_hex) {
        "high"
    } else {
        "medium"
    };
    RiskFinding {
        id: stable_approval_finding_id(holding, &spender),
        category: "risky_approval".into(),
        risk_level: risk_level.into(),
        status: "open".into(),
        wallet_family: holding.wallet_family.clone(),
        wallet_profile: holding.wallet_profile.clone(),
        provider_profile: holding.provider_profile.clone(),
        chain_id: holding.chain_id,
        address: holding.address.clone(),
        subject_type: "approval".into(),
        subject: spender.clone(),
        source: "local-risk-engine".into(),
        recommendation: "Review the spender and revoke the allowance if it is no longer needed."
            .into(),
        evidence: vec![
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
        first_seen_at_unix: holding.first_seen_at_unix,
        last_checked_at_unix: holding.last_checked_at_unix,
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

fn is_very_large_approval(amount_hex: &str) -> bool {
    decode_quantity_hex(amount_hex)
        .map(|amount| amount[0] >= 0x80)
        .unwrap_or(false)
}
