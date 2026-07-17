use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sigillum_api::{
    ActiveCompartment, CapabilitySessionRequest, ClaimCandidateProbe, ConsolidationPlan,
    Counterparty, DefiTokenProbe, ErrorResponse, EthStealthAnnouncementPayload, EthStealthDeposit,
    EthStealthDepositCreateNativeRequest, EthStealthDepositListResponse, EvmProviderProfile,
    EvmProviderProfileListResponse, EvmProviderProfileUpsertRequest, EvmProviderRef,
    NftMetadataCacheEntry, ProviderPartitionObservation, QueueEthStealthNativeSweepRequest,
    QueueJob, QueueJobListResponse, QueueJobPayload, ReceivingCoverage, ReceivingDepositTagRequest,
    ReceivingItem,
    ReceivingOverviewResponse, ReceivingPartyGroup, ReceivingTotals, StatusResponse,
    StealthPaymentRef, TreasuryAllowedDestination, TreasuryAllowedDestinationInput, TreasuryPolicy,
    TreasuryPolicyResponse, TreasuryPolicyUpdateRequest, UnlockedCompartment,
    WalletAddressActivityState, WalletAddressClassification, WalletAssetHolding, WalletAssetKind,
    WalletDiscoveryBlockCursor, WalletDiscoveryCheckpoint, WalletDiscoveryJob,
    WalletInventoryAddress, WalletInventoryListResponse, WalletInventoryScanRequest,
    WalletPlanStatus, WalletPlanStepAction, WalletPlanStepStatus, WalletSignerStatus,
    WalletSimulationStatus, WatchAddressProbe,
};
use std::fmt::Debug;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let json = serde_json::to_string(value).expect("serialize fixture to JSON");
    let decoded: T = serde_json::from_str(&json).expect("deserialize fixture from JSON");
    assert_eq!(&decoded, value, "JSON roundtrip changed value: {json}");
}

fn assert_wire_literal<T>(value: T, literal: &str)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_string(&value).expect("serialize wire enum");
    assert_eq!(json, serde_json::to_string(literal).unwrap());
    let decoded: T = serde_json::from_str(&json).expect("deserialize wire enum");
    assert_eq!(decoded, value);
}

#[test]
fn wallet_domain_enums_serialize_known_wire_literals() {
    assert_wire_literal(WalletAddressActivityState::Funded, "funded");
    assert_wire_literal(WalletAddressActivityState::Active, "active");
    assert_wire_literal(WalletAddressActivityState::Empty, "empty");

    for (value, literal) in [
        (
            WalletAddressClassification::SignerAvailable,
            "signer_available",
        ),
        (WalletAddressClassification::WatchOnly, "watch_only"),
        (WalletAddressClassification::SignerUnknown, "signer_unknown"),
        (WalletAddressClassification::GasAvailable, "gas_available"),
        (
            WalletAddressClassification::TransactionHistory,
            "transaction_history",
        ),
        (WalletAddressClassification::TokenHolding, "token_holding"),
        (WalletAddressClassification::NftHolding, "nft_holding"),
        (
            WalletAddressClassification::ProtocolHolding,
            "protocol_holding",
        ),
        (WalletAddressClassification::ValueDetected, "value_detected"),
        (
            WalletAddressClassification::AssetValueDetected,
            "asset_value_detected",
        ),
        (WalletAddressClassification::StrandedValue, "stranded_value"),
        (
            WalletAddressClassification::ApprovalExposure,
            "approval_exposure",
        ),
        (
            WalletAddressClassification::DormantCandidate,
            "dormant_candidate",
        ),
        (
            WalletAddressClassification::EmptyCandidate,
            "empty_candidate",
        ),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletAssetKind::Native, "native"),
        (WalletAssetKind::Erc20, "erc20"),
        (WalletAssetKind::Erc721, "erc721"),
        (WalletAssetKind::Erc1155, "erc1155"),
        (WalletAssetKind::Nft, "nft"),
        (WalletAssetKind::Approval, "approval"),
        (WalletAssetKind::Defi, "defi"),
        (WalletAssetKind::Airdrop, "airdrop"),
        (WalletAssetKind::Reward, "reward"),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletPlanStepAction::SweepNative, "sweep_native"),
        (WalletPlanStepAction::SweepErc20, "sweep_erc20"),
        (WalletPlanStepAction::SweepNft, "sweep_nft"),
        (
            WalletPlanStepAction::RevokeErc20Approval,
            "revoke_erc20_approval",
        ),
        (
            WalletPlanStepAction::RevokePermit2Allowance,
            "revoke_permit2_allowance",
        ),
        (
            WalletPlanStepAction::RevokeNftOperatorApproval,
            "revoke_nft_operator_approval",
        ),
        (WalletPlanStepAction::RevokeApproval, "revoke_approval"),
        (WalletPlanStepAction::ExitDefiPosition, "exit_defi_position"),
        (WalletPlanStepAction::ClaimReward, "claim_reward"),
        (WalletPlanStepAction::FundGas, "fund_gas"),
        (WalletPlanStepAction::ReviewAsset, "review_asset"),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletPlanStepStatus::ReviewRequired, "review_required"),
        (WalletPlanStepStatus::Blocked, "blocked"),
        (WalletPlanStepStatus::Approved, "approved"),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletSignerStatus::WatchOnly, "watch_only"),
        (WalletSignerStatus::Available, "available"),
        (WalletSignerStatus::Unknown, "unknown"),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletSimulationStatus::Required, "required"),
        (WalletSimulationStatus::NotRun, "not_run"),
        (WalletSimulationStatus::Passed, "passed"),
        (WalletSimulationStatus::Failed, "failed"),
        (WalletSimulationStatus::Unsupported, "unsupported"),
        (WalletSimulationStatus::Blocked, "blocked"),
    ] {
        assert_wire_literal(value, literal);
    }

    for (value, literal) in [
        (WalletPlanStatus::Empty, "empty"),
        (WalletPlanStatus::Blocked, "blocked"),
        (WalletPlanStatus::ReviewRequired, "review_required"),
        (WalletPlanStatus::Approved, "approved"),
    ] {
        assert_wire_literal(value, literal);
    }
}

#[test]
fn wallet_domain_enums_preserve_other_literals() {
    assert_wire_literal(
        WalletAddressActivityState::Other("paused_by_operator".to_string()),
        "paused_by_operator",
    );
    assert_wire_literal(
        WalletAddressClassification::Other("future_classification".to_string()),
        "future_classification",
    );
    assert_wire_literal(
        WalletAssetKind::Other("quantum_bond".to_string()),
        "quantum_bond",
    );
    assert_wire_literal(
        WalletPlanStepAction::Other("mint_soulbound".to_string()),
        "mint_soulbound",
    );
    assert_wire_literal(
        WalletPlanStepStatus::Other("queued_for_review".to_string()),
        "queued_for_review",
    );
    assert_wire_literal(
        WalletSignerStatus::Other("remote_signer".to_string()),
        "remote_signer",
    );
    assert_wire_literal(
        WalletSimulationStatus::Other("deferred".to_string()),
        "deferred",
    );
    assert_wire_literal(
        WalletPlanStatus::Other("partially_approved".to_string()),
        "partially_approved",
    );
}

#[test]
fn wallet_inventory_legacy_json_domains_deserialize_unchanged() {
    #[derive(Debug, Deserialize)]
    struct LegacyFixture {
        inventory: WalletInventoryListResponse,
        plan: ConsolidationPlan,
    }

    let fixture: LegacyFixture = serde_json::from_str(
        r#"{
          "inventory": {
            "jobs": [],
            "addresses": [{
              "id": "addr_legacy_1",
              "wallet_family": "eth-seed",
              "wallet_profile": "ops-seed-mainnet",
              "provider_profile": "ethereum-mainnet-alchemy",
              "chain_id": 1,
              "address": "0xaaaaaaaa11111111222222223333333344444444",
              "derivation_path": "m/44'/60'/0'/0/5",
              "derivation_pattern": "ledger_live",
              "account_index": 0,
              "address_index": 5,
              "activity_state": "funded",
              "native_balance_wei_hex": "0xde0b6b3a7640000",
              "transaction_count": 12,
              "classifications": ["signer_available", "gas_available", "value_detected"],
              "source": "scan",
              "first_seen_at_unix": 1782960000,
              "last_checked_at_unix": 1783046000
            }],
            "holdings": [{
              "id": "hold_legacy_1",
              "wallet_family": "eth-seed",
              "wallet_profile": "ops-seed-mainnet",
              "provider_profile": "ethereum-mainnet-alchemy",
              "chain_id": 1,
              "address": "0xaaaaaaaa11111111222222223333333344444444",
              "derivation_path": "m/44'/60'/0'/0/5",
              "asset_kind": "erc20",
              "asset_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
              "amount_hex": "0x5f5e100",
              "source": "scan",
              "status": "active",
              "first_seen_at_unix": 1782960000,
              "last_checked_at_unix": 1783046000
            }],
            "nft_metadata_cache": []
          },
          "plan": {
            "id": "plan_legacy_1",
            "status": "review_required",
            "destination_address": "0xbbbbbbbb11111111222222223333333344444444",
            "created_at_unix": 1783046000,
            "updated_at_unix": 1783046000,
            "summary": {
              "total_steps": 1,
              "blocked_steps": 0,
              "review_required_steps": 1,
              "approved_steps": 0,
              "executable_steps": 0,
              "value_items": 1
            },
            "policy_violations": [],
            "linkage_findings": [],
            "steps": [{
              "id": "step_legacy_1",
              "action": "sweep_erc20",
              "status": "review_required",
              "wallet_family": "eth-seed",
              "wallet_profile": "ops-seed-mainnet",
              "provider_profile": "ethereum-mainnet-alchemy",
              "chain_id": 1,
              "address": "0xaaaaaaaa11111111222222223333333344444444",
              "derivation_path": "m/44'/60'/0'/0/5",
              "asset_kind": "erc20",
              "asset_address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
              "amount_hex": "0x5f5e100",
              "destination_address": "0xbbbbbbbb11111111222222223333333344444444",
              "signer_status": "available",
              "simulation_status": "required",
              "risk_level": "low",
              "blockers": [],
              "linkage_warnings": [],
              "auto_eligible": false,
              "approved": false
            }]
          }
        }"#,
    )
    .expect("legacy wallet inventory and plan JSON deserializes");

    assert_eq!(
        fixture.inventory.addresses[0].activity_state,
        WalletAddressActivityState::Funded
    );
    assert_eq!(
        fixture.inventory.addresses[0].classifications,
        vec![
            WalletAddressClassification::SignerAvailable,
            WalletAddressClassification::GasAvailable,
            WalletAddressClassification::ValueDetected,
        ]
    );
    assert_eq!(
        fixture.inventory.holdings[0].asset_kind,
        WalletAssetKind::Erc20
    );
    assert_eq!(fixture.plan.status, WalletPlanStatus::ReviewRequired);
    assert_eq!(fixture.plan.chain_id, 1);
    assert_eq!(
        fixture.plan.steps[0].action,
        WalletPlanStepAction::SweepErc20
    );
    assert_eq!(
        fixture.plan.steps[0].simulation_status,
        WalletSimulationStatus::Required
    );
    assert_eq!(fixture.plan.steps[0].sequence, 0);
    assert!(fixture.plan.steps[0].depends_on.is_empty());
}

#[test]
fn session_request_roundtrip() {
    roundtrip(&CapabilitySessionRequest {
        scopes: vec![
            "status:read".to_string(),
            "treasury:read".to_string(),
            "queue:write".to_string(),
        ],
        ttl_secs: Some(900),
    });
}

#[test]
fn session_response_roundtrip() {
    roundtrip(&StatusResponse {
        locked: false,
        initialized: true,
        active_compartment: Some(ActiveCompartment {
            compartment_id: 7,
            compartment_label: "ops-treasury".to_string(),
            api_key_count: 3,
            secret_count: Some(14),
        }),
        unlocked_compartments: vec![UnlockedCompartment {
            id: 7,
            label: "ops-treasury".to_string(),
            threshold: 2,
            passphrase_mode: Some("fixed".to_string()),
        }],
        fido2: None,
    });
}

#[test]
fn profiles_request_roundtrip() {
    roundtrip(&EvmProviderProfileUpsertRequest {
        name: "ethereum-mainnet-alchemy".to_string(),
        provider: EvmProviderRef {
            rpc_url: "https://eth-mainnet.example/rpc".to_string(),
            auth_token_key: Some("providers/alchemy/mainnet/token".to_string()),
            compartment_id: Some(7),
        },
        chain_id: 1,
        max_priority_fee_per_gas_hex: Some("0x59682f00".to_string()),
        max_fee_per_gas_hex: Some("0x12a05f200".to_string()),
        native_gas_limit: Some(21_000),
        erc20_gas_limit: Some(65_000),
        fee_estimation_enabled: Some(false),
    });
}

#[test]
fn profiles_response_roundtrip() {
    roundtrip(&EvmProviderProfileListResponse {
        profiles: vec![EvmProviderProfile {
            name: "ethereum-mainnet-alchemy".to_string(),
            rpc_url: "https://eth-mainnet.example/rpc".to_string(),
            auth_token_key: Some("providers/alchemy/mainnet/token".to_string()),
            compartment_id: 7,
            chain_id: 1,
            max_priority_fee_per_gas_hex: Some("0x59682f00".to_string()),
            max_fee_per_gas_hex: Some("0x12a05f200".to_string()),
            native_gas_limit: Some(21_000),
            erc20_gas_limit: Some(65_000),
            fee_estimation_enabled: false,
        }],
    });
}

#[test]
fn deposits_request_roundtrip() {
    roundtrip(&EthStealthDepositCreateNativeRequest {
        wallet_profile: "ops-stealth-mainnet".to_string(),
        expected_value_wei_hex: Some("0xde0b6b3a7640000".to_string()),
        auto_queue_sweep: Some(true),
        sweep_destination_address: Some("0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string()),
        min_sweep_value_wei_hex: Some("0x2386f26fc10000".to_string()),
        note: Some("client retainer deposit".to_string()),
        ephemeral_private_key_hex: Some(
            "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
        ),
        request_gas: Some(true),
        gas_amount_wei_hex: Some("0xec350c1800".to_string()),
    });
}

#[test]
fn deposits_response_roundtrip() {
    roundtrip(&EthStealthDepositListResponse {
        deposits: vec![EthStealthDeposit {
            id: "dep_2026_0001".to_string(),
            status: "funded".to_string(),
            asset_kind: "native".to_string(),
            wallet_profile: "ops-stealth-mainnet".to_string(),
            chain_id: 1,
            chain_id_assumed: false,
            wallet_compartment_id: 7,
            provider_compartment_id: 7,
            wallet: "0x1111222233334444555566667777888899990000".to_string(),
            short_name: "ops-stealth".to_string(),
            stealth_meta_address: "st:eth:0xabcdef0123456789".to_string(),
            stealth_address: "0x4f4f3e3e2d2d1c1c0b0b9a9a8988787867675656".to_string(),
            ephemeral_public_key_hex:
                "0x020202020202020202020202020202020202020202020202020202020202020202".to_string(),
            view_tag_hex: "0x7f".to_string(),
            stealth_hash_convention: sigillum_core::StealthHashConvention::STANDARD,
            announcement: Some(EthStealthAnnouncementPayload {
                announcer_address: "0x1111222233334444555566667777888899990000".to_string(),
                announce_function: "announce".to_string(),
                scheme_id: 1,
                stealth_address: "0x4f4f3e3e2d2d1c1c0b0b9a9a8988787867675656".to_string(),
                ephemeral_public_key_hex:
                    "0x020202020202020202020202020202020202020202020202020202020202020202"
                        .to_string(),
                metadata_hex: "0x7f".to_string(),
                calldata_hex: "0xabcdef".to_string(),
                value_wei_hex: "0x0".to_string(),
            }),
            token_address: None,
            expected_amount_hex: Some("0xde0b6b3a7640000".to_string()),
            observed_amount_hex: Some("0xde0b6b3a7640000".to_string()),
            observed_native_balance_wei_hex: Some("0xde0b6b3a7640000".to_string()),
            auto_queue_sweep: true,
            sweep_destination_address: Some(
                "0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string(),
            ),
            min_sweep_amount_hex: Some("0x2386f26fc10000".to_string()),
            queue_job_id: Some("job_sweep_0001".to_string()),
            queue_job_state: Some("pending".to_string()),
            note: Some("client retainer deposit".to_string()),
            created_at_unix: 1_783_042_400,
            updated_at_unix: 1_783_046_000,
            last_checked_at_unix: Some(1_783_046_000),
            broadcast_transaction_hash_hex: Some(
                "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ),
            counterparty_id: Some("party_client_alpha".to_string()),
            requested_gas_wei_hex: Some("0xec350c1800".to_string()),
            gas_topup_job_id: Some("job_topup_0001".to_string()),
            gas_topup_job_state: Some("sent".to_string()),
        }],
        pagination: None,
    });
}

#[test]
fn queue_request_roundtrip() {
    roundtrip(&QueueEthStealthNativeSweepRequest {
        wallet_profile: "ops-stealth-mainnet".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0x4f4f3e3e2d2d1c1c0b0b9a9a8988787867675656".to_string(),
            ephemeral_public_key_hex:
                "0x020202020202020202020202020202020202020202020202020202020202020202".to_string(),
            view_tag_hex: Some("0x7f".to_string()),
            stealth_hash_convention: None,
        },
        destination_address: Some("0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string()),
        min_value_wei_hex: Some("0x2386f26fc10000".to_string()),
        gas_limit: Some(42_000),
    });
}

#[test]
fn queue_response_roundtrip() {
    roundtrip(&QueueJobListResponse {
        jobs: vec![QueueJob {
            id: "job_sweep_0001".to_string(),
            state: "retrying".to_string(),
            attempts: 1,
            created_at_unix: 1_783_042_400,
            updated_at_unix: 1_783_046_000,
            next_attempt_after_unix: Some(1_783_046_600),
            payload: QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "ops-stealth-mainnet".to_string(),
                stealth_address: "0x4f4f3e3e2d2d1c1c0b0b9a9a8988787867675656".to_string(),
                ephemeral_public_key_hex:
                    "0x020202020202020202020202020202020202020202020202020202020202020202"
                        .to_string(),
                destination_address: Some("0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string()),
                min_value_wei_hex: Some("0x2386f26fc10000".to_string()),
                gas_limit: Some(42_000),
                view_tag_hex: Some("0x7f".to_string()),
                stealth_hash_convention: None,
            },
            last_error: Some("provider rate limited previous attempt".to_string()),
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }],
        pagination: None,
    });
}

#[test]
fn treasury_request_roundtrip() {
    roundtrip(&TreasuryPolicyUpdateRequest {
        enabled: true,
        allowed_destinations: vec![TreasuryAllowedDestinationInput {
            address: "0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string(),
            label: Some("cold-treasury-safe".to_string()),
        }],
        max_step_native_wei_hex: Some("0x0de0b6b3a7640000".to_string()),
        max_plan_native_wei_hex: Some("0x4563918244f40000".to_string()),
        require_simulation: Some(true),
        allow_raw_digest_signing: Some(false),
        block_cross_party_linkage: Some(true),
        allow_claim_execution: Some(true),
        allow_gas_topups: Some(true),
        max_gas_topup_wei_hex: Some("0x2386f26fc10000".to_string()),
        allow_plan_execution: Some(true),
        allow_sweep_execution: Some(true),
        allow_revoke_execution: Some(true),
        allow_exit_execution: Some(true),
        execution_paused: Some(true),
        max_fee_per_gas_cap_hex: Some("0x59682f00".to_string()),
        simulation_freshness_secs: Some(900),
        hot_floor_wei_hex: Some("0xde0b6b3a7640000".to_string()),
        hot_target_wei_hex: Some("0xde0b6b3a7640000".to_string()),
        hot_overflow_wei_hex: None,
        allow_treasury_automation: None,
    });
}

#[test]
fn treasury_response_roundtrip() {
    roundtrip(&TreasuryPolicyResponse {
        policy: Some(TreasuryPolicy {
            enabled: true,
            allowed_destinations: vec![TreasuryAllowedDestination {
                address: "0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string(),
                label: Some("cold-treasury-safe".to_string()),
            }],
            max_step_native_wei_hex: Some("0x0de0b6b3a7640000".to_string()),
            max_plan_native_wei_hex: Some("0x4563918244f40000".to_string()),
            require_simulation: true,
            allow_raw_digest_signing: false,
            block_cross_party_linkage: true,
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
            hot_floor_wei_hex: "0xde0b6b3a7640000".to_string(),
            hot_target_wei_hex: "0xde0b6b3a7640000".to_string(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1_783_000_000,
            updated_at_unix: 1_783_046_000,
        }),
    });
}

#[test]
fn treasury_policy_gas_topup_defaults_and_roundtrip() {
    let legacy: TreasuryPolicy = serde_json::from_value(serde_json::json!({
        "enabled": true,
        "allowed_destinations": [],
        "max_step_native_wei_hex": null,
        "max_plan_native_wei_hex": null,
        "require_simulation": true,
        "allow_raw_digest_signing": false,
        "block_cross_party_linkage": false,
        "allow_claim_execution": false,
        "simulation_freshness_secs": 900,
        "hot_floor_wei_hex": "0xde0b6b3a7640000",
        "hot_target_wei_hex": "0xde0b6b3a7640000",
        "created_at_unix": 1_783_000_000,
        "updated_at_unix": 1_783_046_000
    }))
    .unwrap();
    assert!(!legacy.allow_gas_topups);
    assert!(legacy.max_gas_topup_wei_hex.is_none());
    assert!(!legacy.allow_plan_execution);
    assert!(!legacy.allow_sweep_execution);
    assert!(!legacy.allow_revoke_execution);
    assert!(!legacy.allow_exit_execution);
    assert!(!legacy.execution_paused);
    assert!(legacy.max_fee_per_gas_cap_hex.is_none());

    roundtrip(&TreasuryPolicy {
        enabled: true,
        allowed_destinations: Vec::new(),
        max_step_native_wei_hex: None,
        max_plan_native_wei_hex: None,
        require_simulation: true,
        allow_raw_digest_signing: false,
        block_cross_party_linkage: false,
        allow_claim_execution: false,
        allow_gas_topups: true,
        max_gas_topup_wei_hex: Some("0x2386f26fc10000".into()),
        allow_plan_execution: true,
        allow_sweep_execution: true,
        allow_revoke_execution: true,
        allow_exit_execution: true,
        execution_paused: true,
        max_fee_per_gas_cap_hex: Some("0x59682f00".into()),
        simulation_freshness_secs: 900,
        hot_floor_wei_hex: "0xde0b6b3a7640000".to_string(),
        hot_target_wei_hex: "0xde0b6b3a7640000".to_string(),
        hot_overflow_wei_hex: None,
        allow_treasury_automation: false,
        created_at_unix: 1_783_000_000,
        updated_at_unix: 1_783_046_000,
    });
}

#[test]
fn receiving_request_roundtrip() {
    roundtrip(&ReceivingDepositTagRequest {
        deposit_id: "dep_2026_0001".to_string(),
        counterparty_id: Some("party_client_alpha".to_string()),
    });
}

#[test]
fn receiving_response_roundtrip() {
    roundtrip(&ReceivingOverviewResponse {
        generated_at_unix: 1_783_046_000,
        include_retired: false,
        groups: vec![ReceivingPartyGroup {
            counterparty: Some(Counterparty {
                id: "party_client_alpha".to_string(),
                name: "Client Alpha".to_string(),
                note: Some("quarterly retainer payer".to_string()),
                sweep_destination_address: Some(
                    "0x8b8b6f6f5e5e4d4d3c3c2b2b1a1a090908080707".to_string(),
                ),
                created_at_unix: 1_782_960_000,
            }),
            item_count: 1,
            native_total_wei_hex: "0xde0b6b3a7640000".to_string(),
            items: vec![ReceivingItem {
                source_type: "hd_allocation".to_string(),
                address: "0x9c9c8b8b7a7a6969585847473636252514140303".to_string(),
                chain_id: 1,
                chain_id_assumed: false,
                derivation_path: Some("m/44'/60'/0'/0/17".to_string()),
                purpose: Some("client-retainer".to_string()),
                label: Some("Client Alpha July retainer".to_string()),
                counterparty_id: Some("party_client_alpha".to_string()),
                linkage_warning: Some("shares configured sweep destination".to_string()),
                balance_native_wei_hex: Some("0xde0b6b3a7640000".to_string()),
                balance_known: true,
                status: "active".to_string(),
                created_at_unix: 1_782_960_000,
            }],
        }],
        totals: ReceivingTotals {
            item_count: 1,
            hd_count: 1,
            stealth_count: 0,
            native_total_wei_hex: "0xde0b6b3a7640000".to_string(),
        },
        coverage: ReceivingCoverage {
            addresses_total: 1,
            addresses_with_known_balance: 1,
            note: "all active receiving addresses refreshed".to_string(),
        },
    });
}

#[test]
fn inventory_request_roundtrip() {
    roundtrip(&WalletInventoryScanRequest {
        wallet_family: Some("eth-seed".to_string()),
        wallet_profile: Some("ops-seed-mainnet".to_string()),
        provider_profile: Some("ethereum-mainnet-alchemy".to_string()),
        all_configured_chains: Some(false),
        derivation_pattern: Some("ledger_live".to_string()),
        account_limit: Some(3),
        watch_addresses: vec![WatchAddressProbe {
            address: "0xaaaaaaaa11111111222222223333333344444444".to_string(),
            label: Some("legacy hardware wallet".to_string()),
        }],
        include_watch_book: Some(true),
        gap_limit: Some(20),
        max_index: Some(250),
        resume_from_latest_checkpoint: Some(true),
        run_async: Some(false),
        partition_providers: Some(true),
        token_addresses: vec![
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            "0xdac17f958d2ee523a2206206994597c13d831ec7".to_string(),
        ],
        block_tag: Some("safe".to_string()),
        probe_token_registry: Some(true),
        discover_erc20_transfers: Some(true),
        token_discovery_from_block: Some("0x11e1a300".to_string()),
        token_discovery_to_block: Some("latest".to_string()),
        token_discovery_limit: Some(500),
        discover_erc20_allowances: Some(true),
        allowance_spender_addresses: vec!["0x1111111254eeb25477b68fb85ed929f73a960582".to_string()],
        allowance_discovery_limit: Some(200),
        discover_permit2_allowances: Some(true),
        permit2_contract_addresses: vec!["0x000000000022d473030f116ddee9f6b43ac78ba3".to_string()],
        permit2_spender_addresses: vec!["0x3f3f2e2e1d1d0c0c0b0b9a9a8988787867675656".to_string()],
        permit2_allowance_limit: Some(200),
        discover_erc721_transfers: Some(true),
        discover_erc1155_transfers: Some(true),
        discover_nft_operator_approvals: Some(true),
        nft_operator_addresses: vec!["0x5f5f4e4e3d3d2c2c1b1b0a0a9999888877776666".to_string()],
        nft_operator_approval_limit: Some(100),
        discover_defi_token_positions: Some(true),
        defi_token_probes: vec![DefiTokenProbe {
            protocol: "aave-v3".to_string(),
            token_address: "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".to_string(),
            protocol_address: Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2".to_string()),
        }],
        defi_position_limit: Some(50),
        discover_claim_candidates: Some(true),
        claim_candidate_probes: vec![ClaimCandidateProbe {
            kind: "reward".to_string(),
            protocol: "safe-rewards".to_string(),
            claimant_address: "0xaaaaaaaa11111111222222223333333344444444".to_string(),
            claim_contract_address: "0x6f6f5e5e4d4d3c3c2b2b1a1a0909080807079696".to_string(),
            asset_address: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            amount_hex: "0x5f5e100".to_string(),
            source_label: "july rewards export".to_string(),
            claim_adapter: Some("safe-rewards-v1".to_string()),
            claim_index_hex: Some("0x2a".to_string()),
            claim_proof: vec!["0xaaaaaaaa".to_string(), "0xbbbbbbbb".to_string()],
        }],
        claim_candidate_limit: Some(25),
        nft_discovery_from_block: Some("0x11e1a300".to_string()),
        nft_discovery_to_block: Some("latest".to_string()),
        nft_discovery_limit: Some(150),
    });
}

#[test]
fn inventory_response_roundtrip() {
    roundtrip(&WalletInventoryListResponse {
        jobs: vec![WalletDiscoveryJob {
            id: "disc_2026_0001".to_string(),
            status: "completed".to_string(),
            source: "manual_scan".to_string(),
            wallet_families: vec!["eth-seed".to_string()],
            wallet_profiles: vec!["ops-seed-mainnet".to_string()],
            provider_profiles: vec!["ethereum-mainnet-alchemy".to_string()],
            chain_ids: vec![1],
            gap_limit: 20,
            max_index: 250,
            addresses_scanned: 18,
            active_addresses: 1,
            holdings_detected: 2,
            checkpoints: vec![WalletDiscoveryCheckpoint {
                wallet_family: "eth-seed".to_string(),
                wallet_profile: "ops-seed-mainnet".to_string(),
                provider_profile: "ethereum-mainnet-alchemy".to_string(),
                derivation_pattern: Some("ledger_live".to_string()),
                account_index: Some(0),
                next_index: 18,
                last_scanned_index: Some(17),
                consecutive_empty: 12,
                completed: true,
                updated_at_unix: 1_783_046_000,
            }],
            block_cursors: vec![WalletDiscoveryBlockCursor {
                address: "0xaaaaaaaa11111111222222223333333344444444".to_string(),
                chain_id: 1,
                topic_family: "erc20-transfer".to_string(),
                last_scanned_block: 18_748_000,
                updated_at_unix: 1_783_046_000,
            }],
            started_at_unix: 1_783_042_400,
            completed_at_unix: Some(1_783_046_000),
            last_error: None,
            partition_providers: Some(true),
            provider_partition_observations: vec![ProviderPartitionObservation {
                provider_profile: "ethereum-mainnet-alchemy".to_string(),
                chain_id: 1,
                addresses_observed: 18,
            }],
        }],
        addresses: vec![WalletInventoryAddress {
            id: "addr_2026_0001".to_string(),
            wallet_family: "eth-seed".to_string(),
            wallet_profile: "ops-seed-mainnet".to_string(),
            provider_profile: "ethereum-mainnet-alchemy".to_string(),
            chain_id: 1,
            address: "0xaaaaaaaa11111111222222223333333344444444".to_string(),
            derivation_path: "m/44'/60'/0'/0/5".to_string(),
            derivation_pattern: Some("ledger_live".to_string()),
            account_index: Some(0),
            address_index: 5,
            activity_state: WalletAddressActivityState::Funded,
            native_balance_wei_hex: "0xde0b6b3a7640000".to_string(),
            transaction_count: 12,
            last_activity_block: None,
            classifications: vec![
                WalletAddressClassification::Other("signer".to_string()),
                WalletAddressClassification::Other("treasury".to_string()),
            ],
            source: "scan".to_string(),
            first_seen_at_unix: 1_782_960_000,
            last_checked_at_unix: 1_783_046_000,
        }],
        holdings: vec![WalletAssetHolding {
            id: "hold_2026_0001".to_string(),
            wallet_family: "eth-seed".to_string(),
            wallet_profile: "ops-seed-mainnet".to_string(),
            provider_profile: "ethereum-mainnet-alchemy".to_string(),
            chain_id: 1,
            address: "0xaaaaaaaa11111111222222223333333344444444".to_string(),
            derivation_path: "m/44'/60'/0'/0/5".to_string(),
            asset_kind: WalletAssetKind::Erc20,
            asset_address: Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string()),
            token_id_hex: None,
            counterparty_address: Some("0x1111111254eeb25477b68fb85ed929f73a960582".to_string()),
            protocol_address: Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2".to_string()),
            claim_adapter: Some("safe-rewards-v1".to_string()),
            claim_index_hex: Some("0x2a".to_string()),
            claim_proof: vec!["0xaaaaaaaa".to_string(), "0xbbbbbbbb".to_string()],
            metadata_uri: Some("ipfs://bafymetadata".to_string()),
            metadata_name: Some("Aave USDC position".to_string()),
            spam_label: Some("trusted".to_string()),
            amount_hex: "0x5f5e100".to_string(),
            source: "scan".to_string(),
            status: "active".to_string(),
            first_seen_at_unix: 1_782_960_000,
            last_checked_at_unix: 1_783_046_000,
        }],
        nft_metadata_cache: vec![NftMetadataCacheEntry {
            chain_id: 1,
            contract_address: "0x7f7f6e6e5d5d4c4c3b3b2a2a1919080807070606".to_string(),
            token_id_hex: "0x2a".to_string(),
            metadata_uri: Some("ipfs://bafynftmetadata".to_string()),
            name: Some("Operations Safe Badge".to_string()),
            spam_label: "trusted".to_string(),
            spam_reasons: vec!["operator_reviewed".to_string()],
            fetched_at_unix: Some(1_783_046_000),
            fetched_uri: Some("http://127.0.0.1:1/ipfs/bafynftmetadata".to_string()),
            content_sha256: Some("aa".repeat(32)),
            fetch_skipped_reason: None,
            updated_at_unix: 1_783_046_000,
        }],
        pagination: None,
    });
}

#[test]
fn unknown_fields_are_tolerated_for_request_and_response() {
    let expected_request = ReceivingDepositTagRequest {
        deposit_id: "dep_2026_0001".to_string(),
        counterparty_id: Some("party_client_alpha".to_string()),
    };
    let request: ReceivingDepositTagRequest = serde_json::from_value(serde_json::json!({
        "deposit_id": "dep_2026_0001",
        "counterparty_id": "party_client_alpha",
        "__future_field": 123
    }))
    .expect("ReceivingDepositTagRequest should ignore unknown fields");
    assert_eq!(request, expected_request);

    let expected_response = ErrorResponse {
        code: sigillum_api::error_codes::UNKNOWN.to_string(),
        error: "treasury policy violation".to_string(),
        action: Some("review_treasury_policy".to_string()),
        fields: None,
    };
    let response: ErrorResponse = serde_json::from_value(serde_json::json!({
        "error": "treasury policy violation",
        "action": "review_treasury_policy",
        "__future_field": 123
    }))
    .expect("ErrorResponse should ignore unknown fields");
    assert_eq!(response, expected_response);
}

#[test]
fn error_response_code_and_fields_roundtrip() {
    let expected = ErrorResponse {
        code: sigillum_api::error_codes::VALIDATION_FAILED.to_string(),
        error: "name exceeds maximum length of 256 bytes (got 300 bytes)".to_string(),
        action: None,
        fields: Some(vec![sigillum_api::response::FieldError {
            field: "name".to_string(),
            message: "name exceeds maximum length of 256 bytes (got 300 bytes)".to_string(),
        }]),
    };
    let value = serde_json::to_value(&expected).expect("ErrorResponse should serialize");
    assert_eq!(value["code"], "validation_failed");
    assert_eq!(value["fields"][0]["field"], "name");

    let parsed: ErrorResponse =
        serde_json::from_value(value).expect("ErrorResponse should deserialize");
    assert_eq!(parsed, expected);

    // Forward-compat: payloads from a newer daemon carrying extra fields parse.
    let future: ErrorResponse = serde_json::from_value(serde_json::json!({
        "code": "some_future_code",
        "error": "new failure mode",
        "fields": [{"field": "name", "message": "bad", "extra": true}],
        "__future_field": 123
    }))
    .expect("ErrorResponse should tolerate newer fields");
    assert_eq!(future.code, "some_future_code");
}
