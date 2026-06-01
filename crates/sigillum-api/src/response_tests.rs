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
            },
            EthStealthWalletProfile {
                name: "wallet2".to_string(),
                wallet: "0xwallet2".to_string(),
                short_name: "w2".to_string(),
                provider_profile: "testnet".to_string(),
                compartment_id: 2,
                chain_id: Some(5),
                default_destination_address: Some("0xdest2".to_string()),
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
        default_destination_address: Some("0xdest".to_string()),
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
                default_destination_address: None,
            },
            EthXpubWalletProfile {
                name: "project_b".to_string(),
                project_account: 15,
                provider_profile: "testnet".to_string(),
                compartment_id: 2,
                chain_id: Some(5),
                default_destination_address: Some("0xdest2".to_string()),
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
        }],
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
        address_index: 0,
        activity_state: "funded".to_string(),
        native_balance_wei_hex: "0x1".to_string(),
        transaction_count: 1,
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
        asset_kind: "native".to_string(),
        asset_address: None,
        token_id_hex: None,
        counterparty_address: None,
        protocol_address: None,
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
        gap_limit: 20,
        max_index: 200,
        addresses_scanned: 1,
        active_addresses: 1,
        holdings_detected: 1,
        started_at_unix: 1,
        completed_at_unix: Some(2),
        last_error: None,
    };
    roundtrip_test(WalletInventoryScanResponse {
        job,
        addresses: vec![address],
        holdings: vec![holding],
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
        explorer_url: Some("https://basescan.org".to_string()),
        capabilities: vec!["native".to_string(), "erc20".to_string()],
        enabled: true,
        source: "operator".to_string(),
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

    let step = ConsolidationPlanStep {
        id: "step_1".to_string(),
        action: "sweep_erc20".to_string(),
        status: "blocked".to_string(),
        wallet_family: "eth-seed".to_string(),
        wallet_profile: "seed-main".to_string(),
        provider_profile: "mainnet".to_string(),
        chain_id: 1,
        address: "0x1111111111111111111111111111111111111111".to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        asset_kind: "erc20".to_string(),
        asset_address: Some("0xtoken".to_string()),
        token_id_hex: None,
        counterparty_address: None,
        protocol_address: None,
        amount_hex: "0x1".to_string(),
        destination_address: Some("0xdestination".to_string()),
        signer_status: "signing_not_implemented".to_string(),
        simulation_status: "not_run".to_string(),
        simulation_evidence: vec!["preflight not available".to_string()],
        risk_level: "blocked".to_string(),
        blockers: vec!["signing_not_implemented".to_string()],
        auto_eligible: false,
        approved: false,
    };
    let plan = ConsolidationPlan {
        id: "plan_1".to_string(),
        status: "blocked".to_string(),
        destination_address: Some("0xdestination".to_string()),
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
        steps: vec![step],
    };
    roundtrip_test(ConsolidationPlanListResponse {
        plans: vec![plan.clone()],
    });
    roundtrip_test(ConsolidationPlanMutationResponse {
        status: "generated".to_string(),
        plan,
    });
}

#[test]
fn test_eth_stealth_deposit_roundtrip() {
    let deposit = EthStealthDeposit {
        id: "deposit_1".to_string(),
        status: "funded".to_string(),
        asset_kind: "native".to_string(),
        wallet_profile: "wallet1".to_string(),
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
    };
    roundtrip_test(deposit);
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
        failed: 1,
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
        }],
    };
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
