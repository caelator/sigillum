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

use std::collections::{BTreeSet, HashMap};

use sha3::{Digest, Keccak256};
use sigillum_api::{
    Counterparty, EthStealthAnnouncementPayload, EthStealthAnnouncementScanRequest,
    EthStealthAnnouncementScanResponse, EthStealthDeposit, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositEnqueueSweepResponse,
    EthStealthDepositListResponse, EthStealthDepositMutationResponse,
    EthStealthDepositRefreshRequest, EthStealthDepositRefreshResponse, EthStealthGenerateRequest,
    EthStealthWalletProfile, EvmProviderProfile, QueueEnqueueResponse, QueueJob, QueueJobPayload,
    ReceivingDepositTagRequest,
};
use sigillum_core::{
    ERC5564_ANNOUNCE_FUNCTION, ERC5564_ANNOUNCER_ADDRESS, ETHEREUM_STEALTH_SCHEME_ID,
    EthereumStealthError, VaultLifecycle, check_ethereum_stealth_address, decode_quantity_hex,
    derive_sigillum_ethereum_stealth_wallet, encode_erc5564_announce_calldata,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use sigillum_api::TreasuryPolicy;

    fn abi_word(value: usize) -> String {
        format!("{value:064x}")
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
    ) -> ServiceResult<EthStealthDepositListResponse> {
        let _ = self.require_scope(token, super::capability_scopes::DEPOSITS_READ)?;
        let deposits = crate::deposits::load_deposits(&self.state.base_dir)
            .map_err(|error| ServiceError::internal(format!("Failed to load deposits: {error}")))?;
        Ok(EthStealthDepositListResponse {
            deposits: deposits.eth_stealth,
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
        let from_block = normalize_log_block_tag(&body.from_block, "from_block")?;
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
        let asset_kind = if token_address.is_some() {
            "erc20"
        } else {
            "native"
        };

        let derived_wallet = self.with_vault(wallet.compartment_id, |vault| {
            let master_key = vault
                .extract_master_key()
                .ok_or_else(|| ServiceError::forbidden("Wallet compartment is locked."))?;
            derive_sigillum_ethereum_stealth_wallet(
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

        for log in logs.iter().take(limit) {
            let event = decode_erc5564_announcement_log(log)?;
            let view_tag = hex::decode(&event.view_tag_hex)
                .ok()
                .and_then(|bytes| bytes.first().copied());
            let check = match check_ethereum_stealth_address(
                &derived_wallet,
                &event.stealth_address,
                &event.ephemeral_public_key_hex,
                view_tag,
            ) {
                Ok(check) => check,
                Err(EthereumStealthError::ViewTagMismatch) => continue,
                Err(error) => return Err(map_wallet_error(error)),
            };
            if !check.matches {
                continue;
            }
            matched += 1;

            if let Some(existing_deposit) = deposits.eth_stealth.iter().find(|deposit| {
                discovered_deposit_matches(deposit, &wallet, &event, asset_kind, &token_address)
            }) {
                existing += 1;
                response_deposits.push(existing_deposit.clone());
                continue;
            }

            let deposit = EthStealthDeposit {
                id: random_id(),
                status: "pending".into(),
                asset_kind: asset_kind.into(),
                wallet_profile: wallet.name.clone(),
                chain_id: provider.chain_id,
                chain_id_assumed: false,
                wallet_compartment_id: wallet.compartment_id,
                provider_compartment_id: provider.compartment_id,
                wallet: wallet.wallet.clone(),
                short_name: wallet.short_name.clone(),
                stealth_meta_address: derived_wallet.meta_address().stealth_meta_address.clone(),
                stealth_address: event.stealth_address.clone(),
                ephemeral_public_key_hex: event.ephemeral_public_key_hex.clone(),
                view_tag_hex: check.view_tag_hex.clone(),
                announcement: Some(discovered_announcement_payload(&event)?),
                token_address: token_address.clone(),
                expected_amount_hex: None,
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
            };
            created += 1;
            response_deposits.push(deposit.clone());
            deposits.eth_stealth.push(deposit);
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
        validate_optional_quantity(body.expected_value_wei_hex.as_deref(), "expected_value_wei")?;
        validate_optional_quantity(
            body.min_sweep_value_wei_hex.as_deref(),
            "min_sweep_value_wei",
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
            },
        )
        .await
    }

    pub(crate) async fn create_eth_stealth_erc20_deposit(
        &self,
        token: Option<&str>,
        body: EthStealthDepositCreateErc20Request,
    ) -> ServiceResult<EthStealthDepositMutationResponse> {
        let token = self.require_scope(token, super::capability_scopes::DEPOSITS_DELETE)?;
        let (provider, wallet) = self.resolve_wallet_profile(&body.wallet_profile)?;
        validate_optional_quantity(body.expected_amount_hex.as_deref(), "expected_amount")?;
        validate_optional_quantity(body.min_sweep_amount_hex.as_deref(), "min_sweep_amount")?;
        let normalized_token = super::evm::normalize_address(&body.token_address)?;

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
                .ok_or_else(|| ServiceError::forbidden("Wallet compartment is locked."))?;
            let derived = derive_sigillum_ethereum_stealth_wallet(
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
            announcement: payment.announcement,
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
            let (enqueue, linkage_warning) = self.enqueue_deposit_sweep_job(
                DepositSweepJobParams {
                    token,
                    deposit: &*deposit,
                    other_deposits: &all_deposits,
                    wallet: &wallet,
                    provider: &provider,
                    strict_destination: true,
                },
                &mut queue,
            )?;
            deposit.queue_job_id = Some(enqueue.job.id.clone());
            deposit.queue_job_state = Some(enqueue.job.state.clone());
            deposit.status = super::queue::queue_status(&enqueue.job.state);
            deposit.updated_at_unix = now_unix();
            (deposit.clone(), enqueue, linkage_warning)
        };

        crate::queue_store::save_queue(&self.state.base_dir, &queue)
            .map_err(|error| ServiceError::internal(format!("Failed to save queue: {error}")))?;
        crate::deposits::save_deposits(&self.state.base_dir, &deposits)
            .map_err(|error| ServiceError::internal(format!("Failed to save deposits: {error}")))?;

        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::DepositsEthStealthEnqueueSweep {
                id: deposit_snapshot.0.id.clone(),
                job_id: deposit_snapshot.1.job.id.clone(),
            },
        )?;

        Ok(EthStealthDepositEnqueueSweepResponse {
            status: deposit_snapshot.1.status,
            deposit: deposit_snapshot.0,
            job: deposit_snapshot.1.job,
            linkage_warning: deposit_snapshot.2,
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

impl SigillumService {
    fn enqueue_deposit_sweep_job(
        &self,
        params: DepositSweepJobParams<'_>,
        queue: &mut crate::queue_store::QueueState,
    ) -> ServiceResult<(QueueEnqueueResponse, Option<String>)> {
        let DepositSweepJobParams {
            token,
            deposit,
            other_deposits,
            wallet,
            provider,
            strict_destination,
        } = params;
        let inventory =
            crate::inventory::load_wallet_inventory(&self.state.base_dir).map_err(|error| {
                ServiceError::internal(format!("Failed to load wallet inventory: {error}"))
            })?;
        let destination = resolve_stealth_sweep_destination(deposit, wallet, &inventory)?;
        let linkage_warning = destination.as_deref().and_then(|destination| {
            detect_stealth_sweep_linkage(deposit, destination, other_deposits, &inventory)
        });
        if linkage_warning.is_some()
            && inventory
                .treasury_policy
                .as_ref()
                .map(|policy| policy.block_cross_party_linkage)
                .unwrap_or(false)
        {
            return Err(ServiceError::policy_violation("cross_party_linkage"));
        }

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
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
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
                },
                last_error: None,
                transaction_hash_hex: None,
                broadcast_transaction_hash_hex: None,
            }
        };

        queue.jobs.push(job.clone());
        self.record_audit(
            self.state.active_compartment_id_for(token),
            AuditEventSpec::QueueEnqueue {
                id: job.id.clone(),
                job_kind: AuditQueueJobKind::from_payload(&job.payload),
            },
        )?;

        Ok((
            QueueEnqueueResponse {
                status: "queued".into(),
                job,
            },
            linkage_warning,
        ))
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
            if auto_enqueue && deposit.auto_queue_sweep && !has_active_job && min_ready {
                let enqueue_result = self.enqueue_deposit_sweep_job(
                    DepositSweepJobParams {
                        token,
                        deposit: &*deposit,
                        other_deposits: &all_deposits,
                        wallet: &plan.wallet,
                        provider: &plan.provider,
                        strict_destination: false,
                    },
                    queue,
                );
                if let Ok((enqueue_result, _linkage_warning)) = enqueue_result {
                    queued += 1;
                    deposit.queue_job_id = Some(enqueue_result.job.id.clone());
                    deposit.queue_job_state = Some(enqueue_result.job.state.clone());
                    deposit.status = super::queue::queue_status(&enqueue_result.job.state);
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
    (
        queue_state,
        previous_queue_job_state != deposit.queue_job_state
            || previous_status != deposit.status
            || previous_broadcast != deposit.broadcast_transaction_hash_hex,
    )
}
