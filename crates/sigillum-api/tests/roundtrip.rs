use serde::{Serialize, de::DeserializeOwned};
use sigillum_api::{
    ActiveCompartment, CapabilitySessionRequest, ClaimCandidateProbe, Counterparty, DefiTokenProbe,
    ErrorResponse, EthStealthAnnouncementPayload, EthStealthDeposit,
    EthStealthDepositCreateNativeRequest, EthStealthDepositListResponse, EvmProviderProfile,
    EvmProviderProfileListResponse, EvmProviderProfileUpsertRequest, EvmProviderRef,
    NftMetadataCacheEntry, QueueEthStealthNativeSweepRequest, QueueJob, QueueJobListResponse,
    QueueJobPayload, ReceivingCoverage, ReceivingDepositTagRequest, ReceivingItem,
    ReceivingOverviewResponse, ReceivingPartyGroup, ReceivingTotals, StatusResponse,
    StealthPaymentRef, TreasuryAllowedDestination, TreasuryAllowedDestinationInput, TreasuryPolicy,
    TreasuryPolicyResponse, TreasuryPolicyUpdateRequest, UnlockedCompartment, WalletAssetHolding,
    WalletDiscoveryCheckpoint, WalletDiscoveryJob, WalletInventoryAddress,
    WalletInventoryListResponse, WalletInventoryScanRequest, WatchAddressProbe,
};
use std::fmt::Debug;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let json = serde_json::to_string(value).expect("serialize fixture to JSON");
    let decoded: T = serde_json::from_str(&json).expect("deserialize fixture from JSON");
    assert_eq!(&decoded, value, "JSON roundtrip changed value: {json}");
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
        }],
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
            },
            last_error: Some("provider rate limited previous attempt".to_string()),
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
        }],
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
            created_at_unix: 1_783_000_000,
            updated_at_unix: 1_783_046_000,
        }),
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
        token_addresses: vec![
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            "0xdac17f958d2ee523a2206206994597c13d831ec7".to_string(),
        ],
        block_tag: Some("safe".to_string()),
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
            started_at_unix: 1_783_042_400,
            completed_at_unix: Some(1_783_046_000),
            last_error: None,
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
            activity_state: "funded".to_string(),
            native_balance_wei_hex: "0xde0b6b3a7640000".to_string(),
            transaction_count: 12,
            classifications: vec!["signer".to_string(), "treasury".to_string()],
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
            asset_kind: "erc20".to_string(),
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
            updated_at_unix: 1_783_046_000,
        }],
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
        error: "treasury policy violation".to_string(),
        action: Some("review_treasury_policy".to_string()),
    };
    let response: ErrorResponse = serde_json::from_value(serde_json::json!({
        "error": "treasury policy violation",
        "action": "review_treasury_policy",
        "__future_field": 123
    }))
    .expect("ErrorResponse should ignore unknown fields");
    assert_eq!(response, expected_response);
}
