use std::collections::BTreeMap;

use sigillum_api::{
    Counterparty, EthStealthDeposit, TreasuryReceiveAllocation, TreasuryReceiveSummary,
    WalletInventoryAddress,
};
use sigillum_core::decode_quantity_hex;

use crate::deposits::DepositState;
use crate::inventory::WalletInventoryState;
use crate::service::helpers::{add_u256, encode_quantity_hex};

use super::allocations::{RECEIVE_STATUS_ACTIVE, RECEIVE_STATUS_RETIRED, next_receive_index};
use super::overview::receive_summary;
use super::receiving::{
    RECEIVING_LINKAGE_WARNING, build_receiving_overview, hd_receiving_item, stealth_receiving_item,
};

#[test]
fn add_u256_carries_and_saturates() {
    let one = decode_quantity_hex("0x1").unwrap();
    let max_byte = decode_quantity_hex("0xff").unwrap();
    let sum = add_u256(&one, &max_byte);
    assert_eq!(encode_quantity_hex(&sum), "0x100");

    let max = [0xffu8; 32];
    let saturated = add_u256(&max, &one);
    assert_eq!(saturated, [0xffu8; 32]);
}

#[test]
fn encode_quantity_hex_trims_leading_zeroes() {
    assert_eq!(encode_quantity_hex(&[0u8; 32]), "0x0");
    let value = decode_quantity_hex("0xde0b6b3a7640000").unwrap();
    assert_eq!(encode_quantity_hex(&value), "0xde0b6b3a7640000");
}

#[test]
fn build_receiving_overview_groups_active_hd_and_stealth_deposits() {
    let party = Counterparty {
        id: "party_1".into(),
        name: "Acme Labs".into(),
        note: None,
        sweep_destination_address: None,
        created_at_unix: 1,
    };
    let named_address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let unresolved_address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let untagged_stealth_address = "0xcccccccccccccccccccccccccccccccccccccccc";
    let retired_address = "0xdddddddddddddddddddddddddddddddddddddddd";
    let tagged_stealth_address = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let state = WalletInventoryState {
        parties: vec![party],
        addresses: vec![
            receiving_inventory_address("named_a", named_address, "0x1"),
            receiving_inventory_address("named_b", &named_address.to_ascii_uppercase(), "0x2"),
        ],
        receive_allocations: vec![
            receiving_allocation(
                "alloc_named",
                named_address,
                RECEIVE_STATUS_ACTIVE,
                Some("party_1"),
                10,
            ),
            receiving_allocation(
                "alloc_unresolved",
                unresolved_address,
                RECEIVE_STATUS_ACTIVE,
                Some("missing_party"),
                11,
            ),
            receiving_allocation(
                "alloc_retired",
                retired_address,
                RECEIVE_STATUS_RETIRED,
                Some("party_1"),
                12,
            ),
        ],
        ..WalletInventoryState::default()
    };
    let deposits = DepositState {
        eth_stealth: vec![
            receiving_stealth_deposit("dep_1", untagged_stealth_address, Some("0x4"), None),
            receiving_stealth_deposit(
                "dep_2",
                tagged_stealth_address,
                Some("0x5"),
                Some("party_1"),
            ),
        ],
    };

    let overview = build_receiving_overview(&state, &deposits, 123);

    assert_eq!(overview.generated_at_unix, 123);
    assert!(!overview.include_retired);
    assert_eq!(overview.groups.len(), 2);

    let named = &overview.groups[0];
    assert_eq!(named.counterparty.as_ref().unwrap().id, "party_1");
    assert_eq!(named.item_count, 2);
    assert_eq!(named.native_total_wei_hex, "0x8");
    assert_eq!(named.items[0].source_type, "hd");
    assert_eq!(named.items[0].counterparty_id.as_deref(), Some("party_1"));
    assert!(named.items[0].balance_known);
    assert_eq!(
        named.items[0].balance_native_wei_hex.as_deref(),
        Some("0x3")
    );
    let tagged_stealth = named
        .items
        .iter()
        .find(|item| item.address == tagged_stealth_address)
        .expect("tagged stealth item");
    assert_eq!(tagged_stealth.source_type, "stealth");
    assert_eq!(tagged_stealth.counterparty_id.as_deref(), Some("party_1"));
    assert!(tagged_stealth.balance_known);
    assert_eq!(
        tagged_stealth.balance_native_wei_hex.as_deref(),
        Some("0x5")
    );

    let unassigned = &overview.groups[1];
    assert!(unassigned.counterparty.is_none());
    assert_eq!(unassigned.item_count, 2);
    assert_eq!(unassigned.native_total_wei_hex, "0x4");

    let unresolved = unassigned
        .items
        .iter()
        .find(|item| item.address == unresolved_address)
        .expect("unresolved HD item");
    assert_eq!(unresolved.source_type, "hd");
    assert_eq!(unresolved.counterparty_id.as_deref(), Some("missing_party"));
    assert!(!unresolved.balance_known);
    assert!(unresolved.balance_native_wei_hex.is_none());

    let stealth = unassigned
        .items
        .iter()
        .find(|item| item.address == untagged_stealth_address)
        .expect("untagged stealth item");
    assert_eq!(stealth.source_type, "stealth");
    assert!(stealth.balance_known);
    assert_eq!(stealth.balance_native_wei_hex.as_deref(), Some("0x4"));
    assert_eq!(stealth.label.as_deref(), Some("stealth note"));
    assert!(stealth.counterparty_id.is_none());

    assert!(
        !overview
            .groups
            .iter()
            .flat_map(|group| group.items.iter())
            .any(|item| item.address == retired_address)
    );
    assert_eq!(overview.totals.item_count, 4);
    assert_eq!(overview.totals.hd_count, 2);
    assert_eq!(overview.totals.stealth_count, 2);
    assert_eq!(overview.totals.native_total_wei_hex, "0xc");
    assert_eq!(overview.coverage.addresses_total, 4);
    assert_eq!(overview.coverage.addresses_with_known_balance, 3);
}

#[test]
fn build_receiving_overview_warns_for_cross_party_stealth_sweep_destinations() {
    let destination = "0x9999999999999999999999999999999999999999";
    let parties = vec![
        Counterparty {
            id: "party_1".into(),
            name: "Acme Labs".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 1,
        },
        Counterparty {
            id: "party_2".into(),
            name: "Beta Labs".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 2,
        },
    ];
    let mut party_one_deposit = receiving_stealth_deposit(
        "dep_1",
        "0x1111111111111111111111111111111111111111",
        Some("0x1"),
        Some("party_1"),
    );
    party_one_deposit.sweep_destination_address = Some(destination.into());
    let mut party_two_deposit = receiving_stealth_deposit(
        "dep_2",
        "0x2222222222222222222222222222222222222222",
        Some("0x2"),
        Some("party_2"),
    );
    party_two_deposit.sweep_destination_address = Some(destination.into());
    let state = WalletInventoryState {
        parties,
        ..WalletInventoryState::default()
    };
    let deposits = DepositState {
        eth_stealth: vec![party_one_deposit, party_two_deposit],
    };

    let overview = build_receiving_overview(&state, &deposits, 123);
    let warnings: Vec<_> = overview
        .groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|item| item.linkage_warning.as_deref())
        .collect();
    assert_eq!(
        warnings,
        vec![
            Some(RECEIVING_LINKAGE_WARNING),
            Some(RECEIVING_LINKAGE_WARNING)
        ]
    );

    let same_party = Counterparty {
        id: "party_same".into(),
        name: "Same Party".into(),
        note: None,
        sweep_destination_address: None,
        created_at_unix: 1,
    };
    let mut first_same_party_deposit = receiving_stealth_deposit(
        "dep_3",
        "0x3333333333333333333333333333333333333333",
        Some("0x3"),
        Some("party_same"),
    );
    first_same_party_deposit.sweep_destination_address = Some(destination.into());
    let mut second_same_party_deposit = receiving_stealth_deposit(
        "dep_4",
        "0x4444444444444444444444444444444444444444",
        Some("0x4"),
        Some("party_same"),
    );
    second_same_party_deposit.sweep_destination_address = Some(destination.into());
    let same_party_overview = build_receiving_overview(
        &WalletInventoryState {
            parties: vec![same_party],
            ..WalletInventoryState::default()
        },
        &DepositState {
            eth_stealth: vec![first_same_party_deposit, second_same_party_deposit],
        },
        123,
    );

    assert!(same_party_overview.groups.iter().all(|group| {
        group
            .items
            .iter()
            .all(|item| item.linkage_warning.is_none())
    }));
}

fn sample_allocation(
    wallet_profile: &str,
    address_index: u32,
    status: &str,
    purpose: &str,
) -> TreasuryReceiveAllocation {
    TreasuryReceiveAllocation {
        id: format!("alloc_{wallet_profile}_{address_index}"),
        wallet_family: "eth-seed".into(),
        wallet_profile: wallet_profile.into(),
        chain_id: 1,
        chain_id_assumed: false,
        address: "0x1111111111111111111111111111111111111111".into(),
        derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
        address_index,
        purpose: purpose.into(),
        label: None,
        status: status.into(),
        created_at_unix: 1,
        retired_at_unix: None,
        counterparty_id: None,
    }
}

fn sample_inventory_address(
    wallet_family: &str,
    wallet_profile: &str,
    address_index: u32,
) -> WalletInventoryAddress {
    WalletInventoryAddress {
        id: format!("addr_{wallet_profile}_{address_index}"),
        wallet_family: wallet_family.into(),
        wallet_profile: wallet_profile.into(),
        provider_profile: "mainnet".into(),
        chain_id: 1,
        address: "0x2222222222222222222222222222222222222222".into(),
        derivation_path: format!("m/44'/60'/0'/0/{address_index}"),
        derivation_pattern: Some("project".into()),
        account_index: Some(0),
        address_index,
        activity_state: "funded".into(),
        native_balance_wei_hex: "0x1".into(),
        transaction_count: 0,
        last_activity_block: None,
        classifications: Vec::new(),
        source: "local-rpc".into(),
        first_seen_at_unix: 1,
        last_checked_at_unix: 2,
    }
}

fn receiving_allocation(
    id: &str,
    address: &str,
    status: &str,
    counterparty_id: Option<&str>,
    created_at_unix: u64,
) -> TreasuryReceiveAllocation {
    TreasuryReceiveAllocation {
        id: id.into(),
        wallet_family: "eth-xpub".into(),
        wallet_profile: "mainnet-xpub".into(),
        chain_id: 1,
        chain_id_assumed: false,
        address: address.into(),
        derivation_path: "m/44'/60'/0'/0/0".into(),
        address_index: 0,
        purpose: "invoice".into(),
        label: Some(format!("label-{id}")),
        status: status.into(),
        created_at_unix,
        retired_at_unix: (status == RECEIVE_STATUS_RETIRED).then_some(created_at_unix + 1),
        counterparty_id: counterparty_id.map(str::to_string),
    }
}

fn receiving_inventory_address(id: &str, address: &str, balance: &str) -> WalletInventoryAddress {
    WalletInventoryAddress {
        id: id.into(),
        wallet_family: "eth-xpub".into(),
        wallet_profile: "mainnet-xpub".into(),
        provider_profile: "mainnet".into(),
        chain_id: 1,
        address: address.into(),
        derivation_path: "m/44'/60'/0'/0/0".into(),
        derivation_pattern: Some("project".into()),
        account_index: Some(0),
        address_index: 0,
        activity_state: "funded".into(),
        native_balance_wei_hex: balance.into(),
        transaction_count: 0,
        last_activity_block: None,
        classifications: Vec::new(),
        source: "persisted-test".into(),
        first_seen_at_unix: 1,
        last_checked_at_unix: 2,
    }
}

fn receiving_stealth_deposit(
    id: &str,
    address: &str,
    balance: Option<&str>,
    counterparty_id: Option<&str>,
) -> EthStealthDeposit {
    EthStealthDeposit {
        id: id.into(),
        status: "detected".into(),
        asset_kind: "native".into(),
        wallet_profile: "mainnet-xpub".into(),
        chain_id: 1,
        chain_id_assumed: false,
        wallet_compartment_id: 0,
        provider_compartment_id: 0,
        wallet: "mainnet-xpub".into(),
        short_name: "eth".into(),
        stealth_meta_address: "st:eth:example".into(),
        stealth_address: address.into(),
        ephemeral_public_key_hex: "0x02".into(),
        view_tag_hex: "0xaa".into(),
        announcement: None,
        token_address: None,
        expected_amount_hex: None,
        observed_amount_hex: None,
        observed_native_balance_wei_hex: balance.map(str::to_string),
        auto_queue_sweep: false,
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        queue_job_id: None,
        queue_job_state: None,
        note: Some("stealth note".into()),
        created_at_unix: 20,
        updated_at_unix: 21,
        last_checked_at_unix: None,
        broadcast_transaction_hash_hex: None,
        counterparty_id: counterparty_id.map(str::to_string),
    }
}

#[test]
fn next_receive_index_starts_at_zero_when_nothing_is_known() {
    assert_eq!(next_receive_index(&[], &[], "eth-seed", "seed-main"), 0);
}

#[test]
fn next_receive_index_advances_past_allocations_only() {
    let allocations = vec![
        sample_allocation("seed-main", 0, "retired", "acme"),
        sample_allocation("seed-main", 4, "active", "beta"),
    ];
    assert_eq!(
        next_receive_index(&allocations, &[], "eth-seed", "seed-main"),
        5
    );
}

#[test]
fn next_receive_index_advances_past_inventory_only() {
    let addresses = vec![
        sample_inventory_address("eth-seed", "seed-main", 2),
        sample_inventory_address("eth-seed", "seed-main", 7),
    ];
    assert_eq!(
        next_receive_index(&[], &addresses, "eth-seed", "seed-main"),
        8
    );
}

#[test]
fn next_receive_index_takes_the_max_of_both_sources() {
    let allocations = vec![sample_allocation("seed-main", 9, "active", "acme")];
    let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 3)];
    assert_eq!(
        next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
        10
    );

    let allocations = vec![sample_allocation("seed-main", 1, "active", "acme")];
    let addresses = vec![sample_inventory_address("eth-seed", "seed-main", 6)];
    assert_eq!(
        next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
        7
    );
}

#[test]
fn next_receive_index_ignores_other_profiles_and_families() {
    let allocations = vec![sample_allocation("seed-other", 40, "active", "acme")];
    let addresses = vec![
        sample_inventory_address("eth-seed", "seed-other", 50),
        // Same profile name under a different family does not count for
        // the inventory source.
        sample_inventory_address("eth-xpub", "seed-main", 60),
    ];
    assert_eq!(
        next_receive_index(&allocations, &addresses, "eth-seed", "seed-main"),
        0
    );
}

#[test]
fn receive_summary_counts_active_retired_and_distinct_purposes() {
    let allocations = vec![
        sample_allocation("seed-main", 0, "retired", "acme"),
        sample_allocation("seed-main", 1, "active", "acme"),
        sample_allocation("seed-main", 2, "active", "acme"),
        sample_allocation("seed-main", 3, "active", "beta"),
    ];
    let summary = receive_summary(&allocations);
    assert_eq!(summary.active_allocations, 3);
    assert_eq!(summary.retired_allocations, 1);
    // Retired purposes do not count; duplicates collapse.
    assert_eq!(summary.purposes, 2);

    assert_eq!(receive_summary(&[]), TreasuryReceiveSummary::default());
}

#[test]
fn hd_receiving_item_uses_allocation_chain_id_and_assumption_marker() {
    let mut allocation = receiving_allocation(
        "alloc-base",
        "0x1111111111111111111111111111111111111111",
        RECEIVE_STATUS_ACTIVE,
        None,
        1,
    );
    allocation.chain_id = 8453;
    allocation.chain_id_assumed = true;

    let item = hd_receiving_item(&allocation, &BTreeMap::new());

    assert_eq!(item.chain_id, 8453);
    assert!(item.chain_id_assumed);
}

#[test]
fn stealth_receiving_item_uses_deposit_chain_id_and_assumption_marker() {
    let mut deposit = receiving_stealth_deposit(
        "dep-base",
        "0x2222222222222222222222222222222222222222",
        Some("0x1"),
        None,
    );
    deposit.chain_id = 8453;
    deposit.chain_id_assumed = true;

    let item = stealth_receiving_item(&deposit);

    assert_eq!(item.chain_id, 8453);
    assert!(item.chain_id_assumed);
}
