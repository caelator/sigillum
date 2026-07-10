//! W5 Merkle claim execution enablement gate.
//!
//! Claim plan steps are generated fail-closed with `claim_execution_disabled`.
//! This module is the single place that can remove that blocker, and only for a
//! structurally complete `merkle-distributor-v1` claim step after every
//! execution gate is satisfied. Execution-time rule for W7.3: a claim step that
//! reverts on-chain must transition to operator_action_required and must never
//! be auto-retried - a reverted MerkleDistributor claim may have consumed or
//! partially consumed its proof/index state, so retry safety cannot be assumed.
//! This module only decides enablement (blocker present/absent); queue execution
//! is W7.

use sigillum_api::{
    ConsolidationPlanStep, RiskCatalogEntry, TreasuryPolicy, WalletPlanStepAction,
    WalletPlanStepStatus, WalletSimulationStatus,
};

use super::claim_discovery::CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1;

const CLAIM_EXECUTION_DISABLED_BLOCKER: &str = "claim_execution_disabled";
pub(super) const CLAIM_EXECUTION_REVIEW_NOTE: &str = "claim_execution_reviewed";

pub(super) fn claim_contract_review_passed(
    risk_catalog: &[RiskCatalogEntry],
    claim_contract: &str,
) -> bool {
    risk_catalog
        .iter()
        .find(|entry| entry.address.eq_ignore_ascii_case(claim_contract))
        .map(|entry| {
            entry.risk_level.eq_ignore_ascii_case("trusted")
                || entry
                    .notes
                    .iter()
                    .any(|note| note == CLAIM_EXECUTION_REVIEW_NOTE)
        })
        .unwrap_or(false)
}

pub(super) fn claim_execution_gate_satisfied(
    policy: Option<&TreasuryPolicy>,
    risk_catalog: &[RiskCatalogEntry],
    step: &ConsolidationPlanStep,
) -> bool {
    let Some(policy) = policy else {
        return false;
    };
    if !policy.enabled || !policy.allow_claim_execution {
        return false;
    }
    if step.claim_adapter.as_deref() != Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1) {
        return false;
    }
    if step.simulation_status != WalletSimulationStatus::Passed {
        return false;
    }
    let Some(claim_contract) = step.protocol_address.as_deref() else {
        return false;
    };
    claim_contract_review_passed(risk_catalog, claim_contract) && step.approved
}

pub(super) fn refresh_claim_execution_blocker(
    policy: Option<&TreasuryPolicy>,
    risk_catalog: &[RiskCatalogEntry],
    step: &mut ConsolidationPlanStep,
) {
    if !is_structurally_complete_merkle_claim(step) {
        return;
    }

    if claim_execution_gate_satisfied(policy, risk_catalog, step) {
        step.blockers
            .retain(|blocker| blocker != CLAIM_EXECUTION_DISABLED_BLOCKER);
        if step.blockers.is_empty() {
            step.status = if step.approved {
                WalletPlanStepStatus::Approved
            } else {
                WalletPlanStepStatus::ReviewRequired
            };
        }
        if step.risk_level == "blocked" {
            step.risk_level = "low".into();
        }
    } else {
        if !step
            .blockers
            .iter()
            .any(|blocker| blocker == CLAIM_EXECUTION_DISABLED_BLOCKER)
        {
            step.blockers.push(CLAIM_EXECUTION_DISABLED_BLOCKER.into());
        }
        step.status = WalletPlanStepStatus::Blocked;
        step.risk_level = "blocked".into();
    }
}

fn is_structurally_complete_merkle_claim(step: &ConsolidationPlanStep) -> bool {
    step.action == WalletPlanStepAction::ClaimReward
        && step.claim_adapter.as_deref() == Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1)
        && step.protocol_address.is_some()
        && step.claim_index_hex.is_some()
        && !step.claim_proof.is_empty()
}

#[cfg(test)]
mod tests {
    use sigillum_api::{
        TreasuryAllowedDestination, WalletAssetKind, WalletSignerStatus, WalletSimulationStatus,
    };

    use super::*;

    fn baseline_policy() -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".into(),
                label: Some("cold-treasury".into()),
            }],
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: false,
            allow_claim_execution: true,
            allow_gas_topups: false,
            max_gas_topup_wei_hex: None,
            allow_plan_execution: false,
            allow_sweep_execution: false,
            allow_revoke_execution: false,
            allow_exit_execution: false,
            execution_paused: false,
            max_fee_per_gas_cap_hex: None,
            simulation_freshness_secs: 900,
            hot_floor_wei_hex: "0xde0b6b3a7640000".into(),
            hot_target_wei_hex: "0xde0b6b3a7640000".into(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    fn catalog_entry(risk_level: &str, notes: Vec<String>) -> RiskCatalogEntry {
        RiskCatalogEntry {
            address: "0x1111111111111111111111111111111111111111".into(),
            label: "Claim contract".into(),
            risk_level: risk_level.into(),
            source: "operator".into(),
            notes,
            created_at_unix: 1,
            updated_at_unix: 2,
        }
    }

    fn trusted_catalog() -> Vec<RiskCatalogEntry> {
        vec![catalog_entry("trusted", Vec::new())]
    }

    fn baseline_claim_step() -> ConsolidationPlanStep {
        ConsolidationPlanStep {
            id: "step_claim".into(),
            sequence: 0,
            depends_on: Vec::new(),
            action: WalletPlanStepAction::ClaimReward,
            status: WalletPlanStepStatus::Blocked,
            wallet_family: "eth-seed".into(),
            wallet_profile: "seed-main".into(),
            provider_profile: "mainnet".into(),
            chain_id: 1,
            address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            derivation_path: "m/44'/60'/0'/0/0".into(),
            asset_kind: WalletAssetKind::Airdrop,
            asset_address: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            token_id_hex: None,
            counterparty_address: None,
            protocol_address: Some("0x1111111111111111111111111111111111111111".into()),
            claim_adapter: Some(CLAIM_ADAPTER_MERKLE_DISTRIBUTOR_V1.into()),
            claim_index_hex: Some("0x7".into()),
            claim_proof: vec![
                format!("0x{}", "11".repeat(32)),
                format!("0x{}", "22".repeat(32)),
            ],
            exit_token0_address: None,
            exit_token1_address: None,
            exit_amount0_min_hex: None,
            exit_amount1_min_hex: None,
            exit_deadline_unix: None,
            amount_hex: "0x1".into(),
            destination_address: None,
            signer_status: WalletSignerStatus::Available,
            simulation_status: WalletSimulationStatus::Passed,
            simulation_evidence: vec!["prepared_call=claim.merkle_distributor_v1".into()],
            risk_level: "blocked".into(),
            blockers: vec![CLAIM_EXECUTION_DISABLED_BLOCKER.into()],
            linkage_warnings: Vec::new(),
            auto_eligible: false,
            approved: true,
            queued_job_id: None,
        }
    }

    fn assert_claim_refresh_blocked(
        policy: Option<&TreasuryPolicy>,
        risk_catalog: &[RiskCatalogEntry],
        step: &mut ConsolidationPlanStep,
    ) {
        assert!(!claim_execution_gate_satisfied(policy, risk_catalog, step));
        refresh_claim_execution_blocker(policy, risk_catalog, step);
        assert_eq!(
            step.blockers,
            vec![CLAIM_EXECUTION_DISABLED_BLOCKER.to_string()]
        );
        assert_eq!(step.status, WalletPlanStepStatus::Blocked);
        assert_eq!(step.risk_level, "blocked");
    }

    #[test]
    fn claim_gate_satisfied_refresh_removes_claim_blocker() {
        let policy = baseline_policy();
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();

        assert!(claim_execution_gate_satisfied(
            Some(&policy),
            &catalog,
            &step
        ));
        refresh_claim_execution_blocker(Some(&policy), &catalog, &mut step);

        assert!(step.blockers.is_empty());
        assert_eq!(step.status, WalletPlanStepStatus::Approved);
        assert_eq!(step.risk_level, "low");
    }

    #[test]
    fn claim_policy_none_keeps_claim_blocker() {
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();

        assert_claim_refresh_blocked(None, &catalog, &mut step);
    }

    #[test]
    fn claim_policy_disabled_keeps_claim_blocker() {
        let mut policy = baseline_policy();
        policy.enabled = false;
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();

        assert_claim_refresh_blocked(Some(&policy), &catalog, &mut step);
    }

    #[test]
    fn claim_policy_without_claim_optin_keeps_claim_blocker() {
        let mut policy = baseline_policy();
        policy.allow_claim_execution = false;
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();

        assert_claim_refresh_blocked(Some(&policy), &catalog, &mut step);
    }

    #[test]
    fn claim_other_adapter_gate_fails_and_refresh_leaves_step_unchanged() {
        let policy = baseline_policy();
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();
        step.claim_adapter = Some("other-adapter".into());
        let before = step.clone();

        assert!(!claim_execution_gate_satisfied(
            Some(&policy),
            &catalog,
            &step
        ));
        refresh_claim_execution_blocker(Some(&policy), &catalog, &mut step);

        assert_eq!(step, before);
        assert_eq!(
            step.blockers,
            vec![CLAIM_EXECUTION_DISABLED_BLOCKER.to_string()]
        );
    }

    #[test]
    fn claim_simulation_required_keeps_claim_blocker() {
        let policy = baseline_policy();
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();
        step.simulation_status = WalletSimulationStatus::Required;

        assert_claim_refresh_blocked(Some(&policy), &catalog, &mut step);
    }

    #[test]
    fn claim_without_risk_catalog_entry_keeps_claim_blocker() {
        let policy = baseline_policy();
        let mut step = baseline_claim_step();

        assert_claim_refresh_blocked(Some(&policy), &[], &mut step);
    }

    #[test]
    fn claim_untrusted_catalog_entry_without_note_keeps_claim_blocker() {
        let policy = baseline_policy();
        let catalog = vec![catalog_entry("low", Vec::new())];
        let mut step = baseline_claim_step();

        assert_claim_refresh_blocked(Some(&policy), &catalog, &mut step);
    }

    #[test]
    fn claim_not_approved_keeps_claim_blocker() {
        let policy = baseline_policy();
        let catalog = trusted_catalog();
        let mut step = baseline_claim_step();
        step.approved = false;

        assert_claim_refresh_blocked(Some(&policy), &catalog, &mut step);
    }

    #[test]
    fn claim_review_note_allows_untrusted_catalog_entry() {
        let policy = baseline_policy();
        let catalog = vec![catalog_entry(
            "low",
            vec![CLAIM_EXECUTION_REVIEW_NOTE.to_string()],
        )];
        let mut step = baseline_claim_step();

        assert!(claim_execution_gate_satisfied(
            Some(&policy),
            &catalog,
            &step
        ));
        refresh_claim_execution_blocker(Some(&policy), &catalog, &mut step);

        assert!(step.blockers.is_empty());
        assert_eq!(step.status, WalletPlanStepStatus::Approved);
    }
}
