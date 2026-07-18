//! Deposit tracking and management for stealth addresses.
//!
//! Manages creation, tracking, and sweeping of Ethereum stealth deposits
//! with auto-queueing and balance refresh capabilities.
//!
//! ## Deposit lifecycle
//!
//! 1. **Create** — generates a fresh stealth address from the wallet profile,
//!    persists a `pending` deposit record, and optionally configures auto-sweep.
//! 2. **Refresh** — queries on-chain balances, updates `observed_amount_hex`,
//!    transitions status, and auto-enqueues sweep jobs when thresholds are met.
//! 3. **Enqueue sweep** — places a native or ERC-20 sweep job on the queue.
//! 4. **Delete** — removes the deposit record (does not affect on-chain state).
//!
//! Both native and ERC-20 deposit creation share core logic extracted into
//! [`DepositBlueprint`] and [`SigillumService::persist_new_deposit`] to
//! avoid structural duplication while keeping the public API surface clean.
//!
//! ## Watch-only detection
//!
//! Announcement scanning ([`SigillumService::scan_eth_stealth_announcements`])
//! detects deposits from the viewing private key + spending PUBLIC key alone
//! (the EIP-5564 `checkStealthAddress` key material), via
//! `derive_watch_only_sigillum_ethereum_stealth_wallet`: the spending private
//! key never enters the scan path. Detection still requires the wallet
//! compartment unlocked — the viewing key derives from its master key — but
//! no spending secret materializes while scanning. The spending private key
//! is only derived later, at sweep-signing time.

use std::collections::{BTreeSet, HashMap};

use sha3::{Digest, Keccak256};
use sigillum_api::{
    Counterparty, EthStealthAnnouncementPayload, EthStealthAnnouncementScanCursor,
    EthStealthAnnouncementScanRequest, EthStealthAnnouncementScanResponse, EthStealthDeposit,
    EthStealthDepositCreateErc20Request, EthStealthDepositCreateNativeRequest,
    EthStealthDepositDeleteRequest, EthStealthDepositEnqueueSweepRequest,
    EthStealthDepositEnqueueSweepResponse, EthStealthDepositListResponse,
    EthStealthDepositMutationResponse, EthStealthDepositRefreshRequest,
    EthStealthDepositRefreshResponse, EthStealthGenerateRequest, EthStealthWalletProfile,
    EvmProviderProfile, QueueEnqueueResponse, QueueJob, QueueJobPayload,
    ReceivingDepositTagRequest, RiskFinding,
};
use sigillum_core::{
    ERC5564_ANNOUNCE_FUNCTION, ERC5564_ANNOUNCER_ADDRESS, ERC5564_METADATA_ERC20_TRANSFER_SELECTOR,
    ETHEREUM_STEALTH_SCHEME_ID, Erc5564MetadataHints, EthereumStealthError, StealthHashConvention,
    VaultLifecycle, check_ethereum_stealth_address_any_watch_only, decode_erc5564_metadata_hints,
    decode_quantity_hex, derive_watch_only_sigillum_ethereum_stealth_wallet,
    encode_erc5564_announce_calldata, encode_erc5564_metadata_erc20_transfer,
    encode_erc5564_metadata_native,
};

use crate::audit_log::{AuditEventSpec, AuditQueueJobKind};
use crate::inventory::WalletInventoryState;

use super::helpers::{
    compare_u256, is_zero_u256, map_wallet_error, multiply_u256_u64, now_unix, random_id,
};
use super::transaction_policy::{TransactionPolicyCheck, TransactionPolicyKind};
use super::{ServiceError, ServiceResult, SigillumService};

const DEFAULT_ANNOUNCEMENT_SCAN_LIMIT: usize = 1_000;
const MAX_ANNOUNCEMENT_SCAN_LIMIT: usize = 10_000;
const ERC5564_DISCOVERY_SOURCE: &str = "erc5564-announcement";

// ── Deposit Blueprint & Plans ──────────────────────────────────────────────

/// Intermediate representation capturing all parameters needed to construct a
/// new [`EthStealthDeposit`], shared between native and ERC-20 creation paths.
struct DepositBlueprint {
    wallet_profile: String,
    wallet_compartment_id: usize,
    provider_compartment_id: usize,
    wallet: String,
    short_name: String,
    asset_kind: String,
    token_address: Option<String>,
    expected_amount_hex: Option<String>,
    auto_queue_sweep: bool,
    sweep_destination_address: Option<String>,
    min_sweep_amount_hex: Option<String>,
    note: Option<String>,
    /// Native gas the payer is asked to attach for this deposit's sweep
    /// (`request_gas` at creation; already resolved to a concrete amount —
    /// explicit `gas_amount_wei_hex` or the provider's static sweep gas
    /// estimate). When set, the announcement metadata follows the EIP-5564
    /// SHOULD layouts so standards-aware payer wallets learn the asset info.
    requested_gas_wei_hex: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sigillum_api::TreasuryPolicy;

    fn abi_word(value: usize) -> String {
        format!("{value:064x}")
    }

    fn scan_cursor(
        wallet: &str,
        provider: &str,
        block: u64,
        updated: u64,
    ) -> EthStealthAnnouncementScanCursor {
        EthStealthAnnouncementScanCursor {
            wallet_profile: wallet.into(),
            provider_profile: provider.into(),
            chain_id: 1,
            last_scanned_block: block,
            updated_at_unix: updated,
        }
    }

    #[test]
    fn announcement_cursor_upsert_is_monotonic_unless_reset() {
        let mut cursors = vec![scan_cursor("w", "p", 100, 10)];

        // A lower rescan never drags the cursor backward.
        upsert_announcement_scan_cursor(&mut cursors, scan_cursor("w", "p", 40, 20), false);
        assert_eq!(cursors[0].last_scanned_block, 100);
        assert_eq!(cursors[0].updated_at_unix, 20);

        // Forward progress lands.
        upsert_announcement_scan_cursor(&mut cursors, scan_cursor("w", "p", 160, 30), false);
        assert_eq!(cursors[0].last_scanned_block, 160);

        // Reset re-anchors, even backward.
        upsert_announcement_scan_cursor(&mut cursors, scan_cursor("w", "p", 50, 40), true);
        assert_eq!(cursors[0].last_scanned_block, 50);

        // A different (wallet, provider) pair gets its own entry.
        upsert_announcement_scan_cursor(&mut cursors, scan_cursor("w", "q", 7, 50), false);
        assert_eq!(cursors.len(), 2);
        assert_eq!(
            latest_announcement_scan_cursor(&cursors, "w", "p").map(|c| c.last_scanned_block),
            Some(50)
        );
        assert_eq!(
            latest_announcement_scan_cursor(&cursors, "w", "q").map(|c| c.last_scanned_block),
            Some(7)
        );
        assert!(latest_announcement_scan_cursor(&cursors, "w", "unknown").is_none());
    }

    #[test]
    fn max_log_block_tracks_the_highest_processed_log() {
        assert_eq!(max_log_block(None, None), None);
        assert_eq!(max_log_block(None, Some("0x20")), Some(32));
        assert_eq!(max_log_block(Some(32), Some("0x10")), Some(32));
        assert_eq!(max_log_block(Some(32), Some("0x40")), Some(64));
        // Named tags and junk never move the cursor.
        assert_eq!(max_log_block(Some(32), Some("latest")), Some(32));
        assert_eq!(parse_block_quantity("latest"), None);
        assert_eq!(parse_block_quantity("0X0A"), Some(10));
        assert_eq!(encode_block_quantity(31), "0x1f");
    }

    fn abi_dynamic_bytes(bytes: &[u8]) -> String {
        let mut out = abi_word(bytes.len());
        let mut padded = bytes.to_vec();
        let padding = (32 - (padded.len() % 32)) % 32;
        padded.resize(padded.len() + padding, 0);
        out.push_str(&hex::encode(padded));
        out
    }

    fn padded_address_topic(address: &str) -> String {
        let raw = address.trim_start_matches("0x");
        format!("0x{raw:0>64}")
    }

    fn test_wallet(
        name: &str,
        default_destination_address: Option<&str>,
    ) -> EthStealthWalletProfile {
        EthStealthWalletProfile {
            name: name.into(),
            wallet: "wallet".into(),
            short_name: "eth".into(),
            provider_profile: "mainnet".into(),
            compartment_id: 0,
            chain_id: Some(1),
            default_destination_address: default_destination_address.map(str::to_string),
            execution_enabled: true,
        }
    }

    fn test_counterparty(id: &str, destination: Option<&str>) -> Counterparty {
        Counterparty {
            id: id.into(),
            name: format!("Party {id}"),
            note: None,
            sweep_destination_address: destination.map(str::to_string),
            created_at_unix: 1,
        }
    }

    fn test_policy(block_cross_party_linkage: bool) -> TreasuryPolicy {
        TreasuryPolicy {
            enabled: true,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage,
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
            updated_at_unix: 1,
        }
    }

    fn test_inventory(
        parties: Vec<Counterparty>,
        block_cross_party_linkage: Option<bool>,
    ) -> WalletInventoryState {
        WalletInventoryState {
            parties,
            treasury_policy: block_cross_party_linkage.map(test_policy),
            ..WalletInventoryState::default()
        }
    }

    fn test_deposit(
        id: &str,
        wallet_profile: &str,
        stealth_address: &str,
        counterparty_id: Option<&str>,
        sweep_destination_address: Option<&str>,
    ) -> EthStealthDeposit {
        EthStealthDeposit {
            id: id.into(),
            status: "funded".into(),
            asset_kind: "native".into(),
            wallet_profile: wallet_profile.into(),
            chain_id: 1,
            chain_id_assumed: false,
            wallet_compartment_id: 0,
            provider_compartment_id: 0,
            wallet: wallet_profile.into(),
            short_name: "eth".into(),
            stealth_meta_address: format!("st:eth:{id}"),
            stealth_address: stealth_address.into(),
            ephemeral_public_key_hex: "0x02".into(),
            view_tag_hex: "0xaa".into(),
            stealth_hash_convention: StealthHashConvention::STANDARD,
            announcement: None,
            token_address: None,
            expected_amount_hex: None,
            observed_amount_hex: Some("0x1".into()),
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: false,
            sweep_destination_address: sweep_destination_address.map(str::to_string),
            min_sweep_amount_hex: None,
            queue_job_id: None,
            queue_job_state: None,
            note: None,
            created_at_unix: 1,
            updated_at_unix: 1,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
            counterparty_id: counterparty_id.map(str::to_string),
            requested_gas_wei_hex: None,
            gas_topup_job_id: None,
            gas_topup_job_state: None,
        }
    }

    #[test]
    fn decodes_standard_erc5564_announcement_log() {
        let stealth_address = "0x1111111111111111111111111111111111111111";
        let caller_address = "0x2222222222222222222222222222222222222222";
        let ephemeral_public_key = vec![0x03; 33];
        let metadata = vec![0x7f, 0xaa, 0xbb];
        let first_tail = abi_dynamic_bytes(&ephemeral_public_key);
        let second_offset = 64 + first_tail.len() / 2;
        let data = format!(
            "0x{}{}{}{}",
            abi_word(64),
            abi_word(second_offset),
            first_tail,
            abi_dynamic_bytes(&metadata),
        );
        let log = super::super::evm::EvmLogEntry {
            address: ERC5564_ANNOUNCER_ADDRESS.into(),
            topics: vec![
                erc5564_announcement_topic(),
                padded_u64_topic(ETHEREUM_STEALTH_SCHEME_ID),
                padded_address_topic(stealth_address),
                padded_address_topic(caller_address),
            ],
            data,
            block_number: Some("0xabc".into()),
            transaction_hash: Some(format!("0x{}", "33".repeat(32))),
            log_index: Some("0x1".into()),
        };

        let event = decode_erc5564_announcement_log(&log).unwrap();

        assert_eq!(event.stealth_address, stealth_address);
        assert_eq!(event.caller_address.as_deref(), Some(caller_address));
        assert_eq!(
            event.ephemeral_public_key_hex,
            hex::encode(ephemeral_public_key)
        );
        assert_eq!(event.metadata_hex, hex::encode(metadata));
        assert_eq!(event.view_tag_hex, "7f");
        assert_eq!(event.block_number.as_deref(), Some("0xabc"));
    }

    #[test]
    fn normalizes_log_block_tags_and_quantities() {
        assert_eq!(
            normalize_log_block_tag(" latest ", "from_block").unwrap(),
            "latest"
        );
        assert_eq!(
            normalize_log_block_tag("0X000abc", "from_block").unwrap(),
            "0x000abc"
        );
        assert!(normalize_log_block_tag("123", "from_block").is_err());
    }

    #[test]
    fn party_destination_used_when_deposit_has_none() {
        let party_destination = "0x1111111111111111111111111111111111111111";
        let wallet_destination = "0x2222222222222222222222222222222222222222";
        let deposit = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("party_1"),
            None,
        );
        let wallet = test_wallet("wallet_1", Some(wallet_destination));
        let inventory = test_inventory(
            vec![test_counterparty("party_1", Some(party_destination))],
            None,
        );

        let destination = resolve_stealth_sweep_destination(&deposit, &wallet, &inventory).unwrap();

        assert_eq!(destination.as_deref(), Some(party_destination));
    }

    #[test]
    fn deposit_destination_takes_precedence_over_party() {
        let deposit_destination = "0x3333333333333333333333333333333333333333";
        let party_destination = "0x4444444444444444444444444444444444444444";
        let deposit = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("party_1"),
            Some(deposit_destination),
        );
        let wallet = test_wallet(
            "wallet_1",
            Some("0x5555555555555555555555555555555555555555"),
        );
        let inventory = test_inventory(
            vec![test_counterparty("party_1", Some(party_destination))],
            None,
        );

        let destination = resolve_stealth_sweep_destination(&deposit, &wallet, &inventory).unwrap();

        assert_eq!(destination.as_deref(), Some(deposit_destination));
    }

    #[test]
    fn cross_party_shared_destination_blocks_under_fail_closed() {
        let destination = "0x6666666666666666666666666666666666666666";
        let target = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("party_1"),
            None,
        );
        let other = test_deposit(
            "deposit_2",
            "wallet_1",
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Some("party_2"),
            None,
        );
        let inventory = test_inventory(
            vec![
                test_counterparty("party_1", Some(destination)),
                test_counterparty("party_2", Some(&destination.to_ascii_uppercase())),
            ],
            Some(true),
        );
        let warning = detect_stealth_sweep_linkage(
            &target,
            destination,
            &[target.clone(), other],
            &inventory,
        );

        assert!(warning.is_some());
        let would_block = warning.is_some()
            && inventory
                .treasury_policy
                .as_ref()
                .map(|policy| policy.block_cross_party_linkage)
                .unwrap_or(false);
        assert!(would_block);
    }

    #[test]
    fn same_party_multiple_deposits_allowed() {
        let destination = "0x7777777777777777777777777777777777777777";
        let target = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("party_1"),
            None,
        );
        let other = test_deposit(
            "deposit_2",
            "wallet_1",
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Some("party_1"),
            None,
        );
        let inventory = test_inventory(
            vec![test_counterparty("party_1", Some(destination))],
            Some(true),
        );

        let warning = detect_stealth_sweep_linkage(
            &target,
            destination,
            &[target.clone(), other],
            &inventory,
        );

        assert!(warning.is_none());
    }

    #[test]
    fn policy_off_yields_warning_not_block() {
        let destination = "0x8888888888888888888888888888888888888888";
        let target = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("party_1"),
            None,
        );
        let other = test_deposit(
            "deposit_2",
            "wallet_1",
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Some("party_2"),
            None,
        );
        let inventory = test_inventory(
            vec![
                test_counterparty("party_1", Some(destination)),
                test_counterparty("party_2", Some(destination)),
            ],
            Some(false),
        );
        let warning = detect_stealth_sweep_linkage(
            &target,
            destination,
            &[target.clone(), other],
            &inventory,
        );

        assert!(warning.is_some());
        let would_block = warning.is_some()
            && inventory
                .treasury_policy
                .as_ref()
                .map(|policy| policy.block_cross_party_linkage)
                .unwrap_or(false);
        assert!(!would_block);
    }

    #[test]
    fn two_distinct_unattributed_deposits_to_same_wallet_default_link() {
        let wallet_destination = "0x9999999999999999999999999999999999999999";
        let target = test_deposit(
            "deposit_1",
            "wallet_1",
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            None,
            None,
        );
        let other = test_deposit(
            "deposit_2",
            "wallet_1",
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            None,
            None,
        );
        let wallet = test_wallet("wallet_1", Some(wallet_destination));
        let inventory = test_inventory(Vec::new(), Some(false));
        let target_destination = resolve_stealth_sweep_destination(&target, &wallet, &inventory)
            .unwrap()
            .expect("wallet default destination");

        let warning = detect_stealth_sweep_linkage(
            &target,
            &target_destination,
            &[target.clone(), other],
            &inventory,
        );

        assert!(warning.is_some());
    }

    #[test]
    fn native_expected_amount_rejects_dust_and_accepts_equal_or_over() {
        let dust = decode_quantity_hex("0x1").expect("valid dust amount");
        let exact = decode_quantity_hex("0x64").expect("valid exact amount");
        let over = decode_quantity_hex("0x65").expect("valid overpayment amount");

        assert!(!observed_amount_meets_expected(&dust, Some("0x64")).unwrap());
        assert!(observed_amount_meets_expected(&exact, Some("0x64")).unwrap());
        assert!(observed_amount_meets_expected(&over, Some("0x64")).unwrap());
    }

    #[test]
    fn erc20_expected_amount_rejects_dust_and_accepts_equal_or_over() {
        let dust = decode_quantity_hex("0xff").expect("valid dust amount");
        let exact = decode_quantity_hex("0x100").expect("valid exact amount");
        let over = decode_quantity_hex("0x101").expect("valid overpayment amount");

        assert!(!observed_amount_meets_expected(&dust, Some("0x100")).unwrap());
        assert!(observed_amount_meets_expected(&exact, Some("0x100")).unwrap());
        assert!(observed_amount_meets_expected(&over, Some("0x100")).unwrap());
    }

    #[test]
    fn deposit_without_expected_amount_still_requires_nonzero_observation() {
        let zero = decode_quantity_hex("0x0").expect("valid zero amount");
        let nonzero = decode_quantity_hex("0x1").expect("valid nonzero amount");

        assert!(!observed_amount_meets_expected(&zero, None).unwrap());
        assert!(observed_amount_meets_expected(&nonzero, None).unwrap());
    }

    #[test]
    fn native_expected_value_must_be_a_positive_quantity() {
        assert!(validate_optional_positive_quantity(Some("0x0"), "expected_value_wei").is_err());
        assert!(validate_optional_positive_quantity(Some("0x"), "expected_value_wei").is_err());
        assert!(validate_optional_positive_quantity(Some("0x1"), "expected_value_wei").is_ok());
    }

    #[test]
    fn erc20_expected_amount_must_be_a_positive_quantity() {
        assert!(validate_optional_positive_quantity(Some("0x0"), "expected_amount").is_err());
        assert!(validate_optional_positive_quantity(Some("0x"), "expected_amount").is_err());
        assert!(validate_optional_positive_quantity(Some("0x1"), "expected_amount").is_ok());
    }

    fn announcement_event_with_metadata(metadata: Vec<u8>) -> Erc5564AnnouncementEvent {
        Erc5564AnnouncementEvent {
            stealth_address: "0x1111111111111111111111111111111111111111".into(),
            caller_address: None,
            ephemeral_public_key_hex: hex::encode([0x03; 33]),
            metadata_hex: hex::encode(&metadata),
            view_tag_hex: hex::encode([metadata[0]]),
            block_number: None,
            transaction_hash: None,
            log_index: None,
        }
    }

    #[test]
    fn token_layout_hint_autopopulates_asset_and_expected_amount() {
        let token_address = "0x2222222222222222222222222222222222222222";
        let mut amount = [0u8; 32];
        amount[31] = 0x2a;
        let metadata = hex::decode(
            sigillum_core::encode_erc5564_metadata_erc20_transfer(0x7f, token_address, &amount)
                .unwrap(),
        )
        .unwrap();
        let event = announcement_event_with_metadata(metadata);

        let (asset_kind, hinted_token, expected) =
            resolve_announcement_asset_hints(&event, None).unwrap();

        assert_eq!(asset_kind, "erc20");
        assert_eq!(hinted_token.as_deref(), Some(token_address));
        assert_eq!(expected.as_deref(), Some("0x2a"));
    }

    #[test]
    fn native_layout_hint_sets_expected_amount_only() {
        let mut amount = [0u8; 32];
        amount[30] = 0x01;
        let metadata =
            hex::decode(sigillum_core::encode_erc5564_metadata_native(0x7f, &amount)).unwrap();
        let event = announcement_event_with_metadata(metadata);

        let (asset_kind, hinted_token, expected) =
            resolve_announcement_asset_hints(&event, None).unwrap();

        assert_eq!(asset_kind, "native");
        assert_eq!(hinted_token, None);
        assert_eq!(expected.as_deref(), Some("0x100"));
    }

    #[test]
    fn explicit_token_address_wins_over_metadata_hints() {
        let hinted_token = "0x2222222222222222222222222222222222222222";
        let metadata = hex::decode(
            sigillum_core::encode_erc5564_metadata_erc20_transfer(0x7f, hinted_token, &[1u8; 32])
                .unwrap(),
        )
        .unwrap();
        let event = announcement_event_with_metadata(metadata);
        let explicit = "0x3333333333333333333333333333333333333333";

        let (asset_kind, token, expected) =
            resolve_announcement_asset_hints(&event, Some(explicit)).unwrap();

        assert_eq!(asset_kind, "erc20");
        assert_eq!(token.as_deref(), Some(explicit));
        assert_eq!(expected, None);
    }

    #[test]
    fn unknown_layouts_and_zero_amounts_yield_no_hints() {
        // View-tag-only metadata (historical default).
        let event = announcement_event_with_metadata(vec![0x7f]);
        assert_eq!(
            resolve_announcement_asset_hints(&event, None).unwrap(),
            ("native", None, None)
        );

        // 57-byte token layout with an unrecognized selector: not acted on.
        let mut unknown = vec![0x7f];
        unknown.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        unknown.extend_from_slice(&[0x22; 20]);
        unknown.extend_from_slice(&[0u8; 32]);
        let event = announcement_event_with_metadata(unknown);
        assert_eq!(
            resolve_announcement_asset_hints(&event, None).unwrap(),
            ("native", None, None)
        );

        // Zero-amount hints carry no expected amount.
        let metadata = hex::decode(
            sigillum_core::encode_erc5564_metadata_erc20_transfer(
                0x7f,
                "0x2222222222222222222222222222222222222222",
                &[0u8; 32],
            )
            .unwrap(),
        )
        .unwrap();
        let event = announcement_event_with_metadata(metadata);
        let (asset_kind, token, expected) = resolve_announcement_asset_hints(&event, None).unwrap();
        assert_eq!(asset_kind, "erc20");
        assert!(token.is_some());
        assert_eq!(expected, None);
    }

    fn queue_with_gas_topup(
        id: &str,
        sponsor: &str,
        destination: &str,
    ) -> crate::queue_store::QueueState {
        crate::queue_store::QueueState {
            jobs: vec![sigillum_api::QueueJob {
                id: id.into(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: 1,
                updated_at_unix: 1,
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthGasTopup {
                    wallet_profile: "stealth".into(),
                    sponsor_address: sponsor.into(),
                    destination_address: destination.into(),
                    value_wei_hex: "0x1".into(),
                    gas_limit: None,
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
                receipt: Default::default(),
            }],
        }
    }

    #[test]
    fn sponsor_funding_two_parties_warns_with_mirrored_identity_axis() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let target = test_deposit(
            "dep_1",
            "w",
            "0xaaaa00000000000000000000000000000000aaaa",
            Some("party_a"),
            None,
        );
        let mut other = test_deposit(
            "dep_2",
            "w",
            "0xbbbb00000000000000000000000000000000bbbb",
            Some("party_b"),
            None,
        );
        other.gas_topup_job_id = Some("topup_1".into());
        let queue = queue_with_gas_topup("topup_1", sponsor, &other.stealth_address);

        let linkage = detect_stealth_gas_sponsor_linkage(
            &target,
            sponsor,
            "mainnet",
            &[other.clone()],
            &queue,
        )
        .unwrap();
        assert!(
            linkage
                .warning
                .starts_with("shared gas sponsor links this party"),
            "{}",
            linkage.warning
        );
        // Plan task 3.5: the same detection surfaces a structured
        // `common_gas_funder` risk finding (advisory).
        let finding = &linkage.risk_finding;
        assert_eq!(finding.category, "common_gas_funder");
        assert_eq!(finding.risk_level, "medium");
        assert_eq!(finding.subject_type, "gas_funder");
        assert_eq!(finding.subject, sponsor);
        assert_eq!(finding.wallet_family, "eth-stealth");
        assert_eq!(finding.provider_profile, "mainnet");
        assert_eq!(finding.chain_id, target.chain_id);
        assert!(
            finding
                .evidence
                .iter()
                .any(|value| value == "Linked payer: counterparty:party_b"),
            "evidence: {:?}",
            finding.evidence
        );

        // Same counterparty on both deposits: one identity, no linkage.
        let mut same_party = other.clone();
        same_party.counterparty_id = Some("party_a".into());
        assert!(
            detect_stealth_gas_sponsor_linkage(&target, sponsor, "mainnet", &[same_party], &queue)
                .is_none()
        );

        // A different sponsor funds the other deposit: no shared funder.
        assert!(
            detect_stealth_gas_sponsor_linkage(
                &target,
                "0x5555555555555555555555555555555555555555",
                "mainnet",
                &[other],
                &queue
            )
            .is_none()
        );
    }

    #[test]
    fn sponsor_linkage_ignores_deposits_without_tracked_topups() {
        let sponsor = "0x4444444444444444444444444444444444444444";
        let target = test_deposit(
            "dep_1",
            "w",
            "0xaaaa00000000000000000000000000000000aaaa",
            Some("party_a"),
            None,
        );
        let other = test_deposit(
            "dep_2",
            "w",
            "0xbbbb00000000000000000000000000000000bbbb",
            Some("party_b"),
            None,
        );
        let queue = queue_with_gas_topup("topup_1", sponsor, &other.stealth_address);

        // The other deposit never recorded a top-up job: nothing to link.
        assert!(
            detect_stealth_gas_sponsor_linkage(&target, sponsor, "mainnet", &[other], &queue)
                .is_none()
        );
    }
}

#[derive(Clone)]
struct DepositRefreshPlan {
    deposit_index: usize,
    provider: EvmProviderProfile,
    wallet: EthStealthWalletProfile,
}

#[derive(Clone, Debug)]
struct Erc5564AnnouncementEvent {
    stealth_address: String,
    caller_address: Option<String>,
    ephemeral_public_key_hex: String,
    metadata_hex: String,
    view_tag_hex: String,
    block_number: Option<String>,
    transaction_hash: Option<String>,
    log_index: Option<String>,
}

// ── Deposit Creation & Deletion ────────────────────────────────────────────

impl SigillumService {
    pub(crate) fn list_eth_stealth_deposits(
        &self,
        token: Option<&str>,
        query: super::list_query::EthStealthDepositListQuery,
    ) -> ServiceResult<EthStealthDepositListResponse> {
        use super::list_query::{
            CreatedUpdatedSort, DEPOSIT_STATUSES, SortOrder, effective_order, paginate,
            validated_value,
        };
        let _ = self.require_scope(token, super::capability_scopes::DEPOSITS_READ)?;
        let status = query
            .status
            .map(|value| validated_value("status", value, &DEPOSIT_STATUSES))
            .transpose()?;
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let mut deposits = deposits.eth_stealth;
        if let Some(status) = status.as_deref() {
            deposits.retain(|deposit| deposit.status == status);
        }
        if let Some(chain_id) = query.chain_id {
            deposits.retain(|deposit| deposit.chain_id == chain_id);
        }
        if let Some(counterparty_id) = query.counterparty_id.as_deref() {
            deposits.retain(|deposit| deposit.counterparty_id.as_deref() == Some(counterparty_id));
        }
        if let Some(sort) = query.sort {
            let order = effective_order(query.sort.as_ref(), query.order);
            let key = |deposit: &EthStealthDeposit| match sort {
                CreatedUpdatedSort::Created => deposit.created_at_unix,
                CreatedUpdatedSort::Updated => deposit.updated_at_unix,
            };
            match order {
                SortOrder::Asc => deposits.sort_by_key(&key),
                SortOrder::Desc => deposits.sort_by_key(|deposit| std::cmp::Reverse(key(deposit))),
            }
        }
        let (deposits, pagination) = paginate(deposits, query.page);
        Ok(EthStealthDepositListResponse {
            deposits,
            pagination,
        })
    }

    pub(crate) async fn scan_eth_stealth_announcements(
        &self,
        token: Option<&str>,
        body: EthStealthAnnouncementScanRequest,
    ) -> ServiceResult<EthStealthAnnouncementScanResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_CREATE)?;
        if body.auto_queue_sweep.unwrap_or(false) {
            self.require_scope(Some(token), super::capability_scopes::QUEUE_ENQUEUE_SWEEP)?;
        }
        let to_block = body
            .to_block
            .as_deref()
            .map(|value| normalize_log_block_tag(value, "to_block"))
            .transpose()?
            .unwrap_or_else(|| "latest".into());
        let limit = validated_announcement_scan_limit(body.limit)?;
        let token_address = body
            .token_address
            .as_deref()
            .map(super::evm::normalize_address)
            .transpose()?;
        validate_optional_quantity(body.min_sweep_amount_hex.as_deref(), "min_sweep_amount")?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;

        // Plan task 2.6: an explicit `from_block` always wins (manual
        // rescan); when omitted, resume from the persisted per-(wallet,
        // provider) announcement cursor, or scan from `earliest` when no
        // cursor is stored (first scan, or after `reset_cursor`). The cursor
        // read is a cheap store load; the authoritative mutation copy loads
        // under the operation guard below.
        let reset_cursor = body.reset_cursor.unwrap_or(false);
        let from_block = match body.from_block.as_deref() {
            Some(value) => normalize_log_block_tag(value, "from_block")?,
            None => {
                let stored =
                    crate::deposits::load_deposits(&self.state.base_dir).map_err(|error| {
                        ServiceError::internal(format!("Failed to load deposits: {error}"))
                    })?;
                match latest_announcement_scan_cursor(
                    &stored.announcement_scan_cursors,
                    &wallet.name,
                    &provider.name,
                )
                .filter(|_| !reset_cursor)
                {
                    Some(cursor) => {
                        encode_block_quantity(cursor.last_scanned_block.saturating_add(1))
                    }
                    None => "earliest".into(),
                }
            }
        };

        // Watch-only detection: the scan derives the viewing private key +
        // spending PUBLIC key only; the spending private key never
        // materializes outside a short zeroize-on-drop scope inside the core
        // derivation helper, so no spending secret enters the scan path. The
        // compartment must still be unlocked (the viewing key derives from
        // the master key). Sweep signing re-derives the full wallet later.
        let watch_view = self.with_vault(wallet.compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::vault_locked("Wallet compartment is locked."))?;
            derive_watch_only_sigillum_ethereum_stealth_wallet(
                master_key.as_ref(),
                &wallet.wallet,
                &wallet.short_name,
            )
            .map_err(map_wallet_error)
        })?;

        let topics = vec![
            erc5564_announcement_topic(),
            padded_u64_topic(ETHEREUM_STEALTH_SCHEME_ID),
        ];
        let logs = self
            .evm_logs_for_provider(
                provider.compartment_id,
                &provider,
                ERC5564_ANNOUNCER_ADDRESS,
                &topics,
                &from_block,
                &to_block,
            )
            .await?;

        let _guard = self.state.operation_guard().await;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let now = now_unix();
        let mut matched = 0usize;
        let mut created = 0usize;
        let mut existing = 0usize;
        let mut response_deposits = Vec::new();
        // Farthest announcement block this scan actually decoded (the cursor
        // anchor; mirrors the ERC-20 transfer-log `max_log_block` semantics).
        let mut cursor_block: Option<u64> = None;

        for log in logs.iter().take(limit) {
            let event = decode_erc5564_announcement_log(log)?;
            cursor_block = max_log_block(cursor_block, event.block_number.as_deref());
            let view_tag = hex::decode(&event.view_tag_hex)
                .ok()
                .and_then(|bytes| bytes.first().copied());
            // Dual-decode: probe the standard compressed-point convention
            // first, then the legacy x-only one, so announcements created
            // before the hash-convention switch (whose on-chain view tag only
            // matches under the legacy convention) are still found in the
            // same pass. The matched convention is persisted on the record so
            // sweeping derives the stealth key the same way. Runs watch-only:
            // viewing key + spending public key, no spending secret.
            let check = match check_ethereum_stealth_address_any_watch_only(
                &watch_view,
                &event.stealth_address,
                &event.ephemeral_public_key_hex,
                view_tag,
                &StealthHashConvention::PROBE_ORDER,
            ) {
                Ok(check) => check,
                Err(EthereumStealthError::ViewTagMismatch) => continue,
                Err(error) => return Err(map_wallet_error(error)),
            };
            if !check.matches {
                continue;
            }
            matched += 1;

            // EIP-5564 metadata SHOULD-layout hints: when the operator did not
            // pin an explicit `token_address`, a standards-following
            // announcement tells us the asset itself — the token layout
            // (transfer selector + token contract + amount) makes the match an
            // ERC-20 deposit candidate, the native layout a native one with an
            // expected amount. Unknown layouts yield no hints (never an
            // error) and the historical operator-driven defaults apply.
            let (event_asset_kind, event_token_address, hint_expected_amount_hex) =
                resolve_announcement_asset_hints(&event, token_address.as_deref())?;

            if let Some(existing_index) = deposits.eth_stealth.iter().position(|deposit| {
                discovered_deposit_matches(
                    deposit,
                    &wallet,
                    &event,
                    event_asset_kind,
                    &event_token_address,
                )
            }) {
                existing += 1;
                let existing_deposit = &mut deposits.eth_stealth[existing_index];
                if existing_deposit.stealth_hash_convention != check.stealth_hash_convention {
                    existing_deposit.stealth_hash_convention = check.stealth_hash_convention;
                    existing_deposit.updated_at_unix = now;
                }
                response_deposits.push(existing_deposit.clone());
                continue;
            }

            let deposit = EthStealthDeposit {
                id: random_id(),
                status: "pending".into(),
                asset_kind: event_asset_kind.into(),
                wallet_profile: wallet.name.clone(),
                chain_id: provider.chain_id,
                chain_id_assumed: false,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                stealth_meta_address: watch_view.meta_address().stealth_meta_address.clone(),
                stealth_address: event.stealth_address.clone(),
                ephemeral_public_key_hex: event.ephemeral_public_key_hex.clone(),
                view_tag_hex: check.view_tag_hex.clone(),
                stealth_hash_convention: check.stealth_hash_convention,
                announcement: Some(discovered_announcement_payload(&event)?),
                token_address: event_token_address.clone(),
                expected_amount_hex: hint_expected_amount_hex,
                observed_amount_hex: None,
                observed_native_balance_wei_hex: None,
                auto_queue_sweep: body.auto_queue_sweep.unwrap_or(false),
                sweep_destination_address: body
                    .sweep_destination_address
                    .clone()
                    .or_else(|| wallet.default_destination_address.clone()),
                min_sweep_amount_hex: body.min_sweep_amount_hex.clone(),
                queue_job_id: None,
                queue_job_state: None,
                note: Some(discovery_note(&event, body.note.as_deref())),
                created_at_unix: now,
                updated_at_unix: now,
                last_checked_at_unix: None,
                broadcast_transaction_hash_hex: None,
                counterparty_id: None,
                requested_gas_wei_hex: None,
                gas_topup_job_id: None,
                gas_topup_job_state: None,
            };
            created += 1;
            response_deposits.push(deposit.clone());
            deposits.eth_stealth.push(deposit);
        }

        // Plan task 2.6: advance the persisted announcement cursor to the
        // farthest block this scan covered — the highest PROCESSED log block
        // (never beyond what was decoded, so a `limit`-capped scan re-reads
        // the tail next time). When the range held no logs at all, anchor at
        // the concrete upper bound: a numeric `to_block`, or the chain head
        // for the default `latest` (best-effort — a head-read failure leaves
        // the cursor untouched rather than failing the scan); other block
        // tags can't be anchored honestly and also leave it untouched. The
        // cursor write rides the same atomic store save as the deposits.
        if cursor_block.is_none() && logs.is_empty() {
            cursor_block = match parse_block_quantity(&to_block) {
                Some(block) => Some(block),
                None if to_block == "latest" => self
                    .evm_block_number_for_provider(provider.compartment_id, &provider)
                    .await
                    .ok(),
                None => None,
            };
        }
        if let Some(last_scanned_block) = cursor_block {
            upsert_announcement_scan_cursor(
                &mut deposits.announcement_scan_cursors,
                EthStealthAnnouncementScanCursor {
                    wallet_profile: wallet.name.clone(),
                    provider_profile: provider.name.clone(),
                    chain_id: provider.chain_id,
                    last_scanned_block,
                    updated_at_unix: now,
                },
                reset_cursor,
            );
        }

        deposits
            .eth_stealth
            .sort_by(|left, right| left.created_at_unix.cmp(&right.created_at_unix));
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthAnnouncementScan {
                wallet_profile: wallet.name.clone(),
                provider_profile: provider.name.clone(),
                scanned: logs.len().min(limit),
                matched,
                created,
            },
        )?;

        Ok(EthStealthAnnouncementScanResponse {
            status: "scanned".into(),
            wallet_profile: wallet.name,
            provider_profile: provider.name,
            from_block,
            to_block,
            scanned: logs.len().min(limit),
            matched,
            created,
            existing,
            deposits: response_deposits,
        })
    }

    pub(crate) async fn create_eth_stealth_native_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositCreateNativeRequest,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_CREATE)?;
        if body.auto_queue_sweep.unwrap_or(false) {
            self.require_scope(Some(token), super::capability_scopes::QUEUE_ENQUEUE_SWEEP)?;
        }
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        validate_optional_positive_quantity(
            body.expected_value_wei_hex.as_deref(),
            "expected_value_wei",
        )?;
        validate_optional_quantity(
            body.min_sweep_value_wei_hex.as_deref(),
            "min_sweep_value_wei",
        )?;
        validate_optional_positive_quantity(body.gas_amount_wei_hex.as_deref(), "gas_amount_wei")?;
        if body.gas_amount_wei_hex.is_some() && body.request_gas != Some(true) {
            return Err(ServiceError::bad_request(
                "gas_amount_wei_hex requires request_gas",
            ));
        }
        let requested_gas_wei_hex = resolve_requested_gas_wei_hex(
            body.request_gas.unwrap_or(false),
            body.gas_amount_wei_hex.as_deref(),
            &provider,
            provider.native_gas_limit.unwrap_or(21_000),
        )?;

        self.persist_new_deposit(
            token,
            &wallet,
            &provider,
            body.ephemeral_private_key_hex,
            DepositBlueprint {
                wallet_profile: body.wallet_profile,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                asset_kind: "native".into(),
                token_address: None,
                expected_amount_hex: body.expected_value_wei_hex,
                auto_queue_sweep: body.auto_queue_sweep.unwrap_or(false),
                sweep_destination_address: body
                    .sweep_destination_address
                    .or(wallet.default_destination_address.clone()),
                min_sweep_amount_hex: body.min_sweep_value_wei_hex,
                note: body.note,
                requested_gas_wei_hex,
            },
        )
        .await
    }

    pub(crate) async fn create_eth_stealth_erc20_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositCreateErc20Request,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_CREATE)?;
        if body.auto_queue_sweep.unwrap_or(false) {
            self.require_scope(Some(token), super::capability_scopes::QUEUE_ENQUEUE_SWEEP)?;
        }
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        validate_optional_positive_quantity(
            body.expected_amount_hex.as_deref(),
            "expected_amount",
        )?;
        validate_optional_quantity(body.min_sweep_amount_hex.as_deref(), "min_sweep_amount")?;
        validate_optional_positive_quantity(body.gas_amount_wei_hex.as_deref(), "gas_amount_wei")?;
        if body.gas_amount_wei_hex.is_some() && body.request_gas != Some(true) {
            return Err(ServiceError::bad_request(
                "gas_amount_wei_hex requires request_gas",
            ));
        }
        let normalized_token = super::evm::normalize_address(&body.token_address)?;
        let requested_gas_wei_hex = resolve_requested_gas_wei_hex(
            body.request_gas.unwrap_or(false),
            body.gas_amount_wei_hex.as_deref(),
            &provider,
            provider.erc20_gas_limit.unwrap_or(65_000),
        )?;

        self.persist_new_deposit(
            token,
            &wallet,
            &provider,
            body.ephemeral_private_key_hex,
            DepositBlueprint {
                wallet_profile: body.wallet_profile,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                asset_kind: "erc20".into(),
                token_address: Some(normalized_token),
                expected_amount_hex: body.expected_amount_hex,
                auto_queue_sweep: body.auto_queue_sweep.unwrap_or(false),
                sweep_destination_address: body
                    .sweep_destination_address
                    .or(wallet.default_destination_address.clone()),
                min_sweep_amount_hex: body.min_sweep_amount_hex,
                note: body.note,
                requested_gas_wei_hex,
            },
        )
        .await
    }

    /// Shared deposit creation: derive stealth address, build record, persist, and audit.
    ///
    /// Both native and ERC-20 deposit flows converge here after validating their
    /// type-specific fields and constructing a [`DepositBlueprint`].
    async fn persist_new_deposit(
        &self,
        token: &str,
        wallet: &EthStealthWalletProfile,
        provider: &EvmProviderProfile,
        ephemeral_private_key_hex: Option<String>,
        blueprint: DepositBlueprint,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let meta = self.with_vault(wallet.compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::vault_locked("Wallet compartment is locked."))?;
            // Only the meta-address (spending/viewing public keys) is needed
            // to mint a deposit address, so derive the watch-only view: the
            // spending secret never persists outside the core helper's
            // zeroize-on-drop scope.
            let derived = derive_watch_only_sigillum_ethereum_stealth_wallet(
                master_key.as_ref(),
                &wallet.wallet,
                &wallet.short_name,
            )
            .map_err(map_wallet_error)?;
            Ok(derived.meta_address().clone())
        })?;
        let payment = self.eth_stealth_generate(EthStealthGenerateRequest {
            stealth_meta_address: meta.stealth_meta_address.clone(),
            ephemeral_private_key_hex,
        })?;
        let warnings = payment.warnings.clone();

        // Payer-attached gas (`request_gas`): rebuild the announcement with
        // EIP-5564 SHOULD-layout metadata so a standards-aware payer wallet
        // learns the asset/amount (and, for native, the payment+gas total) to
        // attach. Without it the announcement keeps the minimal view-tag-only
        // metadata from generation.
        let announcement = match blueprint.requested_gas_wei_hex.as_deref() {
            Some(requested_gas) => Some(build_gas_requesting_announcement(
                &payment,
                &blueprint,
                requested_gas,
            )?),
            None => payment.announcement,
        };

        let now = now_unix();
        let deposit = EthStealthDeposit {
            id: random_id(),
            status: "pending".into(),
            asset_kind: blueprint.asset_kind,
            wallet_profile: blueprint.wallet_profile,
            chain_id: provider.chain_id,
            chain_id_assumed: false,
            wallet_compartment_id: blueprint.wallet_compartment_id,
            provider_compartment_id: blueprint.provider_compartment_id,
            wallet: blueprint.wallet,
            short_name: blueprint.short_name,
            stealth_meta_address: meta.stealth_meta_address,
            stealth_address: payment.stealth_address,
            ephemeral_public_key_hex: payment.ephemeral_public_key_hex,
            view_tag_hex: payment.view_tag_hex,
            // New deposits are always generated with the standard convention.
            stealth_hash_convention: payment.stealth_hash_convention,
            announcement,
            token_address: blueprint.token_address,
            expected_amount_hex: blueprint.expected_amount_hex,
            observed_amount_hex: None,
            observed_native_balance_wei_hex: None,
            auto_queue_sweep: blueprint.auto_queue_sweep,
            sweep_destination_address: blueprint.sweep_destination_address,
            min_sweep_amount_hex: blueprint.min_sweep_amount_hex,
            queue_job_id: None,
            queue_job_state: None,
            note: blueprint.note,
            created_at_unix: now,
            updated_at_unix: now,
            last_checked_at_unix: None,
            broadcast_transaction_hash_hex: None,
            counterparty_id: None,
            requested_gas_wei_hex: blueprint.requested_gas_wei_hex,
            gas_topup_job_id: None,
            gas_topup_job_state: None,
        };

        let _guard = self.state.operation_guard().await;
        let mut state = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        state.eth_stealth.push(deposit.clone());
        state
            .eth_stealth
            .sort_by(|left, right| left.created_at_unix.cmp(&right.created_at_unix));
        crate::deposits::save_deposits(&self.state.base_dir, &state)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthCreate {
                id: deposit.id.clone(),
                wallet_profile: deposit.wallet_profile.clone(),
                asset_kind: deposit.asset_kind.clone(),
                token_address: deposit.token_address.clone(),
            },
        )?;

        Ok(EthStealthDepositMutationResponse {
            status: "created".into(),
            deposit,
            warnings,
        })
    }

    // ── Deposit Deletion ──────────────────────────────────────────────────

    pub(crate) async fn delete_eth_stealth_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositDeleteRequest,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_DELETE)?;
        let _guard = self.state.operation_guard().await;
        let mut state = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let index = state
            .eth_stealth
            .iter()
            .position(|deposit| deposit.id == body.id)
            .ok_or_else(|| ServiceError::not_found("Deposit not found."))?;
        let deposit = state.eth_stealth.remove(index);
        crate::deposits::save_deposits(&self.state.base_dir, &state)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthDelete {
                id: deposit.id.clone(),
            },
        )?;

        Ok(EthStealthDepositMutationResponse {
            status: "deleted".into(),
            deposit,
            warnings: Vec::new(),
        })
    }

    pub(crate) async fn tag_eth_stealth_deposit(
        &self,
        token: Option<&str>,
        body: ReceivingDepositTagRequest,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_DELETE)?;
        let _guard = self.state.operation_guard().await;
        let counterparty_id = body
            .counterparty_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let counterparty_name = if let Some(id) = counterparty_id.as_deref() {
            let inventory =
                crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                    ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
                })?;
            let Some(party) = inventory
                .parties
                .iter()
                .find(|party| party.id.as_str() == id)
            else {
                return Err(ServiceError::not_found("Counterparty not found."));
            };
            Some(party.name.clone())
        } else {
            None
        };

        let mut state = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let deposit = state
            .eth_stealth
            .iter_mut()
            .find(|deposit| deposit.id == body.deposit_id)
            .ok_or_else(|| ServiceError::not_found("Deposit not found."))?;
        deposit.counterparty_id = counterparty_id.clone();
        deposit.updated_at_unix = now_unix();
        let deposit = deposit.clone();

        crate::deposits::save_deposits(&self.state.base_dir, &state)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        if let Some(name) = counterparty_name {
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::TreasuryReceiveBind { name },
            )?;
        }

        Ok(EthStealthDepositMutationResponse {
            status: if counterparty_id.is_some() {
                "tagged".into()
            } else {
                "untagged".into()
            },
            deposit,
            warnings: Vec::new(),
        })
    }

    // ── Deposit Refresh ───────────────────────────────────────────────────

    pub(crate) async fn refresh_eth_stealth_deposits(
        &self,
        token: Option<&str>,
        body: EthStealthDepositRefreshRequest,
    ) -> ServiceResult<EthStealthDepositRefreshResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_REFRESH)?;
        if body.auto_enqueue.unwrap_or(false) {
            self.require_scope(Some(token), super::capability_scopes::QUEUE_ENQUEUE_SWEEP)?;
        }
        let _guard = self.state.operation_guard().await;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let response = self
            .refresh_eth_stealth_deposits_state(token, &mut deposits, &mut queue, body)
            .await?;

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthRefresh {
                processed: response.processed,
                detected: response.detected,
                queued: response.queued,
            },
        )?;

        Ok(response)
    }

    // ── Deposit Sweep Enqueueing ──────────────────────────────────────────

    pub(crate) async fn enqueue_eth_stealth_deposit_sweep(
        &self,
        token: Option<&str>,
        body: EthStealthDepositEnqueueSweepRequest,
    ) -> ServiceResult<EthStealthDepositEnqueueSweepResponse> {
        let token = self.require_session(token)?;
        let _guard = self.state.operation_guard().await;
        let mut deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        let mut queue = crate::queue_store::load_queue(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load queue: {error}")))?;
        let all_deposits = deposits.eth_stealth.clone();
        let deposit_snapshot = {
            let deposit = deposits
                .eth_stealth
                .iter_mut()
                .find(|deposit| deposit.id == body.id)
                .ok_or_else(|| ServiceError::not_found("Deposit not found."))?;
            let queue_state = deposit
                .queue_job_id
                .as_deref()
                .and_then(|id| queue.jobs.iter().find(|job| job.id == id))
                .map(|job| job.state.clone());
            if !body.force.unwrap_or(false)
                && queue_state
                    .as_deref()
                    .map(super::queue::is_active_or_completed_queue_state)
                    .unwrap_or(false)
            {
                return Err(ServiceError::conflict(
                    "Deposit already has an active or completed sweep job.",
                ));
            }

            let (provider, wallet) = self.resolve_wallet_profile(&deposit.wallet_profile)?;
            let outcome = self
                .enqueue_deposit_sweep_job(
                    DepositSweepJobParams {
                        token,
                        deposit: &*deposit,
                        other_deposits: &all_deposits,
                        wallet: &wallet,
                        provider: &provider,
                        strict_destination: true,
                    },
                    &mut queue,
                )
                .await?;
            deposit.queue_job_id = Some(outcome.enqueue.job.id.clone());
            deposit.queue_job_state = Some(outcome.enqueue.job.state.clone());
            deposit.status = super::queue::queue_status(&outcome.enqueue.job.state);
            if let Some(topup_job) = outcome.gas_topup_job.as_ref() {
                deposit.gas_topup_job_id = Some(topup_job.id.clone());
                deposit.gas_topup_job_state = Some(topup_job.state.clone());
            }
            deposit.updated_at_unix = now_unix();
            (deposit.clone(), outcome)
        };

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthEnqueueSweep {
                id: deposit_snapshot.0.id.clone(),
                job_id: deposit_snapshot.1.enqueue.job.id.clone(),
            },
        )?;

        Ok(EthStealthDepositEnqueueSweepResponse {
            status: deposit_snapshot.1.enqueue.status,
            deposit: deposit_snapshot.0,
            job: deposit_snapshot.1.enqueue.job,
            linkage_warning: deposit_snapshot.1.linkage_warning,
            risk_findings: deposit_snapshot.1.risk_findings,
        })
    }
}

// ── Sweep Job Construction ────────────────────────────────────────────────

struct DepositSweepJobParams<'a> {
    token: &'a str,
    deposit: &'a EthStealthDeposit,
    other_deposits: &'a [EthStealthDeposit],
    wallet: &'a sigillum_api::EthStealthWalletProfile,
    provider: &'a sigillum_api::EvmProviderProfile,
    strict_destination: bool,
}

/// Outcome of enqueueing a deposit sweep: the sweep job itself, an optional
/// non-blocking linkage warning (destination- and/or sponsor-axis), the
/// structured `common_gas_funder` risk findings backing the sponsor-axis
/// warning (plan task 3.5, advisory only), and the sponsor gas top-up job
/// when one was planned ahead of the sweep.
struct DepositSweepEnqueueOutcome {
    enqueue: QueueEnqueueResponse,
    linkage_warning: Option<String>,
    risk_findings: Vec<RiskFinding>,
    gas_topup_job: Option<QueueJob>,
}

impl SigillumService {
    async fn enqueue_deposit_sweep_job(
        &self,
        params: DepositSweepJobParams<'_>,
        queue: &mut crate::queue_store::QueueState,
    ) -> ServiceResult<DepositSweepEnqueueOutcome> {
        let DepositSweepJobParams {
            token,
            deposit,
            other_deposits,
            wallet,
            provider,
            strict_destination,
        } = params;
        // Plan task 2.5: stealth deposit sweeps re-validate the Sweep-family
        // execution gate at enqueue time, at parity with the seed-family
        // `/api/queue/enqueue/*` endpoints (`enqueue_job`) — the sweep jobs
        // built below map to `ExecutionFamily::Sweep`. The drain re-checks
        // the same gate per job. A denial fails the whole enqueue (including
        // any sponsor gas top-up planning) before a job is persisted.
        self.require_execution_family_allowed(super::queue::ExecutionFamily::Sweep)?;
        let inventory =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        let destination = resolve_stealth_sweep_destination(deposit, wallet, &inventory)?;
        let destination_linkage_warning = destination.as_deref().and_then(|destination| {
            detect_stealth_sweep_linkage(deposit, destination, other_deposits, &inventory)
        });
        let block_cross_party_linkage = inventory
            .treasury_policy
            .as_ref()
            .map(|policy| policy.block_cross_party_linkage)
            .unwrap_or(false);
        if destination_linkage_warning.is_some() && block_cross_party_linkage {
            return Err(ServiceError::policy_violation("cross_party_linkage"));
        }

        let mut gas_topup_job = None;
        let mut sponsor_linkage: Option<StealthSponsorLinkage> = None;
        let job = if deposit.asset_kind == "erc20" {
            let recipient_address = destination.ok_or_else(|| {
                ServiceError::bad_request(
                    "ERC-20 deposit requires sweep_destination_address or wallet default destination.",
                )
            })?;
            self.authorize_transaction_policy(TransactionPolicyCheck {
                kind: TransactionPolicyKind::RoutedTransfer,
                destination_address: Some(&recipient_address),
                asset_kind: "erc20",
                amount_hex: deposit.min_sweep_amount_hex.as_deref().unwrap_or("0x0"),
            })?;

            // Sponsor gas top-up: an ERC-20 deposit whose stealth address
            // lacks native gas gets a sponsor-funded top-up as a queue job
            // PRECEDING the sweep (the sweep depends on it and stays blocked
            // until gas is confirmed on-chain). Sponsor linkage flows through
            // the same cross-party accounting as seed-plan sponsor funding:
            // warn by default, hard-block when `block_cross_party_linkage`.
            let (planned_topup, planned_sponsor_linkage) = self
                .plan_stealth_gas_topup_job(
                    deposit,
                    wallet,
                    provider,
                    other_deposits,
                    &inventory,
                    queue,
                )
                .await?;
            if planned_sponsor_linkage.is_some() && block_cross_party_linkage {
                return Err(ServiceError::policy_violation("cross_party_linkage"));
            }
            gas_topup_job = planned_topup;
            sponsor_linkage = planned_sponsor_linkage;

            QueueJob {
                id: random_id(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: now_unix(),
                updated_at_unix: now_unix(),
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthErc20Sweep {
                    wallet_profile: deposit.wallet_profile.clone(),
                    stealth_address: deposit.stealth_address.clone(),
                    ephemeral_public_key_hex: deposit.ephemeral_public_key_hex.clone(),
                    token_address: deposit.token_address.clone().ok_or_else(|| {
                        ServiceError::internal("ERC-20 deposit missing token_address")
                    })?,
                    recipient_address: Some(recipient_address),
                    min_amount_hex: deposit.min_sweep_amount_hex.clone(),
                    gas_limit: provider.erc20_gas_limit,
                    view_tag_hex: Some(deposit.view_tag_hex.clone()),
                    // The sweep derives the stealth key with the record's
                    // stored convention (dual-decoded at detection time).
                    stealth_hash_convention: Some(deposit.stealth_hash_convention),
                    prerequisite_job_ids: gas_topup_job
                        .as_ref()
                        .map(|job| vec![job.id.clone()])
                        .unwrap_or_default(),
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
                receipt: Default::default(),
            }
        } else {
            let destination_address = if strict_destination {
                destination.ok_or_else(|| {
                    ServiceError::bad_request(
                        "Native deposit requires sweep_destination_address or wallet default destination.",
                    )
                })?
            } else {
                destination.unwrap_or_else(|| {
                    wallet
                        .default_destination_address
                        .clone()
                        .unwrap_or_default()
                })
            };
            if destination_address.is_empty() {
                return Err(ServiceError::bad_request(
                    "Native deposit requires sweep_destination_address or wallet default destination.",
                ));
            }
            self.authorize_transaction_policy(TransactionPolicyCheck {
                kind: TransactionPolicyKind::RoutedTransfer,
                destination_address: Some(&destination_address),
                asset_kind: "native",
                amount_hex: deposit.min_sweep_amount_hex.as_deref().unwrap_or("0x0"),
            })?;
            QueueJob {
                id: random_id(),
                state: "queued".into(),
                attempts: 0,
                created_at_unix: now_unix(),
                updated_at_unix: now_unix(),
                next_attempt_after_unix: None,
                payload: QueueJobPayload::EthStealthNativeSweep {
                    wallet_profile: deposit.wallet_profile.clone(),
                    stealth_address: deposit.stealth_address.clone(),
                    ephemeral_public_key_hex: deposit.ephemeral_public_key_hex.clone(),
                    destination_address: Some(destination_address),
                    min_value_wei_hex: deposit.min_sweep_amount_hex.clone(),
                    gas_limit: provider.native_gas_limit,
                    view_tag_hex: Some(deposit.view_tag_hex.clone()),
                    // The sweep derives the stealth key with the record's
                    // stored convention (dual-decoded at detection time).
                    stealth_hash_convention: Some(deposit.stealth_hash_convention),
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
                receipt: Default::default(),
            }
        };

        let linkage_warning = [
            destination_linkage_warning,
            sponsor_linkage
                .as_ref()
                .map(|linkage| linkage.warning.clone()),
        ]
        .into_iter()
        .flatten()
        .reduce(|left, right| format!("{left}; {right}"));
        let risk_findings = sponsor_linkage
            .map(|linkage| vec![linkage.risk_finding])
            .unwrap_or_default();

        // The top-up lands in the queue BEFORE its dependent sweep so a
        // single drain broadcasts it first (dependency ordering is by job id
        // state, but queue order keeps one drain sufficient in the common
        // case).
        if let Some(topup_job) = gas_topup_job.as_ref() {
            queue.jobs.push(topup_job.clone());
            self.record_audit(
                self.state.active_compartment_id_for(token),
                AuditEventSpec::QueueEnqueue {
                    id: topup_job.id.clone(),
                    job_kind: AuditQueueJobKind::from_payload(&topup_job.payload),
                },
            )?;
        }
        queue.jobs.push(job.clone());
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::QueueEnqueue {
                id: job.id.clone(),
                job_kind: AuditQueueJobKind::from_payload(&job.payload),
            },
        )?;

        Ok(DepositSweepEnqueueOutcome {
            enqueue: QueueEnqueueResponse {
                status: "queued".into(),
                job,
            },
            linkage_warning,
            risk_findings,
            gas_topup_job,
        })
    }

    /// Plan a sponsor gas top-up for an ERC-20 deposit that lacks native gas.
    ///
    /// Returns the top-up job to enqueue ahead of the sweep plus an optional
    /// sponsor-linkage detection (warning + structured `common_gas_funder`
    /// risk finding). `(None, _)` means "no top-up": policy off,
    /// gas already sufficient, fee basis missing, cap exceeded, sponsor
    /// unavailable (locked compartment) or insolvent, or a live top-up
    /// already tracked — in every case the deposit keeps its historical
    /// behavior (the sweep job blocks on gas until the operator funds the
    /// address manually, e.g. via payer-attached gas).
    ///
    /// Mirrors the seed-plan rules (`service/inventory/gas_topup.rs`):
    /// amount = 1.5x the sweep's estimated gas, capped by
    /// `max_gas_topup_wei_hex`; the sponsor must cover the top-up plus its
    /// own transfer gas. The sponsor is the stealth wallet's derived gas
    /// sponsor (`stealth_gas_sponsor_address`), funded by the operator
    /// out-of-band — see `docs/architecture.md`.
    async fn plan_stealth_gas_topup_job(
        &self,
        deposit: &EthStealthDeposit,
        wallet: &sigillum_api::EthStealthWalletProfile,
        provider: &sigillum_api::EvmProviderProfile,
        other_deposits: &[EthStealthDeposit],
        inventory: &WalletInventoryState,
        queue: &crate::queue_store::QueueState,
    ) -> ServiceResult<(Option<QueueJob>, Option<StealthSponsorLinkage>)> {
        if !super::inventory::gas_topup::gas_topup_policy_enabled(
            inventory.treasury_policy.as_ref(),
        ) {
            return Ok((None, None));
        }
        // One live top-up per deposit: an active or already-broadcast job is
        // reused, never duplicated.
        if let Some(existing) = deposit
            .gas_topup_job_id
            .as_deref()
            .and_then(|id| queue.jobs.iter().find(|job| job.id == id))
        {
            if super::queue::is_active_or_completed_queue_state(&existing.state) {
                return Ok((None, None));
            }
        }
        let Some(max_fee_hex) = provider.max_fee_per_gas_hex.as_deref() else {
            return Ok((None, None));
        };
        let max_fee = decode_quantity_hex(max_fee_hex).map_err(map_wallet_error)?;
        let gas_limit = provider.erc20_gas_limit.unwrap_or(65_000);
        let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
        // Gas shortfall judged from the last observed native balance on the
        // record (refresh keeps it current); unknown balance means "assume
        // short". The sweep's own on-chain check stays authoritative.
        let observed_native = deposit
            .observed_native_balance_wei_hex
            .as_deref()
            .map(decode_quantity_hex)
            .transpose()
            .map_err(map_wallet_error)?
            .unwrap_or([0u8; 32]);
        if compare_u256(&observed_native, &gas_cost).is_ge() {
            return Ok((None, None));
        }
        // Seed-path formula: 1.5x the sweep's estimated gas, policy-capped.
        let topup = super::inventory::treasury::add_u256(
            &gas_cost,
            &super::inventory::gas_topup::shr1_u256(&gas_cost),
        );
        if super::inventory::gas_topup::topup_exceeds_cap(
            &topup,
            inventory
                .treasury_policy
                .as_ref()
                .and_then(|policy| policy.max_gas_topup_wei_hex.as_deref()),
        ) {
            return Ok((None, None));
        }
        let Some(sponsor_address) =
            self.stealth_gas_sponsor_address(wallet.compartment_id, &wallet.wallet)?
        else {
            return Ok((None, None));
        };
        if sponsor_address.eq_ignore_ascii_case(&deposit.stealth_address) {
            return Ok((None, None));
        }
        // The sponsor must cover the top-up plus its own transfer gas,
        // verified against a fresh balance read.
        let sponsor_balance_hex = self
            .evm_native_balance_for_provider(
                provider.compartment_id,
                provider,
                &sponsor_address,
                "latest",
            )
            .await?;
        let sponsor_balance =
            decode_quantity_hex(&sponsor_balance_hex).map_err(map_wallet_error)?;
        let sponsor_gas_cost =
            multiply_u256_u64(&max_fee, provider.native_gas_limit.unwrap_or(21_000));
        let required = super::inventory::treasury::add_u256(&topup, &sponsor_gas_cost);
        if compare_u256(&sponsor_balance, &required).is_lt() {
            return Ok((None, None));
        }

        let linkage = detect_stealth_gas_sponsor_linkage(
            deposit,
            &sponsor_address,
            &provider.name,
            other_deposits,
            queue,
        );
        let job = QueueJob {
            id: random_id(),
            state: "queued".into(),
            attempts: 0,
            created_at_unix: now_unix(),
            updated_at_unix: now_unix(),
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthGasTopup {
                wallet_profile: deposit.wallet_profile.clone(),
                sponsor_address,
                destination_address: deposit.stealth_address.clone(),
                value_wei_hex: super::evm::encode_quantity_u256(&topup),
                gas_limit: provider.native_gas_limit,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        };
        Ok((Some(job), linkage))
    }

    // ── Deposit State Refresh & Sync ───────────────────────────────────────

    pub(super) async fn refresh_eth_stealth_deposits_state(
        &self,
        token: &str,
        deposits: &mut crate::deposits::DepositState,
        queue: &mut crate::queue_store::QueueState,
        body: EthStealthDepositRefreshRequest,
    ) -> ServiceResult<EthStealthDepositRefreshResponse> {
        let limit = self
            .state
            .runtime_policy()
            .deposit_refresh_limit(body.limit);
        let auto_enqueue = body.auto_enqueue.unwrap_or(true);
        let registry = crate::profiles::load_profiles(&self.state.base_dir).map_err(|error| {
            ServiceError::internal(format!("Failed to load profile registry: {error}"))
        })?;
        let mut processed = Vec::new();
        let mut detected = 0usize;
        let mut queued = 0usize;
        let mut plans = Vec::new();
        let mut observation_plans = Vec::new();

        for (deposit_index, deposit) in deposits.eth_stealth.iter().enumerate() {
            if plans.len() >= limit {
                break;
            }
            if let Some(id) = body.id.as_deref() {
                if deposit.id != id {
                    continue;
                }
            }

            let (provider, wallet) = super::profiles::resolve_wallet_profile_in_registry(
                &registry,
                &deposit.wallet_profile,
            )?;
            plans.push(DepositRefreshPlan {
                deposit_index,
                provider: provider.clone(),
                wallet,
            });
            observation_plans.push(super::evm::EvmBalanceObservationPlan {
                deposit_index,
                provider_compartment_id: deposit.provider_compartment_id,
                provider,
                owner_address: deposit.stealth_address.clone(),
                token_address: deposit.token_address.clone(),
            });

            if body.id.is_some() {
                break;
            }
        }

        let observations = self.fetch_balance_observations(observation_plans).await?;
        let plans_by_index: HashMap<usize, DepositRefreshPlan> = plans
            .into_iter()
            .map(|plan| (plan.deposit_index, plan))
            .collect();
        let all_deposits = deposits.eth_stealth.clone();

        for observation in observations {
            let plan = plans_by_index
                .get(&observation.deposit_index)
                .ok_or_else(|| {
                    ServiceError::internal("Missing deposit refresh plan for observation")
                })?;
            let deposit = deposits
                .eth_stealth
                .get_mut(observation.deposit_index)
                .ok_or_else(|| ServiceError::internal("Deposit index went out of range"))?;
            let native_balance = decode_quantity_hex(&observation.native_balance_wei_hex)
                .map_err(map_wallet_error)?;

            deposit.chain_id = plan.provider.chain_id;
            deposit.chain_id_assumed = false;
            deposit.observed_native_balance_wei_hex =
                (deposit.asset_kind == "erc20").then(|| observation.native_balance_wei_hex.clone());
            deposit.observed_amount_hex = Some(observation.observed_amount_hex.clone());
            deposit.last_checked_at_unix = Some(now_unix());
            deposit.updated_at_unix = now_unix();

            let (queue_state, _) = sync_eth_stealth_deposit_with_queue(deposit, queue);
            let observed_amount_raw =
                decode_quantity_hex(&observation.observed_amount_hex).map_err(map_wallet_error)?;
            let expected_ready = observed_amount_meets_expected(
                &observed_amount_raw,
                deposit.expected_amount_hex.as_deref(),
            )?;
            let min_ready = match deposit.min_sweep_amount_hex.as_deref() {
                Some(minimum) => compare_u256(
                    &observed_amount_raw,
                    &decode_quantity_hex(minimum).map_err(map_wallet_error)?,
                )
                .is_ge(),
                None => !is_zero_u256(&observed_amount_raw),
            };

            if !is_zero_u256(&observed_amount_raw) {
                detected += 1;
            }

            deposit.status = if let Some(job_state) = queue_state.clone() {
                super::queue::queue_status(&job_state)
            } else if is_zero_u256(&observed_amount_raw) {
                "pending".into()
            } else if !expected_ready {
                "underfunded".into()
            } else if deposit.asset_kind == "erc20"
                && !gas_balance_sufficient_for_erc20(deposit, &plan.provider, &native_balance)?
            {
                "funded_needs_gas".into()
            } else {
                "funded".into()
            };

            let has_active_job = queue_state
                .as_deref()
                .map(super::queue::is_active_queue_state)
                .unwrap_or(false);
            if auto_enqueue
                && deposit.auto_queue_sweep
                && !has_active_job
                && expected_ready
                && min_ready
            {
                let enqueue_result = self
                    .enqueue_deposit_sweep_job(
                        DepositSweepJobParams {
                            token,
                            deposit: &*deposit,
                            other_deposits: &all_deposits,
                            wallet: &plan.wallet,
                            provider: &plan.provider,
                            strict_destination: false,
                        },
                        queue,
                    )
                    .await;
                if let Ok(outcome) = enqueue_result {
                    queued += 1;
                    deposit.queue_job_id = Some(outcome.enqueue.job.id.clone());
                    deposit.queue_job_state = Some(outcome.enqueue.job.state.clone());
                    deposit.status = super::queue::queue_status(&outcome.enqueue.job.state);
                    if let Some(topup_job) = outcome.gas_topup_job.as_ref() {
                        deposit.gas_topup_job_id = Some(topup_job.id.clone());
                        deposit.gas_topup_job_state = Some(topup_job.state.clone());
                    }
                }
            }

            processed.push(deposit.clone());
        }

        Ok(EthStealthDepositRefreshResponse {
            processed: processed.len(),
            detected,
            queued,
            deposits: processed,
        })
    }
}

// ── Validation & Helper Functions ──────────────────────────────────────────

fn validate_optional_quantity(value: Option<&str>, label: &str) -> ServiceResult<()> {
    if let Some(value) = value {
        decode_quantity_hex(value).map_err(|_| {
            ServiceError::bad_request(format!("{label} must be a valid hex quantity"))
        })?;
    }
    Ok(())
}

fn validate_optional_positive_quantity(value: Option<&str>, label: &str) -> ServiceResult<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let quantity = decode_quantity_hex(value)
        .map_err(|_| ServiceError::bad_request(format!("{label} must be a valid hex quantity")))?;
    if is_zero_u256(&quantity) {
        return Err(ServiceError::bad_request(format!(
            "{label} must be greater than zero"
        )));
    }
    Ok(())
}

/// Resolve the payer-attached gas request to a concrete wei amount.
///
/// `None` (no `request_gas`) keeps the announcement metadata minimal
/// (view-tag-only). Without an explicit `gas_amount_wei_hex`, the provider
/// profile's static sweep gas estimate (`max_fee_per_gas × gas_limit`, the
/// asset's sweep gas limit resolved by the caller) is used — the same fee
/// basis the sweep itself checks against.
fn resolve_requested_gas_wei_hex(
    request_gas: bool,
    gas_amount_wei_hex: Option<&str>,
    provider: &EvmProviderProfile,
    gas_limit: u64,
) -> ServiceResult<Option<String>> {
    if !request_gas {
        return Ok(None);
    }
    if let Some(explicit) = gas_amount_wei_hex {
        let amount = decode_quantity_hex(explicit).map_err(map_wallet_error)?;
        return Ok(Some(super::evm::encode_quantity_u256(&amount)));
    }
    let max_fee_hex = provider.max_fee_per_gas_hex.as_deref().ok_or_else(|| {
        ServiceError::bad_request(
            "gas_amount_wei_hex is required when the provider profile has no static max fee",
        )
    })?;
    let max_fee = decode_quantity_hex(max_fee_hex).map_err(map_wallet_error)?;
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    Ok(Some(super::evm::encode_quantity_u256(&gas_cost)))
}

/// Build the announcement payload for a gas-requesting deposit, with metadata
/// following the EIP-5564 SHOULD layouts:
///
/// - native deposits: the native layout (`view tag ‖ 0xeeeeeeee ‖ sentinel ‖
///   amount`), amount = expected value + requested gas — the TOTAL native
///   value the payer should attach (the EIP-5564 "Recipients' transaction
///   costs" sponsorship pattern, where the sender attaches ETH sponsoring
///   the recipient's subsequent transactions);
/// - ERC-20 deposits: the token layout (`view tag ‖ transfer(address,
///   uint256) selector ‖ token contract ‖ amount`), amount = expected token
///   amount (zero when unspecified); the requested native gas rides on the
///   deposit record (`requested_gas_wei_hex`) as payment instructions, since
///   the token layout carries asset info only.
fn build_gas_requesting_announcement(
    payment: &sigillum_api::EthStealthGenerateResponse,
    blueprint: &DepositBlueprint,
    requested_gas_wei_hex: &str,
) -> ServiceResult<EthStealthAnnouncementPayload> {
    let view_tag = hex::decode(&payment.view_tag_hex)
        .ok()
        .and_then(|bytes| bytes.first().copied())
        .ok_or_else(|| ServiceError::internal("generated payment is missing a view tag"))?;
    let expected_amount = blueprint
        .expected_amount_hex
        .as_deref()
        .map(decode_quantity_hex)
        .transpose()
        .map_err(map_wallet_error)?
        .unwrap_or([0u8; 32]);
    let metadata_hex = if blueprint.asset_kind == "erc20" {
        let token_address = blueprint.token_address.as_deref().ok_or_else(|| {
            ServiceError::internal("ERC-20 deposit blueprint is missing token_address")
        })?;
        encode_erc5564_metadata_erc20_transfer(view_tag, token_address, &expected_amount)
            .map_err(map_wallet_error)?
    } else {
        let requested_gas = decode_quantity_hex(requested_gas_wei_hex).map_err(map_wallet_error)?;
        let total = super::inventory::treasury::add_u256(&expected_amount, &requested_gas);
        encode_erc5564_metadata_native(view_tag, &total)
    };
    let calldata_hex = encode_erc5564_announce_calldata(
        payment.scheme_id,
        &payment.stealth_address,
        &payment.ephemeral_public_key_hex,
        &metadata_hex,
    )
    .map_err(map_wallet_error)?;
    Ok(EthStealthAnnouncementPayload {
        announcer_address: ERC5564_ANNOUNCER_ADDRESS.into(),
        announce_function: ERC5564_ANNOUNCE_FUNCTION.into(),
        scheme_id: payment.scheme_id,
        stealth_address: payment.stealth_address.clone(),
        ephemeral_public_key_hex: payment.ephemeral_public_key_hex.clone(),
        metadata_hex,
        calldata_hex,
        value_wei_hex: "0x0".into(),
    })
}

/// A deposit is economically ready only after the observed balance reaches the
/// amount requested by the creator. Deposits without an explicit expectation
/// preserve the historical "any non-zero amount" behavior.
fn observed_amount_meets_expected(
    observed_amount: &[u8; 32],
    expected_amount_hex: Option<&str>,
) -> ServiceResult<bool> {
    if is_zero_u256(observed_amount) {
        return Ok(false);
    }

    let Some(expected_amount_hex) = expected_amount_hex else {
        return Ok(true);
    };
    let expected_amount = decode_quantity_hex(expected_amount_hex).map_err(map_wallet_error)?;
    Ok(compare_u256(observed_amount, &expected_amount).is_ge())
}

fn resolve_stealth_sweep_destination(
    deposit: &EthStealthDeposit,
    wallet: &EthStealthWalletProfile,
    inventory: &WalletInventoryState,
) -> ServiceResult<Option<String>> {
    if deposit.sweep_destination_address.is_some() {
        return Ok(deposit.sweep_destination_address.clone());
    }

    let party = deposit.counterparty_id.as_deref().and_then(|id| {
        inventory
            .parties
            .iter()
            .find(|party| party.id.as_str() == id)
    });
    let party_destination = party
        .map(counterparty_sweep_destination)
        .transpose()?
        .flatten();
    Ok(party_destination.or(wallet.default_destination_address.clone()))
}

fn counterparty_sweep_destination(party: &Counterparty) -> ServiceResult<Option<String>> {
    party
        .sweep_destination_address
        .as_deref()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(super::evm::normalize_address)
        .transpose()
}

fn detect_stealth_sweep_linkage(
    target: &EthStealthDeposit,
    target_destination: &str,
    other_deposits: &[EthStealthDeposit],
    inventory: &WalletInventoryState,
) -> Option<String> {
    let destination_key = normalize_stealth_linkage_address(target_destination);
    if destination_key.is_empty() {
        return None;
    }

    let target_identity = stealth_sweep_identity_key(target);
    let mut linked_identities = BTreeSet::new();
    for other in other_deposits {
        if other.id == target.id {
            continue;
        }
        let other_destination =
            resolve_other_stealth_sweep_destination(other, target, target_destination, inventory);
        let Some(other_destination) = other_destination else {
            continue;
        };
        if normalize_stealth_linkage_address(&other_destination) != destination_key {
            continue;
        }
        let other_identity = stealth_sweep_identity_key(other);
        if other_identity != target_identity {
            linked_identities.insert(other_identity);
        }
    }

    if linked_identities.is_empty() {
        None
    } else {
        Some(
            "destination shared with another payer; set a distinct per-party sweep destination"
                .into(),
        )
    }
}

fn resolve_other_stealth_sweep_destination(
    other: &EthStealthDeposit,
    target: &EthStealthDeposit,
    target_destination: &str,
    inventory: &WalletInventoryState,
) -> Option<String> {
    other
        .sweep_destination_address
        .as_deref()
        .map(str::trim)
        .filter(|destination| !destination.is_empty())
        .map(str::to_string)
        .or_else(|| other_counterparty_sweep_destination(other, inventory))
        .or_else(|| {
            (other.wallet_profile == target.wallet_profile).then(|| target_destination.to_string())
        })
}

fn other_counterparty_sweep_destination(
    deposit: &EthStealthDeposit,
    inventory: &WalletInventoryState,
) -> Option<String> {
    let counterparty_id = deposit.counterparty_id.as_deref()?;
    inventory
        .parties
        .iter()
        .find(|party| party.id.as_str() == counterparty_id)
        .and_then(|party| party.sweep_destination_address.as_deref())
        .map(str::trim)
        .filter(|destination| !destination.is_empty())
        .map(str::to_string)
}

fn stealth_sweep_identity_key(deposit: &EthStealthDeposit) -> String {
    deposit
        .counterparty_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| format!("counterparty:{id}"))
        .unwrap_or_else(|| {
            format!(
                "unattributed:{}",
                normalize_stealth_linkage_address(&deposit.stealth_address)
            )
        })
}

/// Sponsor-linkage detection result for a planned stealth gas top-up: the
/// long-standing non-blocking warning string plus the structured
/// `common_gas_funder` risk finding (plan task 3.5, advisory only — blocking
/// stays governed by `block_cross_party_linkage`).
struct StealthSponsorLinkage {
    warning: String,
    risk_finding: RiskFinding,
}

/// Cross-party sponsor linkage for stealth gas top-ups (single hop,
/// destination axis): one sponsor funding stealth deposits attributed to
/// DIFFERENT identities links those parties on-chain — an observer sees a
/// common gas funder paying into both parties' deposit addresses. Mirrors
/// the seed-plan FundGas funder analysis
/// (`service/inventory/planner.rs::analyze_plan_linkage`), with the funded
/// destination's identity resolved from the deposit record's counterparty
/// tag (`stealth_sweep_identity_key`). Since plan task 3.5 the detection
/// also produces the structured `common_gas_funder` risk finding via the
/// shared risk machinery (`service/inventory/risk.rs`).
fn detect_stealth_gas_sponsor_linkage(
    target: &EthStealthDeposit,
    sponsor_address: &str,
    provider_profile: &str,
    other_deposits: &[EthStealthDeposit],
    queue: &crate::queue_store::QueueState,
) -> Option<StealthSponsorLinkage> {
    let sponsor_key = normalize_stealth_linkage_address(sponsor_address);
    if sponsor_key.is_empty() {
        return None;
    }
    let target_identity = stealth_sweep_identity_key(target);
    let mut linked_identities = BTreeSet::new();
    for other in other_deposits {
        if other.id == target.id {
            continue;
        }
        let Some(topup_job_id) = other.gas_topup_job_id.as_deref() else {
            continue;
        };
        let Some(job) = queue.jobs.iter().find(|job| job.id == topup_job_id) else {
            continue;
        };
        let QueueJobPayload::EthStealthGasTopup {
            sponsor_address: other_sponsor,
            ..
        } = &job.payload
        else {
            continue;
        };
        if normalize_stealth_linkage_address(other_sponsor) != sponsor_key {
            continue;
        }
        let other_identity = stealth_sweep_identity_key(other);
        if other_identity != target_identity {
            linked_identities.insert(other_identity);
        }
    }

    if linked_identities.is_empty() {
        None
    } else {
        let mut linked_labels: Vec<String> = linked_identities.into_iter().collect();
        linked_labels.sort();
        let risk_finding = super::inventory::planner::common_gas_funder_finding(
            "eth-stealth",
            &target.wallet_profile,
            provider_profile,
            target.chain_id,
            sponsor_address,
            &linked_labels,
            now_unix(),
        );
        Some(StealthSponsorLinkage {
            warning:
                "shared gas sponsor links this party with another payer; fund gas from a distinct \
                 sponsor or request payer-attached gas"
                    .into(),
            risk_finding,
        })
    }
}

fn normalize_stealth_linkage_address(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn gas_balance_sufficient_for_erc20(
    deposit: &EthStealthDeposit,
    provider: &sigillum_api::EvmProviderProfile,
    native_balance: &[u8; 32],
) -> ServiceResult<bool> {
    let Some(max_fee_hex) = provider.max_fee_per_gas_hex.as_deref() else {
        return Ok(false);
    };
    let max_fee = decode_quantity_hex(max_fee_hex).map_err(map_wallet_error)?;
    let gas_limit = provider.erc20_gas_limit.unwrap_or(65_000);
    let gas_cost = multiply_u256_u64(&max_fee, gas_limit);
    let min_amount_ready = match deposit.min_sweep_amount_hex.as_deref() {
        Some(minimum) => {
            let minimum = decode_quantity_hex(minimum).map_err(map_wallet_error)?;
            let observed = deposit
                .observed_amount_hex
                .as_deref()
                .map(decode_quantity_hex)
                .transpose()
                .map_err(map_wallet_error)?
                .unwrap_or([0u8; 32]);
            compare_u256(&observed, &minimum).is_ge()
        }
        None => true,
    };
    Ok(min_amount_ready && compare_u256(native_balance, &gas_cost).is_ge())
}

fn validated_announcement_scan_limit(limit: Option<usize>) -> ServiceResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_ANNOUNCEMENT_SCAN_LIMIT);
    if limit == 0 || limit > MAX_ANNOUNCEMENT_SCAN_LIMIT {
        return Err(ServiceError::bad_request(format!(
            "limit must be between 1 and {MAX_ANNOUNCEMENT_SCAN_LIMIT}"
        )));
    }
    Ok(limit)
}

/// Highest block number seen across processed announcement logs (mirrors the
/// ERC-20 transfer-log `max_log_block` cursor semantics).
fn max_log_block(cursor: Option<u64>, block_number: Option<&str>) -> Option<u64> {
    match (cursor, block_number.and_then(parse_block_quantity)) {
        (Some(current), Some(next)) => Some(current.max(next)),
        (None, Some(next)) => Some(next),
        (current, None) => current,
    }
}

/// Parse a `0x`-prefixed block quantity; `None` for named block tags.
fn parse_block_quantity(value: &str) -> Option<u64> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok()
}

fn encode_block_quantity(value: u64) -> String {
    format!("0x{value:x}")
}

/// The stored announcement-scan cursor for a (wallet profile, provider
/// profile) pair, if any.
fn latest_announcement_scan_cursor<'a>(
    cursors: &'a [EthStealthAnnouncementScanCursor],
    wallet_profile: &str,
    provider_profile: &str,
) -> Option<&'a EthStealthAnnouncementScanCursor> {
    cursors
        .iter()
        .filter(|cursor| {
            cursor.wallet_profile == wallet_profile && cursor.provider_profile == provider_profile
        })
        .max_by_key(|cursor| cursor.last_scanned_block)
}

/// Upsert the cursor for a (wallet, provider) pair: monotonic max normally
/// (a manual rescan of old blocks never drags the cursor backward), a
/// wholesale re-anchor after `reset_cursor`.
fn upsert_announcement_scan_cursor(
    cursors: &mut Vec<EthStealthAnnouncementScanCursor>,
    next: EthStealthAnnouncementScanCursor,
    reset: bool,
) {
    if let Some(existing) = cursors.iter_mut().find(|existing| {
        existing.wallet_profile == next.wallet_profile
            && existing.provider_profile == next.provider_profile
    }) {
        existing.chain_id = next.chain_id;
        existing.updated_at_unix = next.updated_at_unix;
        existing.last_scanned_block = if reset {
            next.last_scanned_block
        } else {
            existing.last_scanned_block.max(next.last_scanned_block)
        };
    } else {
        cursors.push(next);
    }
}

fn normalize_log_block_tag(value: &str, label: &str) -> ServiceResult<String> {
    let trimmed = value.trim();
    if matches!(
        trimmed,
        "earliest" | "latest" | "pending" | "safe" | "finalized"
    ) {
        return Ok(trimmed.into());
    }
    let raw = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .ok_or_else(|| {
            ServiceError::bad_request(format!("{label} must be a block tag or 0x quantity"))
        })?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::bad_request(format!(
            "{label} must be a block tag or 0x quantity"
        )));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn erc5564_announcement_topic() -> String {
    let digest = Keccak256::digest(b"Announcement(uint256,address,address,bytes,bytes)");
    format!("0x{}", hex::encode(digest))
}

fn padded_u64_topic(value: u64) -> String {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    format!("0x{}", hex::encode(word))
}

fn decode_erc5564_announcement_log(
    log: &super::evm::EvmLogEntry,
) -> ServiceResult<Erc5564AnnouncementEvent> {
    if log.topics.len() < 3 {
        return Err(ServiceError::internal(
            "Provider ERC-5564 announcement log is missing indexed topics",
        ));
    }
    let stealth_address = topic_address(&log.topics[2], "stealth address")?;
    let caller_address = log
        .topics
        .get(3)
        .map(|topic| topic_address(topic, "caller address"))
        .transpose()?;
    let data = decode_prefixed_hex(&log.data, "announcement data")?;
    let ephemeral_public_key = decode_abi_dynamic_bytes(&data, abi_word_as_usize(&data, 0)?)?;
    let metadata = decode_abi_dynamic_bytes(&data, abi_word_as_usize(&data, 32)?)?;
    let view_tag = metadata.first().ok_or_else(|| {
        ServiceError::internal("Provider ERC-5564 announcement metadata is missing view tag")
    })?;
    let view_tag_hex = hex::encode([*view_tag]);
    Ok(Erc5564AnnouncementEvent {
        stealth_address,
        caller_address,
        ephemeral_public_key_hex: hex::encode(ephemeral_public_key),
        metadata_hex: hex::encode(metadata),
        view_tag_hex,
        block_number: log.block_number.clone(),
        transaction_hash: log.transaction_hash.clone(),
        log_index: log.log_index.clone(),
    })
}

fn topic_address(topic: &str, label: &str) -> ServiceResult<String> {
    let bytes = decode_prefixed_hex(topic, label)?;
    if bytes.len() != 32 {
        return Err(ServiceError::internal(format!(
            "Provider ERC-5564 topic has invalid {label} length"
        )));
    }
    Ok(format!("0x{}", hex::encode(&bytes[12..])))
}

fn decode_prefixed_hex(value: &str, label: &str) -> ServiceResult<Vec<u8>> {
    let raw = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if raw.len() % 2 != 0 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::internal(format!(
            "Provider ERC-5564 {label} is not valid hex"
        )));
    }
    hex::decode(raw).map_err(|error| {
        ServiceError::internal(format!("Provider ERC-5564 {label} decode failed: {error}"))
    })
}

fn abi_word_as_usize(data: &[u8], offset: usize) -> ServiceResult<usize> {
    let word = data.get(offset..offset + 32).ok_or_else(|| {
        ServiceError::internal("Provider ERC-5564 announcement data is truncated")
    })?;
    let mut value = 0usize;
    for byte in word {
        value = value
            .checked_mul(256)
            .ok_or_else(|| ServiceError::internal("Provider ERC-5564 ABI offset is too large"))?;
        value = value
            .checked_add(*byte as usize)
            .ok_or_else(|| ServiceError::internal("Provider ERC-5564 ABI offset is too large"))?;
    }
    Ok(value)
}

fn decode_abi_dynamic_bytes(data: &[u8], offset: usize) -> ServiceResult<Vec<u8>> {
    let len = abi_word_as_usize(data, offset)?;
    let start = offset
        .checked_add(32)
        .ok_or_else(|| ServiceError::internal("Provider ERC-5564 ABI offset is too large"))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| ServiceError::internal("Provider ERC-5564 ABI length is too large"))?;
    let bytes = data.get(start..end).ok_or_else(|| {
        ServiceError::internal("Provider ERC-5564 announcement bytes are truncated")
    })?;
    Ok(bytes.to_vec())
}

/// Resolve the effective asset hints for a matched announcement.
///
/// Explicit operator input (`token_address` on the scan request) always wins.
/// Otherwise the EIP-5564 metadata SHOULD layouts provide the asset info: a
/// token layout using the ERC-20 `transfer(address,uint256)` selector makes
/// the match an ERC-20 candidate with the hinted contract and (when non-zero)
/// expected amount; a native layout keeps it native with the hinted expected
/// amount. Unknown layouts and unrecognized selectors yield the historical
/// native default with no expected amount — hints are advisory and never fail
/// the scan.
fn resolve_announcement_asset_hints(
    event: &Erc5564AnnouncementEvent,
    explicit_token_address: Option<&str>,
) -> ServiceResult<(&'static str, Option<String>, Option<String>)> {
    if let Some(explicit) = explicit_token_address {
        return Ok(("erc20", Some(explicit.to_string()), None));
    }
    let metadata = hex::decode(&event.metadata_hex).unwrap_or_default();
    match decode_erc5564_metadata_hints(&metadata) {
        Some(Erc5564MetadataHints::Token {
            function_selector,
            token_address,
            amount,
        }) if function_selector == ERC5564_METADATA_ERC20_TRANSFER_SELECTOR => {
            let token_address = super::evm::normalize_address(&token_address)?;
            let expected =
                (!is_zero_u256(&amount)).then(|| super::evm::encode_quantity_u256(&amount));
            Ok(("erc20", Some(token_address), expected))
        }
        Some(Erc5564MetadataHints::Native { amount_wei }) => {
            let expected =
                (!is_zero_u256(&amount_wei)).then(|| super::evm::encode_quantity_u256(&amount_wei));
            Ok(("native", None, expected))
        }
        _ => Ok(("native", None, None)),
    }
}

fn discovered_deposit_matches(
    deposit: &EthStealthDeposit,
    wallet: &EthStealthWalletProfile,
    event: &Erc5564AnnouncementEvent,
    asset_kind: &str,
    token_address: &Option<String>,
) -> bool {
    deposit.wallet_profile == wallet.name
        && deposit.asset_kind == asset_kind
        && deposit
            .stealth_address
            .eq_ignore_ascii_case(&event.stealth_address)
        && deposit
            .ephemeral_public_key_hex
            .eq_ignore_ascii_case(&event.ephemeral_public_key_hex)
        && deposit.token_address.as_ref() == token_address.as_ref()
}

fn discovered_announcement_payload(
    event: &Erc5564AnnouncementEvent,
) -> ServiceResult<EthStealthAnnouncementPayload> {
    let calldata_hex = encode_erc5564_announce_calldata(
        ETHEREUM_STEALTH_SCHEME_ID,
        &event.stealth_address,
        &event.ephemeral_public_key_hex,
        &event.metadata_hex,
    )
    .map_err(map_wallet_error)?;
    Ok(EthStealthAnnouncementPayload {
        announcer_address: ERC5564_ANNOUNCER_ADDRESS.into(),
        announce_function: ERC5564_ANNOUNCE_FUNCTION.into(),
        scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
        stealth_address: event.stealth_address.clone(),
        ephemeral_public_key_hex: event.ephemeral_public_key_hex.clone(),
        metadata_hex: event.metadata_hex.clone(),
        calldata_hex,
        value_wei_hex: "0x0".into(),
    })
}

fn discovery_note(event: &Erc5564AnnouncementEvent, operator_note: Option<&str>) -> String {
    let mut parts = vec![ERC5564_DISCOVERY_SOURCE.to_string()];
    if let Some(block) = event.block_number.as_deref() {
        parts.push(format!("block={block}"));
    }
    if let Some(tx) = event.transaction_hash.as_deref() {
        parts.push(format!("tx={tx}"));
    }
    if let Some(log_index) = event.log_index.as_deref() {
        parts.push(format!("log={log_index}"));
    }
    if let Some(caller) = event.caller_address.as_deref() {
        parts.push(format!("caller={caller}"));
    }
    if let Some(note) = operator_note.filter(|note| !note.trim().is_empty()) {
        parts.push(format!("note={}", note.trim()));
    }
    parts.join("; ")
}

// ── Queue Synchronization ─────────────────────────────────────────────────

pub(super) fn sync_eth_stealth_deposits_with_queue(
    deposits: &mut crate::deposits::DepositState,
    queue: &crate::queue_store::QueueState,
) -> usize {
    let mut reconciled = 0usize;
    for deposit in &mut deposits.eth_stealth {
        if sync_eth_stealth_deposit_with_queue(deposit, queue).1 {
            reconciled += 1;
        }
    }
    reconciled
}

fn sync_eth_stealth_deposit_with_queue(
    deposit: &mut EthStealthDeposit,
    queue: &crate::queue_store::QueueState,
) -> (Option<String>, bool) {
    let previous_queue_job_state = deposit.queue_job_state.clone();
    let previous_status = deposit.status.clone();
    let previous_broadcast = deposit.broadcast_transaction_hash_hex.clone();
    let previous_gas_topup_job_state = deposit.gas_topup_job_state.clone();
    let job = deposit
        .queue_job_id
        .as_deref()
        .and_then(|id| queue.jobs.iter().find(|candidate| candidate.id == id));
    let queue_state = job.map(|job| job.state.clone());
    deposit.queue_job_state = queue_state.clone();
    if let Some(hash) = job.and_then(|job| job.broadcast_transaction_hash_hex.clone()) {
        deposit.broadcast_transaction_hash_hex = Some(hash);
    }
    if let Some(state) = queue_state.as_deref() {
        deposit.status = super::queue::queue_status(state);
    }
    // Mirror the sponsor gas top-up's queue state so operators see what a
    // gas-starved deposit is waiting for.
    deposit.gas_topup_job_state = deposit
        .gas_topup_job_id
        .as_deref()
        .and_then(|id| queue.jobs.iter().find(|candidate| candidate.id == id))
        .map(|job| job.state.clone());
    (
        queue_state,
        previous_queue_job_state != deposit.queue_job_state
            || previous_status != deposit.status
            || previous_broadcast != deposit.broadcast_transaction_hash_hex
            || previous_gas_topup_job_state != deposit.gas_topup_job_state,
    )
}
