use super::*;

fn roundtrip_test<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
    value: T,
) {
    let json = serde_json::to_string(&value).unwrap();
    let deserialized: T = serde_json::from_str(&json).unwrap();
    assert_eq!(value, deserialized, "Roundtrip failed for JSON: {json}");
}

#[test]
fn test_error_response_roundtrip() {
    let resp = ErrorResponse {
        error: "Something went wrong".to_string(),
        action: None,
    };
    roundtrip_test(resp);
}

#[test]
fn test_active_compartment_roundtrip() {
    let comp = ActiveCompartment {
        compartment_id: 1,
        compartment_label: "vault1".to_string(),
        api_key_count: 5,
        secret_count: Some(10),
    };
    roundtrip_test(comp);
}

#[test]
fn test_active_compartment_no_secret_count() {
    let comp = ActiveCompartment {
        compartment_id: 2,
        compartment_label: "vault2".to_string(),
        api_key_count: 3,
        secret_count: None,
    };
    roundtrip_test(comp);
}

#[test]
fn test_unlocked_compartment_roundtrip() {
    let comp = UnlockedCompartment {
        id: 1,
        label: "vault_unlocked".to_string(),
        threshold: 2,
        passphrase_mode: Some("FIXED".to_string()),
    };
    roundtrip_test(comp);
}

#[test]
fn test_status_response_full() {
    let resp = StatusResponse {
        locked: false,
        initialized: true,
        active_compartment: Some(ActiveCompartment {
            compartment_id: 1,
            compartment_label: "active".to_string(),
            api_key_count: 2,
            secret_count: Some(5),
        }),
        unlocked_compartments: vec![UnlockedCompartment {
            id: 1,
            label: "vault1".to_string(),
            threshold: 1,
            passphrase_mode: None,
        }],
        fido2: Some(Fido2StatusResponse {
            enabled: true,
            key_count: 2,
        }),
    };
    roundtrip_test(resp);
}

#[test]
fn test_status_response_locked() {
    let resp = StatusResponse {
        locked: true,
        initialized: true,
        active_compartment: None,
        unlocked_compartments: vec![],
        fido2: None,
    };
    roundtrip_test(resp);
}

#[test]
fn test_lock_response_roundtrip() {
    let resp = LockResponse {
        status: "locked".to_string(),
        message: "Vault is now locked".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_unlock_response_roundtrip() {
    let resp = UnlockResponse {
        status: "unlocked".to_string(),
        method: "fido2".to_string(),
        cascading: Some(true),
        session_token: "token123".to_string(),
        unlocked_compartments: vec![UnlockedCompartment {
            id: 1,
            label: "vault1".to_string(),
            threshold: 1,
            passphrase_mode: None,
        }],
        active_compartment_id: Some(1),
    };
    roundtrip_test(resp);
}

#[test]
fn test_generic_status_response_roundtrip() {
    let resp = GenericStatusResponse {
        status: "success".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_key_list_response_roundtrip() {
    let resp = KeyListResponse {
        keys: vec!["key1".to_string(), "key2".to_string(), "key3".to_string()],
    };
    roundtrip_test(resp);
}

#[test]
fn test_key_value_response_roundtrip() {
    let resp = KeyValueResponse {
        key: "mykey".to_string(),
        value: "myvalue".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_key_mutation_response_roundtrip() {
    let resp = KeyMutationResponse {
        status: "created".to_string(),
        key: "newkey".to_string(),
        tier: Some(2),
    };
    roundtrip_test(resp);
}

#[test]
fn test_push_response_roundtrip() {
    let resp = PushResponse {
        status: "success".to_string(),
        from: 1,
        to: 2,
        key: "transferred_key".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_compartment_info_roundtrip() {
    let info = CompartmentInfo {
        id: 1,
        label: "vault1".to_string(),
        threshold: 2,
        passphrase_mode: Some("FIXED".to_string()),
        is_active: true,
    };
    roundtrip_test(info);
}

#[test]
fn test_compartment_list_response_roundtrip() {
    let resp = CompartmentListResponse {
        compartments: vec![
            CompartmentInfo {
                id: 1,
                label: "vault1".to_string(),
                threshold: 1,
                passphrase_mode: None,
                is_active: true,
            },
            CompartmentInfo {
                id: 2,
                label: "vault2".to_string(),
                threshold: 2,
                passphrase_mode: Some("EPHEMERAL".to_string()),
                is_active: false,
            },
        ],
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_setup_response_roundtrip() {
    let resp = Fido2SetupResponse {
        status: "registered".to_string(),
        is_first_key: true,
        total_keys: 1,
        compartments: 2,
        unlocked: true,
        session_token: "new_session".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_transit_encrypt_response_roundtrip() {
    let resp = TransitEncryptResponse {
        key: "encryption_key".to_string(),
        nonce_hex: "0123456789abcdef".to_string(),
        ciphertext_hex: "fedcba9876543210".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_transit_decrypt_response_roundtrip() {
    let resp = TransitDecryptResponse {
        key: "encryption_key".to_string(),
        plaintext_hex: "48656c6c6f".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_transit_hmac_response_roundtrip() {
    let resp = TransitHmacResponse {
        key: "hmac_key".to_string(),
        digest_hex: "abcd1234".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_meta_address_response_roundtrip() {
    let resp = EthStealthMetaAddressResponse {
        wallet: "0xwallet".to_string(),
        short_name: "my_wallet".to_string(),
        scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
        stealth_meta_address: "st:0x...".to_string(),
        spending_public_key_hex: "0xspend".to_string(),
        viewing_public_key_hex: "0xview".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_generate_response_roundtrip() {
    let resp = EthStealthGenerateResponse {
        short_name: "wallet1".to_string(),
        scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
        stealth_meta_address: "st:0x...".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        view_tag_hex: "0xaa".to_string(),
        announcement: Some(EthStealthAnnouncementPayload {
            announcer_address: "0x55649e01b5df198d18d95b5cc5051630cfd45564".to_string(),
            announce_function: "announce(uint256,address,bytes,bytes)".to_string(),
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            metadata_hex: "aa".to_string(),
            calldata_hex: "0xcalldata".to_string(),
            value_wei_hex: "0x0".to_string(),
        }),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_check_response_roundtrip() {
    let resp = EthStealthCheckResponse {
        wallet: "0xwallet".to_string(),
        matches: true,
        derived_stealth_address: "0xderived".to_string(),
        view_tag_hex: "0xaa".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_sign_response_roundtrip() {
    let resp = EthStealthSignResponse {
        wallet: "0xwallet".to_string(),
        stealth_address: "0xstealth".to_string(),
        signature_hex: "0xsig".to_string(),
        recovery_id: 27,
        view_tag_hex: "0xaa".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_signed_transaction_response_roundtrip() {
    let resp = EthSignedTransactionResponse {
        wallet: "0xwallet".to_string(),
        kind: "native_transfer".to_string(),
        chain_id: 1,
        nonce: 5,
        from_address: "0xfrom".to_string(),
        to_address: "0xto".to_string(),
        value_hex: "0x100".to_string(),
        data_hex: "0x".to_string(),
        raw_transaction_hex: "0xraw".to_string(),
        transaction_hash_hex: "0xhash".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_evm_rpc_nonce_response_roundtrip() {
    let resp = EvmRpcNonceResponse {
        address: "0xaddr".to_string(),
        nonce: 42,
        block_tag: "latest".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_evm_rpc_balance_response_roundtrip() {
    let resp = EvmRpcBalanceResponse {
        address: "0xaddr".to_string(),
        balance_wei_hex: "0x1000".to_string(),
        block_tag: "latest".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_evm_rpc_erc20_balance_response_roundtrip() {
    let resp = EvmRpcErc20BalanceResponse {
        token_address: "0xtoken".to_string(),
        owner_address: "0xowner".to_string(),
        amount_hex: "0x500".to_string(),
        block_tag: "latest".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_evm_rpc_broadcast_response_roundtrip() {
    let resp = EvmRpcBroadcastResponse {
        transaction_hash_hex: "0xtxhash".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_send_response_with_broadcast() {
    let resp = EthStealthSendResponse {
        wallet: "0xwallet".to_string(),
        kind: "native_transfer".to_string(),
        chain_id: 1,
        nonce: 5,
        from_address: "0xfrom".to_string(),
        to_address: "0xto".to_string(),
        value_hex: "0x100".to_string(),
        data_hex: "0x".to_string(),
        raw_transaction_hex: "0xraw".to_string(),
        transaction_hash_hex: "0xhash".to_string(),
        broadcast: true,
        broadcast_transaction_hash_hex: Some("0xbcast".to_string()),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_stealth_send_response_no_broadcast() {
    let resp = EthStealthSendResponse {
        wallet: "0xwallet".to_string(),
        kind: "native_transfer".to_string(),
        chain_id: 1,
        nonce: 5,
        from_address: "0xfrom".to_string(),
        to_address: "0xto".to_string(),
        value_hex: "0x100".to_string(),
        data_hex: "0x".to_string(),
        raw_transaction_hex: "0xraw".to_string(),
        transaction_hash_hex: "0xhash".to_string(),
        broadcast: false,
        broadcast_transaction_hash_hex: None,
    };
    roundtrip_test(resp);
}

#[test]
fn test_evm_provider_profile_roundtrip() {
    let profile = EvmProviderProfile {
        name: "mainnet".to_string(),
        rpc_url: "https://eth.example.com".to_string(),
        auth_token_key: Some("key123".to_string()),
        compartment_id: 1,
        chain_id: 1,
        max_priority_fee_per_gas_hex: Some("0x3b9aca00".to_string()),
        max_fee_per_gas_hex: Some("0x5f5e100".to_string()),
        native_gas_limit: Some(21000),
        erc20_gas_limit: Some(65000),
        fee_estimation_enabled: false,
    };
    roundtrip_test(profile);
}

#[test]
fn test_eth_stealth_wallet_profile_roundtrip() {
    let profile = EthStealthWalletProfile {
        name: "my_wallet".to_string(),
        wallet: "0xwallet".to_string(),
        short_name: "wallet1".to_string(),
        provider_profile: "mainnet".to_string(),
        compartment_id: 1,
        chain_id: Some(1),
        default_destination_address: Some("0xdest".to_string()),
        execution_enabled: false,
    };
    roundtrip_test(profile);
}

#[test]
fn test_eth_stealth_wallet_profile_list_response_roundtrip() {
    let resp = EthStealthWalletProfileListResponse {
        profiles: vec![
            EthStealthWalletProfile {
                name: "wallet1".to_string(),
                wallet: "0xwallet1".to_string(),
                short_name: "w1".to_string(),
                provider_profile: "mainnet".to_string(),
                compartment_id: 1,
                chain_id: Some(1),
                default_destination_address: None,
                execution_enabled: false,
            },
            EthStealthWalletProfile {
                name: "wallet2".to_string(),
                wallet: "0xwallet2".to_string(),
                short_name: "w2".to_string(),
                provider_profile: "testnet".to_string(),
                compartment_id: 2,
                chain_id: Some(5),
                default_destination_address: Some("0xdest2".to_string()),
                execution_enabled: true,
            },
        ],
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_xpub_wallet_profile_roundtrip() {
    let profile = EthXpubWalletProfile {
        name: "receive_tree".to_string(),
        project_account: 9,
        provider_profile: "mainnet".to_string(),
        compartment_id: 1,
        chain_id: Some(1),
        external_receive_xpub: Some("xpub-imported".to_string()),
        external_receive_path: Some("m/44'/60'/9'/1".to_string()),
        external_account_xpub: None,
        external_account_path: None,
        default_destination_address: Some("0xdest".to_string()),
        execution_enabled: false,
    };
    roundtrip_test(profile);
}

#[test]
fn test_eth_xpub_wallet_profile_list_response_roundtrip() {
    let resp = EthXpubWalletProfileListResponse {
        profiles: vec![
            EthXpubWalletProfile {
                name: "receive_tree".to_string(),
                project_account: 0,
                provider_profile: "mainnet".to_string(),
                compartment_id: 1,
                chain_id: Some(1),
                external_receive_xpub: None,
                external_receive_path: None,
                external_account_xpub: None,
                external_account_path: None,
                default_destination_address: None,
                execution_enabled: false,
            },
            EthXpubWalletProfile {
                name: "project_b".to_string(),
                project_account: 15,
                provider_profile: "testnet".to_string(),
                compartment_id: 2,
                chain_id: Some(5),
                external_receive_xpub: None,
                external_receive_path: None,
                external_account_xpub: Some("xpub-account".to_string()),
                external_account_path: Some("m/44'/60'/15'".to_string()),
                default_destination_address: Some("0xdest2".to_string()),
                execution_enabled: true,
            },
        ],
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_seed_wallet_profile_roundtrip() {
    let profile = EthSeedWalletProfile {
        name: "treasury_seed".to_string(),
        label: Some("Treasury seed wallet".to_string()),
        project_account: 0,
        provider_profile: "mainnet".to_string(),
        compartment_id: 1,
        chain_id: Some(1),
        word_count: 12,
        mnemonic_secret_key: "wallet.seed.treasury_seed.mnemonic".to_string(),
        account_path: "m/44'/60'/0'".to_string(),
        receive_path: "m/44'/60'/0'/0".to_string(),
        receive_xpub: "xpub661MyMwAqRbcFexample".to_string(),
        first_receive_address: "0x1111111111111111111111111111111111111111".to_string(),
        default_destination_address: Some("0xdest".to_string()),
        control_xpub: Some("xpub661MyMwAqRbcFcontrol".to_string()),
        sponsor_address: Some("0x2222222222222222222222222222222222222222".to_string()),
        hot_address: Some("0x3333333333333333333333333333333333333333".to_string()),
        treasury_address: Some("0x4444444444444444444444444444444444444444".to_string()),
        execution_enabled: false,
    };
    roundtrip_test(profile);
}

#[test]
fn test_eth_seed_wallet_profile_list_response_roundtrip() {
    let resp = EthSeedWalletProfileListResponse {
        profiles: vec![EthSeedWalletProfile {
            name: "wallet_12".to_string(),
            label: None,
            project_account: 0,
            provider_profile: "mainnet".to_string(),
            compartment_id: 1,
            chain_id: Some(1),
            word_count: 12,
            mnemonic_secret_key: "wallet.seed.wallet_12.mnemonic".to_string(),
            account_path: "m/44'/60'/0'".to_string(),
            receive_path: "m/44'/60'/0'/0".to_string(),
            receive_xpub: "xpub661MyMwAqRbcFexample".to_string(),
            first_receive_address: "0x1111111111111111111111111111111111111111".to_string(),
            default_destination_address: None,
            control_xpub: None,
            sponsor_address: None,
            hot_address: None,
            treasury_address: None,
            execution_enabled: false,
        }],
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_seed_wallet_create_response_roundtrip() {
    let resp = EthSeedWalletCreateResponse {
        status: "created".to_string(),
        mnemonic: "legal winner thank year wave sausage worth useful legal winner thank yellow"
            .to_string(),
        profile: EthSeedWalletProfile {
            name: "fresh_seed".to_string(),
            label: Some("Generated treasury wallet".to_string()),
            project_account: 0,
            provider_profile: "mainnet".to_string(),
            compartment_id: 1,
            chain_id: Some(1),
            word_count: 12,
            mnemonic_secret_key: "wallet.seed.fresh_seed.mnemonic".to_string(),
            account_path: "m/44'/60'/0'".to_string(),
            receive_path: "m/44'/60'/0'/0".to_string(),
            receive_xpub: "xpub661MyMwAqRbcFexample".to_string(),
            first_receive_address: "0x1111111111111111111111111111111111111111".to_string(),
            default_destination_address: None,
            control_xpub: Some("xpub661MyMwAqRbcFcontrol".to_string()),
            sponsor_address: Some("0x2222222222222222222222222222222222222222".to_string()),
            hot_address: Some("0x3333333333333333333333333333333333333333".to_string()),
            treasury_address: Some("0x4444444444444444444444444444444444444444".to_string()),
            execution_enabled: false,
        },
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_xpub_export_response_roundtrip() {
    let resp = EthXpubExportResponse {
        wallet_profile: "receive_tree".to_string(),
        project_account: 9,
        account_path: "m/44'/60'/9'".to_string(),
        receive_path: "m/44'/60'/9'/0".to_string(),
        receive_xpub: "xpub661MyMwAqRbcFexample".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_eth_xpub_address_response_roundtrip() {
    let resp = EthXpubAddressResponse {
        index: 4,
        address: "0x1111111111111111111111111111111111111111".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_wallet_inventory_scan_response_roundtrip() {
    let address = WalletInventoryAddress {
        id: "addr_1".to_string(),
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        provider_profile: "mainnet".to_string(),
        chain_id: 1,
        address: "0x1111111111111111111111111111111111111111".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        derivation_pattern: Some("project".to_string()),
        account_index: Some(0),
        address_index: 0,
        activity_state: WalletAddressActivityState::Funded,
        native_balance_wei_hex: "0x1".to_string(),
        transaction_count: 1,
        last_activity_block: None,
        classifications: vec![
            WalletAddressClassification::SignerAvailable,
            WalletAddressClassification::GasAvailable,
        ],
        source: "local-rpc".to_string(),
        first_seen_at_unix: 1,
        last_checked_at_unix: 2,
    };
    let holding = WalletAssetHolding {
        id: "holding_1".to_string(),
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        provider_profile: "mainnet".to_string(),
        chain_id: 1,
        address: address.address.clone(),
        derivation_path: address.derivation_path.clone(),
        asset_kind: WalletAssetKind::Native,
        asset_address: None,
        token_id_hex: None,
        counterparty_address: None,
        protocol_address: None,
        claim_adapter: None,
        claim_index_hex: None,
        claim_proof: Vec::new(),
        metadata_uri: None,
        metadata_name: None,
        spam_label: None,
        amount_hex: "0x1".to_string(),
        source: "local-rpc".to_string(),
        status: "detected".to_string(),
        first_seen_at_unix: 1,
        last_checked_at_unix: 2,
    };
    let job = WalletDiscoveryJob {
        id: "job_1".to_string(),
        status: "completed".to_string(),
        source: "local-rpc".to_string(),
        wallet_families: vec!["eth-seed".to_string()],
        wallet_profiles: vec!["seed-main".to_string()],
        provider_profiles: vec!["mainnet".to_string()],
        chain_ids: vec![1],
        gap_limit: 20,
        max_index: 200,
        addresses_scanned: 1,
        active_addresses: 1,
        holdings_detected: 1,
        checkpoints: Vec::new(),
        block_cursors: Vec::new(),
        started_at_unix: 1,
        completed_at_unix: Some(2),
        last_error: None,
        scan_request: None,
        resumed_from_job_id: None,
    };
    roundtrip_test(WalletInventoryScanResponse {
        job,
        addresses: vec![address],
        holdings: vec![holding],
    });
}

#[test]
fn test_nft_metadata_cache_entry_backcompat_and_provenance_fields() {
    let legacy: NftMetadataCacheEntry = serde_json::from_value(serde_json::json!({
        "chain_id": 1,
        "contract_address": "0xdead00000000000000000000000000000000dead",
        "token_id_hex": "0x01",
        "spam_label": "unverified_nft_metadata",
        "updated_at_unix": 2
    }))
    .unwrap();

    assert!(legacy.spam_reasons.is_empty());
    assert!(legacy.fetched_at_unix.is_none());
    assert!(legacy.fetched_uri.is_none());
    assert!(legacy.content_sha256.is_none());
    assert!(legacy.fetch_skipped_reason.is_none());

    let entry = NftMetadataCacheEntry {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
        token_id_hex: "0x01".to_string(),
        metadata_uri: Some("ipfs://bafynftmetadata".to_string()),
        name: Some("Localhost Test NFT".to_string()),
        spam_label: "trusted".to_string(),
        spam_reasons: vec!["operator_opt_in".to_string()],
        fetched_at_unix: Some(3),
        fetched_uri: Some("http://127.0.0.1:1/ipfs/bafynftmetadata".to_string()),
        content_sha256: Some("aa".repeat(32)),
        fetch_skipped_reason: Some("already_cached".to_string()),
        updated_at_unix: 4,
    };
    let json = serde_json::to_value(&entry).unwrap();

    assert_eq!(json["spam_reasons"], serde_json::json!(["operator_opt_in"]));
    assert_eq!(json["fetched_at_unix"], serde_json::json!(3));
    assert_eq!(
        json["fetched_uri"],
        serde_json::json!("http://127.0.0.1:1/ipfs/bafynftmetadata")
    );
    assert_eq!(json["content_sha256"], serde_json::json!("aa".repeat(32)));
    assert_eq!(
        json["fetch_skipped_reason"],
        serde_json::json!("already_cached")
    );
    roundtrip_test(entry);
}

#[test]
fn test_nft_metadata_responses_roundtrip() {
    let opt_in = NftMetadataCollectionOptIn {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
        enabled: true,
        created_at_unix: 1,
        updated_at_unix: 2,
    };
    roundtrip_test(NftMetadataOptInListResponse {
        opt_ins: vec![opt_in.clone()],
        ipfs_gateway_url: Some("http://127.0.0.1:1/ipfs/".to_string()),
    });
    roundtrip_test(NftMetadataOptInMutationResponse {
        status: "upserted".to_string(),
        opt_in,
    });
    roundtrip_test(NftMetadataSettingsResponse {
        status: "updated".to_string(),
        ipfs_gateway_url: Some("http://127.0.0.1:1/ipfs/".to_string()),
    });
    roundtrip_test(NftMetadataFetchResponse {
        fetched: 0,
        skipped: vec![NftMetadataFetchSkip {
            chain_id: 1,
            contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
            token_id_hex: Some("0x01".to_string()),
            reason: "collection_not_opted_in".to_string(),
        }],
        entries: Vec::new(),
    });
}

#[test]
fn test_wallet_inventory_address_last_activity_block_defaults_and_roundtrips() {
    let legacy = serde_json::json!({
        "id": "addr_legacy",
        "wallet_family": "eth-seed",
        "wallet_profile": "seed-main",
        "provider_profile": "mainnet",
        "chain_id": 1,
        "address": "0x1111111111111111111111111111111111111111",
        "derivation_path": "m/44'/60'/0'/0/0",
        "address_index": 0,
        "activity_state": "funded",
        "native_balance_wei_hex": "0x1",
        "transaction_count": 1,
        "source": "local-rpc",
        "first_seen_at_unix": 1,
        "last_checked_at_unix": 2
    });
    let decoded: WalletInventoryAddress = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.last_activity_block, None);

    let with_activity = serde_json::json!({
        "id": "addr_active",
        "wallet_family": "eth-seed",
        "wallet_profile": "seed-main",
        "provider_profile": "mainnet",
        "chain_id": 1,
        "address": "0x1111111111111111111111111111111111111111",
        "derivation_path": "m/44'/60'/0'/0/0",
        "address_index": 0,
        "activity_state": "funded",
        "native_balance_wei_hex": "0x1",
        "transaction_count": 1,
        "last_activity_block": 12345,
        "source": "local-rpc",
        "first_seen_at_unix": 1,
        "last_checked_at_unix": 2
    });
    let decoded: WalletInventoryAddress = serde_json::from_value(with_activity).unwrap();
    assert_eq!(decoded.last_activity_block, Some(12345));
    roundtrip_test(decoded);
}

#[test]
fn test_watch_address_book_responses_roundtrip() {
    let entry = WatchAddressBookEntry {
        id: "watch_1".to_string(),
        address: "0x7777777777777777777777777777777777777777".to_string(),
        label: "old-ledger".to_string(),
        tags: vec!["client".to_string(), "hardware".to_string()],
        source: "operator".to_string(),
        enabled: true,
        created_at_unix: 1,
        updated_at_unix: 2,
    };

    roundtrip_test(WatchAddressBookListResponse {
        entries: vec![entry.clone()],
    });
    roundtrip_test(WatchAddressBookMutationResponse {
        status: "saved".to_string(),
        entry,
    });
}

#[test]
fn test_wallet_operations_response_roundtrips() {
    let chain = ChainProfile {
        name: "base".to_string(),
        chain_family: "evm".to_string(),
        chain_id: Some(8453),
        provider_profile: Some("base-mainnet".to_string()),
        native_symbol: "ETH".to_string(),
        native_decimals: 18,
        finality_blocks: 12,
        dormancy_block_window: DEFAULT_DORMANCY_BLOCK_WINDOW,
        permit2_address: Some("0x000000000022d473030f116ddee9f6b43ac78ba3".to_string()),
        uniswap_v2_router_address: Some("0xdead100e2000000000000000000000000000cccc".to_string()),
        explorer_url: Some("https://basescan.org".to_string()),
        capabilities: vec!["native".to_string(), "erc20".to_string()],
        enabled: true,
        source: "operator".to_string(),
        builtin: false,
        created_at_unix: 1,
        updated_at_unix: 2,
    };
    roundtrip_test(ChainProfileListResponse {
        profiles: vec![chain.clone()],
    });
    roundtrip_test(ChainProfileMutationResponse {
        status: "upserted".to_string(),
        profile: chain,
    });

    let legacy = serde_json::json!({
        "name": "base",
        "chain_family": "evm",
        "chain_id": 8453,
        "native_symbol": "ETH",
        "native_decimals": 18,
        "finality_blocks": 12,
        "enabled": true,
        "source": "operator",
        "builtin": false,
        "created_at_unix": 1,
        "updated_at_unix": 2
    });
    let decoded: ChainProfile = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.dormancy_block_window, DEFAULT_DORMANCY_BLOCK_WINDOW);

    let finding = RiskFinding {
        id: "risk_1".to_string(),
        category: "stranded_value".to_string(),
        risk_level: "medium".to_string(),
        status: "open".to_string(),
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        provider_profile: "mainnet".to_string(),
        chain_id: 1,
        address: "0x1111111111111111111111111111111111111111".to_string(),
        subject_type: "erc20".to_string(),
        subject: "0xtoken".to_string(),
        source: "local-risk-engine".to_string(),
        recommendation: "Fund gas before sweeping.".to_string(),
        evidence: vec!["positive token balance".to_string()],
        first_seen_at_unix: 1,
        last_checked_at_unix: 2,
    };
    roundtrip_test(RiskFindingListResponse {
        findings: vec![finding],
    });
    let catalog_entry = RiskCatalogEntry {
        address: "0x4444444444444444444444444444444444444444".to_string(),
        label: "Known router".to_string(),
        risk_level: "trusted".to_string(),
        source: "operator".to_string(),
        notes: vec!["Operator-approved spender".to_string()],
        created_at_unix: 1,
        updated_at_unix: 2,
    };
    roundtrip_test(RiskCatalogListResponse {
        entries: vec![catalog_entry.clone()],
    });
    roundtrip_test(RiskCatalogMutationResponse {
        status: "upserted".to_string(),
        entry: catalog_entry,
    });

    let registry_entry = TokenRegistryEntry {
        chain_id: 1,
        address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        symbol: "AAA".to_string(),
        decimals: 18,
    };
    let registry_list = TokenRegistryList {
        id: "registry_1".to_string(),
        name: "core-list".to_string(),
        compartment_id: 0,
        source: "pasted-json".to_string(),
        entries: vec![registry_entry],
        created_at_unix: 1,
        updated_at_unix: 2,
    };
    roundtrip_test(TokenRegistryListResponse {
        lists: vec![registry_list.clone()],
    });
    roundtrip_test(TokenRegistryMutationResponse {
        status: "imported".to_string(),
        list: registry_list,
    });

    let step = ConsolidationPlanStep {
        id: "step_1".to_string(),
        sequence: 2,
        depends_on: vec!["step_0".to_string()],
        action: WalletPlanStepAction::SweepErc20,
        status: WalletPlanStepStatus::Blocked,
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        provider_profile: "mainnet".to_string(),
        chain_id: 1,
        address: "0x1111111111111111111111111111111111111111".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        asset_kind: WalletAssetKind::Erc20,
        asset_address: Some("0xtoken".to_string()),
        token_id_hex: None,
        counterparty_address: None,
        protocol_address: None,
        claim_adapter: Some("merkle-distributor-v1".to_string()),
        claim_index_hex: Some("0x7".to_string()),
        claim_proof: vec![format!("0x{}", "11".repeat(32))],
        exit_token0_address: Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
        exit_token1_address: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
        exit_amount0_min_hex: Some("0x10".to_string()),
        exit_amount1_min_hex: Some("0x20".to_string()),
        exit_deadline_unix: Some(1_783_046_000),
        amount_hex: "0x1".to_string(),
        destination_address: Some("0xdestination".to_string()),
        signer_status: WalletSignerStatus::Other("signing_not_implemented".to_string()),
        simulation_status: WalletSimulationStatus::NotRun,
        simulation_evidence: vec!["preflight not available".to_string()],
        risk_level: "blocked".to_string(),
        blockers: vec!["signing_not_implemented".to_string()],
        linkage_warnings: Vec::new(),
        auto_eligible: false,
        approved: false,
        queued_job_id: None,
    };
    let plan = ConsolidationPlan {
        id: "plan_1".to_string(),
        status: WalletPlanStatus::Blocked,
        chain_id: 1,
        destination_address: Some("0xdestination".to_string()),
        origin: None,
        created_at_unix: 1,
        updated_at_unix: 2,
        summary: ConsolidationPlanSummary {
            total_steps: 1,
            blocked_steps: 1,
            review_required_steps: 0,
            approved_steps: 0,
            executable_steps: 0,
            value_items: 1,
        },
        policy_violations: Vec::new(),
        linkage_findings: Vec::new(),
        steps: vec![step],
    };
    roundtrip_test(ConsolidationPlanListResponse {
        plans: vec![plan.clone()],
    });
    roundtrip_test(ConsolidationPlanMutationResponse {
        status: "generated".to_string(),
        plan: plan.clone(),
        plans: vec![plan],
    });
    roundtrip_test(ConsolidationPlanExportResponse {
        status: "exported".to_string(),
        plan_id: "plan_1".to_string(),
        format: "safe_tx_builder".to_string(),
        exported_steps: 1,
        skipped_steps: vec![ConsolidationPlanExportSkippedStep {
            step_id: "step_2".to_string(),
            action: WalletPlanStepAction::ClaimReward,
            reason: "blocked".to_string(),
            blockers: vec!["claim_execution_disabled".to_string()],
        }],
        bundles: vec![ConsolidationPlanExportBundle {
            chain_id: 1,
            provider_profile: "mainnet".to_string(),
            source_address: None,
            safe_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            calls: vec![ConsolidationPlanExportCall {
                step_id: "step_1".to_string(),
                action: WalletPlanStepAction::SweepErc20,
                from_address: "0x1111111111111111111111111111111111111111".to_string(),
                to_address: "0xtoken".to_string(),
                value_wei_hex: "0x0".to_string(),
                data_hex: "0xa9059cbb".to_string(),
                operation: 0,
                chain_id: 1,
                provider_profile: "mainnet".to_string(),
                asset_kind: WalletAssetKind::Erc20,
                amount_hex: "0x1".to_string(),
                evidence: vec!["prepared_call=erc20.transfer(destination,amount)".to_string()],
            }],
            safe_transaction_builder: Some(SafeTransactionBuilderBatch {
                version: "1.0".to_string(),
                chain_id: "1".to_string(),
                meta: SafeTransactionBuilderMeta {
                    name: "Sigillum consolidation export".to_string(),
                    description: "Approved Sigillum consolidation plan calls".to_string(),
                    tx_builder_version: "1.0".to_string(),
                    created_from_safe_address: Some(
                        "0x1111111111111111111111111111111111111111".to_string(),
                    ),
                },
                transactions: vec![SafeTransactionBuilderTransaction {
                    to: "0xtoken".to_string(),
                    value: "0".to_string(),
                    data: "0xa9059cbb".to_string(),
                    operation: 0,
                }],
            }),
        }],
    });
}

#[test]
fn test_eth_stealth_deposit_roundtrip() {
    let deposit = EthStealthDeposit {
        id: "deposit_1".to_string(),
        status: "funded".to_string(),
        asset_kind: "native".to_string(),
        wallet_profile: "wallet1".to_string(),
        chain_id: 1,
        chain_id_assumed: false,
        wallet_compartment_id: 1,
        provider_compartment_id: 1,
        wallet: "0xwallet".to_string(),
        short_name: "w1".to_string(),
        stealth_meta_address: "st:0x...".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        view_tag_hex: "0xaa".to_string(),
        announcement: Some(EthStealthAnnouncementPayload {
            announcer_address: "0x55649e01b5df198d18d95b5cc5051630cfd45564".to_string(),
            announce_function: "announce(uint256,address,bytes,bytes)".to_string(),
            scheme_id: ETHEREUM_STEALTH_SCHEME_ID,
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            metadata_hex: "aa".to_string(),
            calldata_hex: "0xcalldata".to_string(),
            value_wei_hex: "0x0".to_string(),
        }),
        token_address: None,
        expected_amount_hex: None,
        observed_amount_hex: Some("0x100".to_string()),
        observed_native_balance_wei_hex: Some("0x200".to_string()),
        auto_queue_sweep: true,
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        queue_job_id: None,
        queue_job_state: None,
        note: Some("test deposit".to_string()),
        created_at_unix: 1000000,
        updated_at_unix: 1000001,
        last_checked_at_unix: Some(1000002),
        broadcast_transaction_hash_hex: None,
        counterparty_id: None,
    };
    roundtrip_test(deposit);
}

#[test]
fn test_eth_stealth_deposit_legacy_chain_defaults() {
    let deposit = EthStealthDeposit {
        id: "deposit_1".to_string(),
        status: "funded".to_string(),
        asset_kind: "native".to_string(),
        wallet_profile: "wallet1".to_string(),
        chain_id: 8453,
        chain_id_assumed: false,
        wallet_compartment_id: 1,
        provider_compartment_id: 1,
        wallet: "0xwallet".to_string(),
        short_name: "w1".to_string(),
        stealth_meta_address: "st:0x...".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        view_tag_hex: "0xaa".to_string(),
        announcement: None,
        token_address: None,
        expected_amount_hex: None,
        observed_amount_hex: Some("0x100".to_string()),
        observed_native_balance_wei_hex: Some("0x200".to_string()),
        auto_queue_sweep: true,
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        queue_job_id: None,
        queue_job_state: None,
        note: Some("test deposit".to_string()),
        created_at_unix: 1000000,
        updated_at_unix: 1000001,
        last_checked_at_unix: Some(1000002),
        broadcast_transaction_hash_hex: None,
        counterparty_id: None,
    };
    let mut json = serde_json::to_value(deposit).unwrap();
    let object = json.as_object_mut().unwrap();
    object.remove("chain_id");
    object.remove("chain_id_assumed");

    let legacy: EthStealthDeposit = serde_json::from_value(json).unwrap();

    assert_eq!(legacy.chain_id, 1);
    assert!(legacy.chain_id_assumed);
}

#[test]
fn test_eth_stealth_announcement_scan_response_roundtrip() {
    roundtrip_test(EthStealthAnnouncementScanResponse {
        status: "scanned".to_string(),
        wallet_profile: "wallet1".to_string(),
        provider_profile: "mainnet".to_string(),
        from_block: "0x100".to_string(),
        to_block: "latest".to_string(),
        scanned: 10,
        matched: 2,
        created: 1,
        existing: 1,
        deposits: Vec::new(),
    });
}

#[test]
fn test_queue_job_payload_native_transfer_roundtrip() {
    let payload = QueueJobPayload::EthStealthTransfer {
        wallet_profile: "profile".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        value_wei_hex: "0x100".to_string(),
        destination_address: Some("0xdest".to_string()),
        nonce: Some(5),
        gas_limit: Some(21000),
        view_tag_hex: Some("0xaa".to_string()),
    };
    roundtrip_test(payload);
}

#[test]
fn test_queue_job_payload_erc20_transfer_roundtrip() {
    let payload = QueueJobPayload::EthStealthErc20Transfer {
        wallet_profile: "profile".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        token_address: "0xtoken".to_string(),
        recipient_address: "0xrecip".to_string(),
        amount_hex: "0x100".to_string(),
        nonce: None,
        gas_limit: Some(100000),
        view_tag_hex: None,
    };
    roundtrip_test(payload);
}

#[test]
fn test_queue_job_payload_native_sweep_roundtrip() {
    let payload = QueueJobPayload::EthStealthNativeSweep {
        wallet_profile: "profile".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        destination_address: Some("0xdest".to_string()),
        min_value_wei_hex: Some("0x1".to_string()),
        gas_limit: Some(21000),
        view_tag_hex: Some("0xaa".to_string()),
    };
    roundtrip_test(payload);
}

#[test]
fn test_queue_job_payload_erc20_sweep_roundtrip() {
    let payload = QueueJobPayload::EthStealthErc20Sweep {
        wallet_profile: "profile".to_string(),
        stealth_address: "0xstealth".to_string(),
        ephemeral_public_key_hex: "0xeph".to_string(),
        token_address: "0xtoken".to_string(),
        recipient_address: Some("0xrecip".to_string()),
        min_amount_hex: Some("0x10".to_string()),
        gas_limit: None,
        view_tag_hex: None,
    };
    roundtrip_test(payload);
}

#[test]
fn test_queue_job_with_flatten_and_tag_roundtrip() {
    let job = QueueJob {
        id: "job_1".to_string(),
        state: "pending".to_string(),
        attempts: 0,
        created_at_unix: 1000000,
        updated_at_unix: 1000001,
        next_attempt_after_unix: None,
        payload: QueueJobPayload::EthStealthTransfer {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            value_wei_hex: "0x100".to_string(),
            destination_address: Some("0xdest".to_string()),
            nonce: Some(5),
            gas_limit: Some(21000),
            view_tag_hex: None,
        },
        last_error: None,
        transaction_hash_hex: None,
        broadcast_transaction_hash_hex: None,
        receipt: Default::default(),
    };
    roundtrip_test(job);
}

#[test]
fn test_queue_job_with_error_roundtrip() {
    let job = QueueJob {
        id: "job_2".to_string(),
        state: "failed".to_string(),
        attempts: 3,
        created_at_unix: 1000000,
        updated_at_unix: 1000005,
        next_attempt_after_unix: None,
        payload: QueueJobPayload::EthStealthNativeSweep {
            wallet_profile: "profile".to_string(),
            stealth_address: "0xstealth".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            destination_address: None,
            min_value_wei_hex: None,
            gas_limit: None,
            view_tag_hex: None,
        },
        last_error: Some("Insufficient funds".to_string()),
        transaction_hash_hex: None,
        broadcast_transaction_hash_hex: Some("0xbcast".to_string()),
        receipt: Default::default(),
    };
    roundtrip_test(job);
}

#[test]
fn test_queue_job_list_response_roundtrip() {
    let resp = QueueJobListResponse {
        jobs: vec![QueueJob {
            id: "job_1".to_string(),
            state: "pending".to_string(),
            attempts: 0,
            created_at_unix: 1000000,
            updated_at_unix: 1000001,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthTransfer {
                wallet_profile: "profile".to_string(),
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                value_wei_hex: "0x100".to_string(),
                destination_address: None,
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }],
    };
    roundtrip_test(resp);
}

#[test]
fn test_queue_enqueue_response_roundtrip() {
    let resp = QueueEnqueueResponse {
        status: "enqueued".to_string(),
        job: QueueJob {
            id: "job_3".to_string(),
            state: "pending".to_string(),
            attempts: 0,
            created_at_unix: 1000000,
            updated_at_unix: 1000000,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthErc20Transfer {
                wallet_profile: "profile".to_string(),
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                token_address: "0xtoken".to_string(),
                recipient_address: "0xrecip".to_string(),
                amount_hex: "0x100".to_string(),
                nonce: None,
                gas_limit: None,
                view_tag_hex: None,
            },
            last_error: None,
            transaction_hash_hex: None,
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        },
    };
    roundtrip_test(resp);
}

#[test]
fn test_queue_process_response_roundtrip() {
    let resp = QueueProcessResponse {
        processed: 5,
        succeeded: 4,
        blocked: 0,
        retrying: 0,
        operator_action_required: 0,
        failed: 1,
        confirmed: 0,
        failures_by_cause: MaintenanceFailureBreakdown {
            provider_error: 1,
            ..MaintenanceFailureBreakdown::default()
        },
        paused_reason: None,
        jobs: vec![QueueJob {
            id: "job_4".to_string(),
            state: "completed".to_string(),
            attempts: 1,
            created_at_unix: 1000000,
            updated_at_unix: 1000010,
            next_attempt_after_unix: None,
            payload: QueueJobPayload::EthStealthNativeSweep {
                wallet_profile: "profile".to_string(),
                stealth_address: "0xstealth".to_string(),
                ephemeral_public_key_hex: "0xeph".to_string(),
                destination_address: Some("0xdest".to_string()),
                min_value_wei_hex: Some("0x1".to_string()),
                gas_limit: Some(21000),
                view_tag_hex: Some("0xaa".to_string()),
            },
            last_error: None,
            transaction_hash_hex: Some("0xhash".to_string()),
            broadcast_transaction_hash_hex: None,
            receipt: Default::default(),
        }],
    };
    roundtrip_test(resp);
    roundtrip_test(QueueExecutionPauseResponse {
        status: "updated".to_string(),
        execution_paused: true,
    });
}

#[test]
fn test_queue_process_response_defaults_failure_breakdown() {
    let resp: QueueProcessResponse = serde_json::from_value(serde_json::json!({
        "processed": 0,
        "succeeded": 0,
        "failed": 0,
        "jobs": []
    }))
    .unwrap();
    assert_eq!(
        resp.failures_by_cause,
        MaintenanceFailureBreakdown::default()
    );
    assert!(resp.paused_reason.is_none());
}

#[test]
fn test_maintenance_run_response_roundtrip() {
    let resp = MaintenanceRunResponse {
        status: "ok".to_string(),
        refreshed: 3,
        detected: 2,
        queued: 1,
        processed: 4,
        succeeded: 1,
        blocked: 1,
        retrying: 1,
        operator_action_required: 0,
        failed: 1,
        confirmed: 0,
        failures_by_cause: MaintenanceFailureBreakdown {
            provider_error: 1,
            policy_block: 1,
            insufficient_gas: 1,
            validation: 1,
            unknown: 0,
            on_chain_revert: 0,
            broadcast_rejected: 0,
            receipt_timeout: 0,
        },
        treasury_automation: None,
        deposits: vec![],
        jobs: vec![],
    };
    roundtrip_test(resp);
}

#[test]
fn test_maintenance_run_response_defaults_failure_breakdown() {
    let resp: MaintenanceRunResponse = serde_json::from_value(serde_json::json!({
        "status": "ok",
        "refreshed": 0,
        "detected": 0,
        "queued": 0,
        "processed": 0,
        "succeeded": 0,
        "failed": 0,
        "deposits": [],
        "jobs": []
    }))
    .unwrap();
    assert_eq!(
        resp.failures_by_cause,
        MaintenanceFailureBreakdown::default()
    );
    assert!(resp.treasury_automation.is_none());
}

#[test]
fn test_maintenance_run_response_treasury_automation_roundtrip() {
    let resp: MaintenanceRunResponse = serde_json::from_value(serde_json::json!({
        "status": "ok",
        "refreshed": 0,
        "detected": 0,
        "queued": 0,
        "processed": 0,
        "succeeded": 0,
        "failed": 0,
        "treasury_automation": {
            "generated_steps": 2,
            "enqueued_steps": 1,
            "skipped_steps": 1,
            "skipped_reasons": ["simulation_failed"]
        },
        "deposits": [],
        "jobs": []
    }))
    .unwrap();
    assert_eq!(
        resp.treasury_automation,
        Some(TreasuryAutomationRunSummary {
            generated_steps: 2,
            enqueued_steps: 1,
            skipped_steps: 1,
            skipped_reasons: vec!["simulation_failed".to_string()],
        })
    );
    roundtrip_test(resp);
}

#[test]
fn test_snapshot_export_response_roundtrip() {
    let resp = SnapshotExportResponse {
        status: "exported".to_string(),
        snapshot_hex: "deadbeef".to_string(),
        summary: SnapshotSummary {
            created_at_unix: 1000000,
            file_count: 10,
            total_bytes: 5000,
        },
    };
    roundtrip_test(resp);
}

#[test]
fn test_snapshot_restore_response_roundtrip() {
    let resp = SnapshotRestoreResponse {
        status: "restored".to_string(),
        summary: SnapshotSummary {
            created_at_unix: 1000000,
            file_count: 10,
            total_bytes: 5000,
        },
        requires_reauth: false,
    };
    roundtrip_test(resp);
}

#[test]
fn test_audit_event_roundtrip() {
    let event = AuditEvent {
        created_at_unix: 1000000,
        kind: "unlock".to_string(),
        compartment_id: Some(1),
        details: serde_json::json!({"method": "fido2"}),
    };
    roundtrip_test(event);
}

#[test]
fn test_audit_verify_report_roundtrip() {
    roundtrip_test(AuditVerifyReport {
        scope: "daemon".to_string(),
        status: "verified".to_string(),
        verified: 3,
        broken: 0,
        legacy: 1,
    });
}

#[test]
fn test_fido2_status_response_roundtrip() {
    let resp = Fido2StatusResponse {
        enabled: true,
        key_count: 3,
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_detect_response_roundtrip() {
    let resp = Fido2DetectResponse {
        device_present: true,
        device_count: 2,
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_key_info_roundtrip() {
    let info = Fido2KeyInfo {
        label: "my_key".to_string(),
        credential_id_short: "abc123".to_string(),
        registered_at: "2024-01-01T00:00:00Z".to_string(),
    };
    roundtrip_test(info);
}

#[test]
fn test_fido2_list_response_roundtrip() {
    let resp = Fido2ListResponse {
        keys: vec![
            Fido2KeyInfo {
                label: "key1".to_string(),
                credential_id_short: "abc".to_string(),
                registered_at: "2024-01-01T00:00:00Z".to_string(),
            },
            Fido2KeyInfo {
                label: "key2".to_string(),
                credential_id_short: "def".to_string(),
                registered_at: "2024-01-02T00:00:00Z".to_string(),
            },
        ],
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_register_response_with_poison() {
    let resp = Fido2RegisterResponse {
        status: "registered".to_string(),
        label: "new_key".to_string(),
        total_keys: 2,
        poison: Some(true),
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_register_response_no_poison() {
    let resp = Fido2RegisterResponse {
        status: "registered".to_string(),
        label: "new_key".to_string(),
        total_keys: 2,
        poison: None,
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_remove_response_roundtrip() {
    let resp = Fido2RemoveResponse {
        status: "removed".to_string(),
        label: "old_key".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_fido2_set_pin_response_roundtrip() {
    let resp = Fido2SetPinResponse {
        status: "pin_set".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_compartment_added_response_roundtrip() {
    let resp = CompartmentAddedResponse {
        status: "added".to_string(),
        id: 3,
        label: "new_vault".to_string(),
        threshold: 2,
    };
    roundtrip_test(resp);
}

#[test]
fn test_compartment_removed_response_roundtrip() {
    let resp = CompartmentRemovedResponse {
        status: "removed".to_string(),
        id: 2,
    };
    roundtrip_test(resp);
}

#[test]
fn test_compartment_initialized_response_roundtrip() {
    let resp = CompartmentInitializedResponse {
        status: "initialized".to_string(),
        compartment_id: 1,
        compartment_label: "vault1".to_string(),
        session_token: "token123".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_switch_compartment_response_roundtrip() {
    let resp = SwitchCompartmentResponse {
        status: "switched".to_string(),
        compartment_id: 2,
        compartment_label: "vault2".to_string(),
    };
    roundtrip_test(resp);
}

#[test]
fn test_session_revoke_response_roundtrip() {
    let resp = SessionRevokeResponse {
        status: "revoked".to_string(),
        requires_reauth: true,
    };
    roundtrip_test(resp);
}

#[test]
fn test_treasury_overview_response_roundtrip() {
    let resp = TreasuryOverviewResponse {
        generated_at_unix: 99,
        tracked_address_count: 4,
        funded_address_count: 2,
        watch_only_address_count: 1,
        signer_address_count: 3,
        chains: vec![TreasuryChainSummary {
            chain_id: 1,
            native_symbol: "ETH".to_string(),
            address_count: 4,
            funded_address_count: 2,
            native_total_wei_hex: "0xde0b6b3a7640000".to_string(),
        }],
        groups: vec![TreasuryGroupSummary {
            wallet_family: "eth-seed".to_string(),
            wallet_profile: "seed-main".to_string(),
            chain_id: 1,
            address_count: 3,
            funded_address_count: 2,
            native_total_wei_hex: "0xde0b6b3a7640000".to_string(),
            signer_address_count: 3,
            watch_only_address_count: 0,
            erc20_holding_count: 2,
            nft_holding_count: 1,
            defi_holding_count: 1,
            claimable_holding_count: 1,
            approval_exposure_count: 1,
            dormant_candidate_count: 1,
        }],
        routing: vec![TreasuryRoutingStatus {
            wallet_profile: "seed-main".to_string(),
            hot_address: Some("0x1111111111111111111111111111111111111111".to_string()),
            treasury_address: Some("0x2222222222222222222222222222222222222222".to_string()),
            default_destination_address: None,
            hot_native_balance_wei_hex: Some("0x1".to_string()),
            treasury_native_balance_wei_hex: Some("0x2".to_string()),
            routing_ready: true,
        }],
        risk: TreasuryRiskSummary {
            total_findings: 3,
            critical_findings: 0,
            high_findings: 1,
            medium_findings: 1,
            low_findings: 1,
        },
        plans: TreasuryPlanSummary {
            total_plans: 2,
            latest_plan_id: Some("plan_1".to_string()),
            latest_plan_status: Some("review_required".to_string()),
            latest_review_required_steps: 2,
            latest_approved_steps: 1,
            latest_executable_steps: 0,
            latest_blocked_steps: 1,
            latest_policy_violations: vec!["exceeds_policy_plan_cap".to_string()],
        },
        receive: TreasuryReceiveSummary {
            active_allocations: 2,
            retired_allocations: 1,
            purposes: 2,
        },
        automation: TreasuryAutomationStatus {
            enabled: true,
            hot_overflow_wei_hex: Some("0x2".to_string()),
            generated_steps: 3,
            enqueued_steps: 1,
        },
    };
    roundtrip_test(resp);
}

#[test]
fn test_treasury_overview_receive_summary_defaults_when_absent() {
    // Payloads generated before receive allocations existed omit the field.
    let resp: TreasuryOverviewResponse = serde_json::from_str(
        r#"{
            "generated_at_unix": 1,
            "tracked_address_count": 0,
            "funded_address_count": 0,
            "watch_only_address_count": 0,
            "signer_address_count": 0,
            "risk": {
                "total_findings": 0,
                "critical_findings": 0,
                "high_findings": 0,
                "medium_findings": 0,
                "low_findings": 0
            },
            "plans": {
                "total_plans": 0,
                "latest_review_required_steps": 0,
                "latest_approved_steps": 0,
                "latest_executable_steps": 0,
                "latest_blocked_steps": 0
            }
        }"#,
    )
    .unwrap();
    assert_eq!(resp.receive, TreasuryReceiveSummary::default());
    assert_eq!(resp.automation, TreasuryAutomationStatus::default());
}

fn sample_receive_allocation() -> TreasuryReceiveAllocation {
    TreasuryReceiveAllocation {
        id: "alloc_1".to_string(),
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        chain_id: 1,
        chain_id_assumed: false,
        address: "0x1111111111111111111111111111111111111111".to_string(),
        derivation_path: "m/44'/60'/0'/0/5".to_string(),
        address_index: 5,
        purpose: "counterparty-acme".to_string(),
        label: Some("Acme invoices".to_string()),
        status: "active".to_string(),
        created_at_unix: 10,
        retired_at_unix: None,
        counterparty_id: None,
    }
}

#[test]
fn test_treasury_receive_allocation_responses_roundtrip() {
    roundtrip_test(sample_receive_allocation());
    roundtrip_test(TreasuryReceiveAllocation {
        label: None,
        status: "retired".to_string(),
        retired_at_unix: Some(11),
        ..sample_receive_allocation()
    });
    roundtrip_test(TreasuryReceiveAllocation {
        counterparty_id: Some("cp_1".into()),
        ..sample_receive_allocation()
    });
    roundtrip_test(TreasuryReceiveAllocationListResponse {
        allocations: vec![sample_receive_allocation()],
    });
    roundtrip_test(TreasuryReceiveAllocationListResponse {
        allocations: Vec::new(),
    });
    roundtrip_test(TreasuryReceiveAllocationMutationResponse {
        status: "allocated".to_string(),
        allocation: sample_receive_allocation(),
    });
    let mut json = serde_json::to_value(sample_receive_allocation()).unwrap();
    let object = json.as_object_mut().unwrap();
    object.remove("chain_id");
    object.remove("chain_id_assumed");
    let legacy: TreasuryReceiveAllocation = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.chain_id, 1);
    assert!(legacy.chain_id_assumed);
    roundtrip_test(ReceivingRefreshResponse {
        generated_at_unix: 99,
        addresses_requested: 3,
        addresses_refreshed: 2,
        addresses_skipped: 1,
        stealth_refreshed: true,
        provider_status: "partial".to_string(),
        errors: vec!["mainnet 0xabc: timeout".to_string()],
    });
}

#[test]
fn test_counterparty_responses_roundtrip() {
    roundtrip_test(Counterparty {
        id: "cp_1".into(),
        name: "Acme Corp".into(),
        note: Some("net-30".into()),
        sweep_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
        created_at_unix: 5,
    });
    roundtrip_test(Counterparty {
        id: "cp_2".into(),
        name: "Beta LLC".into(),
        note: None,
        sweep_destination_address: None,
        created_at_unix: 6,
    });
    roundtrip_test(CounterpartyListResponse {
        parties: vec![Counterparty {
            id: "cp_1".into(),
            name: "Acme Corp".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 5,
        }],
    });
    roundtrip_test(CounterpartyListResponse {
        parties: Vec::new(),
    });
    roundtrip_test(CounterpartyMutationResponse {
        status: "created".into(),
        party: Some(Counterparty {
            id: "cp_1".into(),
            name: "Acme Corp".into(),
            note: None,
            sweep_destination_address: None,
            created_at_unix: 5,
        }),
    });
    roundtrip_test(CounterpartyMutationResponse {
        status: "deleted".into(),
        party: None,
    });
}

fn sample_treasury_policy() -> TreasuryPolicy {
    TreasuryPolicy {
        enabled: true,
        allowed_destinations: vec![
            TreasuryAllowedDestination {
                address: "0x9999999999999999999999999999999999999999".to_string(),
                label: Some("cold-treasury".to_string()),
            },
            TreasuryAllowedDestination {
                address: "0x8888888888888888888888888888888888888888".to_string(),
                label: None,
            },
        ],
        max_step_native_wei_hex: Some("0xde0b6b3a7640000".to_string()),
        max_plan_native_wei_hex: Some("0x1bc16d674ec80000".to_string()),
        require_simulation: true,
        allow_raw_digest_signing: false,
        block_cross_party_linkage: false,
        allow_claim_execution: false,
        allow_gas_topups: false,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: true,
        allow_sweep_execution: true,
        allow_revoke_execution: true,
        allow_exit_execution: true,
        execution_paused: true,
        max_fee_per_gas_cap_hex: Some("0x59682f00".to_string()),
        simulation_freshness_secs: 900,
        hot_floor_wei_hex: "0xde0b6b3a7640000".to_string(),
        hot_target_wei_hex: "0xde0b6b3a7640000".to_string(),
        hot_overflow_wei_hex: None,
        allow_treasury_automation: false,
        created_at_unix: 1,
        updated_at_unix: 2,
    }
}

#[test]
fn test_treasury_policy_responses_roundtrip() {
    roundtrip_test(sample_treasury_policy());
    roundtrip_test(TreasuryPolicyResponse {
        policy: Some(sample_treasury_policy()),
    });
    roundtrip_test(TreasuryPolicyResponse { policy: None });
    roundtrip_test(TreasuryPolicyMutationResponse {
        status: "updated".to_string(),
        policy: TreasuryPolicy {
            enabled: false,
            allowed_destinations: Vec::new(),
            max_step_native_wei_hex: None,
            max_plan_native_wei_hex: None,
            require_simulation: false,
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
            hot_floor_wei_hex: "0xde0b6b3a7640000".to_string(),
            hot_target_wei_hex: "0xde0b6b3a7640000".to_string(),
            hot_overflow_wei_hex: None,
            allow_treasury_automation: false,
            created_at_unix: 1,
            updated_at_unix: 3,
        },
    });
}

#[test]
fn test_treasury_policy_require_simulation_defaults_true() {
    // Older or hand-written payloads without the field must stay strict.
    let policy: TreasuryPolicy =
        serde_json::from_str(r#"{"enabled":true,"created_at_unix":1,"updated_at_unix":2}"#)
            .unwrap();
    assert!(policy.require_simulation);
    assert!(!policy.block_cross_party_linkage);
    assert!(!policy.allow_claim_execution);
    assert!(!policy.allow_gas_topups);
    assert!(policy.max_gas_topup_wei_hex.is_none());
    assert!(!policy.allow_plan_execution);
    assert!(!policy.allow_sweep_execution);
    assert!(!policy.allow_revoke_execution);
    assert!(!policy.allow_exit_execution);
    assert!(!policy.execution_paused);
    assert!(policy.max_fee_per_gas_cap_hex.is_none());
    assert_eq!(policy.simulation_freshness_secs, 900);
    assert_eq!(policy.hot_floor_wei_hex, "0xde0b6b3a7640000");
    assert_eq!(policy.hot_target_wei_hex, "0xde0b6b3a7640000");
    assert_eq!(policy.hot_overflow_wei_hex, None);
    assert!(!policy.allow_treasury_automation);
    assert!(policy.allowed_destinations.is_empty());
}

#[test]
fn test_evm_provider_profile_fee_estimation_defaults_false() {
    let profile: EvmProviderProfile = serde_json::from_str(
        r#"{
            "name":"mainnet",
            "rpc_url":"https://eth.example.com",
            "compartment_id":1,
            "chain_id":1
        }"#,
    )
    .unwrap();
    assert!(!profile.fee_estimation_enabled);
}

#[test]
fn test_consolidation_plan_policy_violations_roundtrip() {
    let base = ConsolidationPlan {
        id: "plan_2".to_string(),
        status: WalletPlanStatus::Blocked,
        chain_id: 1,
        destination_address: Some("0x9999999999999999999999999999999999999999".to_string()),
        origin: None,
        created_at_unix: 1,
        updated_at_unix: 2,
        summary: ConsolidationPlanSummary {
            total_steps: 0,
            blocked_steps: 0,
            review_required_steps: 0,
            approved_steps: 0,
            executable_steps: 0,
            value_items: 0,
        },
        policy_violations: vec!["exceeds_policy_plan_cap".to_string()],
        linkage_findings: Vec::new(),
        steps: Vec::new(),
    };
    roundtrip_test(base.clone());

    // Empty violations are skipped on the wire and default back in.
    let empty = ConsolidationPlan {
        policy_violations: Vec::new(),
        origin: None,
        ..base
    };
    let json = serde_json::to_string(&empty).unwrap();
    assert!(!json.contains("policy_violations"));
    roundtrip_test(empty);

    let legacy: ConsolidationPlan = serde_json::from_str(
        r#"{
            "id":"plan_legacy",
            "status":"approved",
            "chain_id":1,
            "destination_address":"0x9999999999999999999999999999999999999999",
            "created_at_unix":1,
            "updated_at_unix":2,
            "summary":{
                "total_steps":0,
                "blocked_steps":0,
                "review_required_steps":0,
                "approved_steps":0,
                "executable_steps":0,
                "value_items":0
            },
            "steps":[]
        }"#,
    )
    .unwrap();
    assert_eq!(legacy.origin, None);
}

#[test]
fn test_self_check_run_response_roundtrip() {
    roundtrip_test(SelfCheckRunResponse {
        status: "warn".to_string(),
        generated_at_unix: 1_781_045_920,
        checks: vec![
            SelfCheckResult {
                id: "provider:mainnet".to_string(),
                domain: "provider".to_string(),
                subject: "mainnet".to_string(),
                status: "pass".to_string(),
                detail: "Chain id 1 verified".to_string(),
                latency_ms: Some(42),
            },
            SelfCheckResult {
                id: "policy:treasury".to_string(),
                domain: "policy".to_string(),
                subject: "treasury".to_string(),
                status: "warn".to_string(),
                detail: "No treasury policy configured — sweeps are unguarded".to_string(),
                latency_ms: None,
            },
        ],
    });
}

#[test]
fn test_self_check_result_skips_absent_latency() {
    let probe_less = SelfCheckResult {
        id: "watch-book:0xabc".to_string(),
        domain: "watch-book".to_string(),
        subject: "0xabc".to_string(),
        status: "pass".to_string(),
        detail: "disabled".to_string(),
        latency_ms: None,
    };
    let json = serde_json::to_string(&probe_less).unwrap();
    assert!(!json.contains("latency_ms"));
    roundtrip_test(probe_less);

    let probed = SelfCheckResult {
        id: "provider:mainnet".to_string(),
        domain: "provider".to_string(),
        subject: "mainnet".to_string(),
        status: "fail".to_string(),
        detail: "Chain ID mismatch: provider reports 5, profile says 1".to_string(),
        latency_ms: Some(7),
    };
    let json = serde_json::to_string(&probed).unwrap();
    assert!(json.contains("\"latency_ms\":7"));
    roundtrip_test(probed);
}

#[test]
fn test_setup_reset_response_roundtrip() {
    roundtrip_test(SetupResetResponse {
        status: "reset".to_string(),
        archived_to: Some("/home/op/.sigillum.archived-1781045920".to_string()),
    });
    let bare = SetupResetResponse {
        status: "reset".to_string(),
        archived_to: None,
    };
    let json = serde_json::to_string(&bare).unwrap();
    assert!(!json.contains("archived_to"));
    roundtrip_test(bare);
}

#[test]
fn test_token_registry_entry_accepts_chain_id_alias() {
    let entry: TokenRegistryEntry = serde_json::from_value(serde_json::json!({
        "chainId": 1,
        "address": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "symbol": "AAA",
        "decimals": 18
    }))
    .unwrap();

    assert_eq!(entry.chain_id, 1);
}
