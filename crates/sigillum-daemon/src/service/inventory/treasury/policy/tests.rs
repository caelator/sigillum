use super::*;

fn sample_policy() -> TreasuryPolicy {
    TreasuryPolicy {
        enabled: true,
        allowed_destinations: vec![TreasuryAllowedDestination {
            address: "0x9999999999999999999999999999999999999999".into(),
            label: Some("cold-treasury".into()),
        }],
        max_step_native_wei_hex: Some("0xde0b6b3a7640000".into()),
        max_plan_native_wei_hex: None,
        require_simulation: true,
        allow_raw_digest_signing: false,
        block_cross_party_linkage: false,
        allow_claim_execution: false,
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

#[test]
fn policy_allows_allowlisted_sweep_within_cap() {
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "sweep_native",
        Some("0x9999999999999999999999999999999999999999"),
        "native",
        "0xde0b6b3a7640000",
    );
    assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
}

#[test]
fn policy_allowlist_match_is_case_insensitive() {
    let destination = "0x9999999999999999999999999999999999999999".to_ascii_uppercase();
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "sweep_native",
        Some(destination.as_str()),
        "native",
        "0x1",
    );
    assert!(blockers.is_empty(), "unexpected blockers: {blockers:?}");
}

#[test]
fn policy_blocks_non_allowlisted_sweep_destination() {
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "sweep_erc20",
        Some("0x8888888888888888888888888888888888888888"),
        "erc20",
        "0x1",
    );
    assert_eq!(blockers, vec!["block_destination".to_string()]);
}

#[test]
fn policy_blocks_native_amount_above_step_cap() {
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "sweep_native",
        Some("0x9999999999999999999999999999999999999999"),
        "native",
        "0xde0b6b3a7640001",
    );
    assert_eq!(blockers, vec!["block_step_cap".to_string()]);
}

#[test]
fn policy_reports_destination_and_cap_violations_together() {
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "sweep_native",
        Some("0x8888888888888888888888888888888888888888"),
        "native",
        "0x1bc16d674ec80000",
    );
    assert_eq!(
        blockers,
        vec![
            "block_destination".to_string(),
            "block_step_cap".to_string(),
        ]
    );
}

#[test]
fn disabled_policy_blocks_nothing() {
    let mut policy = sample_policy();
    policy.enabled = false;
    let blockers = policy_blockers_for_step(
        &policy,
        "sweep_native",
        Some("0x8888888888888888888888888888888888888888"),
        "native",
        "0xffffffffffffffffffffffff",
    );
    assert!(blockers.is_empty());
}

#[test]
fn policy_ignores_non_sweep_actions_and_missing_destinations() {
    // Revokes and claims are not destination-routed value moves.
    let blockers = policy_blockers_for_step(
        &sample_policy(),
        "revoke_erc20_approval",
        Some("0x8888888888888888888888888888888888888888"),
        "approval",
        "0x1",
    );
    assert!(blockers.is_empty());

    // A sweep with no destination is already blocked by the planner.
    let blockers = policy_blockers_for_step(&sample_policy(), "sweep_erc20", None, "erc20", "0x1");
    assert!(blockers.is_empty());
}

#[test]
fn empty_allowlist_blocks_routed_sweeps_when_enabled() {
    let mut policy = sample_policy();
    policy.allowed_destinations.clear();
    let blockers = policy_blockers_for_step(
        &policy,
        "sweep_native",
        Some("0x9999999999999999999999999999999999999999"),
        "native",
        "0x1",
    );
    assert_eq!(blockers, vec!["block_destination".to_string()]);
}
