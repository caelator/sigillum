use super::*;
use crate::validation::Validate;

fn roundtrip_test<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
    value: T,
) {
    let json = serde_json::to_string(&value).unwrap();
    let deserialized: T = serde_json::from_str(&json).unwrap();
    assert_eq!(value, deserialized, "Roundtrip failed for JSON: {json}");
}

#[test]
fn test_key_value_request_roundtrip() {
    let req = KeyValueRequest {
        key: "test_key".to_string(),
        value: Some("test_value".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_key_value_request_none_value() {
    let req = KeyValueRequest {
        key: "test_key".to_string(),
        value: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_key_only_request_roundtrip() {
    let req = KeyOnlyRequest {
        key: "my_key".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_passphrase_request_roundtrip() {
    let req = PassphraseRequest {
        passphrase: "my_secure_passphrase".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_secret_resolve_batch_request_roundtrip() {
    let req = SecretResolveBatchRequest {
        entries: vec![
            SecretResolveRequest {
                env_name: "DB_PASS".into(),
                reference: "prod:db.password".into(),
            },
            SecretResolveRequest {
                env_name: "API_TOKEN".into(),
                reference: "api.token".into(),
            },
        ],
    };
    roundtrip_test(req);
}

#[test]
fn test_run_audit_request_roundtrip() {
    let req = RunAuditRequest {
        program: "npm".into(),
        args: vec!["start".into()],
        exit_code: Some(0),
        signal: None,
        success: true,
    };
    roundtrip_test(req);
}

#[test]
fn test_generate_store_request_roundtrip() {
    let req = GenerateStoreRequest {
        key: "db.password".into(),
        kind: GenerateStoreKind::Password {
            length: 32,
            charset: PasswordCharset::MixalphaNumericSymbol,
        },
    };
    roundtrip_test(req);
}

#[test]
fn test_snapshot_restore_request_roundtrip() {
    let req = SnapshotRestoreRequest {
        passphrase: "passphrase123".to_string(),
        snapshot_hex: "deadbeef".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_setup_reset_request_roundtrip() {
    let req = SetupResetRequest {
        confirmation: "RESET LOCAL SIGILLUM DATA".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_compartment_definition_roundtrip() {
    let def = CompartmentDefinition {
        label: "vault_1".to_string(),
        threshold: 2,
        passphrase_mode: Some("FIXED".to_string()),
    };
    roundtrip_test(def);
}

#[test]
fn test_fido2_setup_request_roundtrip() {
    let req = Fido2SetupRequest {
        pin: Some("1234".to_string()),
        label: "my_key".to_string(),
        compartments: vec![CompartmentDefinition {
            label: "vault_1".to_string(),
            threshold: 1,
            passphrase_mode: None,
        }],
        passphrase: Some("setup_pass".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_setup_request_without_pin_roundtrip() {
    let req = Fido2SetupRequest {
        pin: None,
        label: "touch_only_key".to_string(),
        compartments: vec![CompartmentDefinition {
            label: "vault_1".to_string(),
            threshold: 1,
            passphrase_mode: None,
        }],
        passphrase: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_set_pin_request_roundtrip() {
    let req = Fido2SetPinRequest {
        new_pin: "2468".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_register_request_without_pin_roundtrip() {
    let req = Fido2RegisterRequest {
        pin: None,
        label: "backup_key".to_string(),
        poison: Some(false),
        skip_keys: Some(vec!["retired".to_string()]),
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_unlock_request_roundtrip() {
    let req = Fido2UnlockRequest {
        pins: vec!["1234".to_string(), "5678".to_string()],
        tap_count: 3,
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_unlock_request_without_pins_roundtrip() {
    let req = Fido2UnlockRequest {
        pins: Vec::new(),
        tap_count: 2,
    };
    roundtrip_test(req);
}

#[test]
fn test_fido2_remove_request_without_pin_roundtrip() {
    let req = Fido2RemoveRequest {
        label: "backup_key".to_string(),
        pin: None,
        skip_keys: Some(vec!["offline".to_string()]),
    };
    roundtrip_test(req);
}

#[test]
fn test_compartment_init_request_roundtrip() {
    let req = CompartmentInitRequest {
        id: 1,
        passphrase: "init_pass".to_string(),
        label: Some("new_label".to_string()),
        threshold: Some(2),
    };
    roundtrip_test(req);
}

#[test]
fn test_compartment_init_rejects_zero_threshold() {
    use crate::validation::Validate;

    let req = CompartmentInitRequest {
        id: 1,
        passphrase: "init_pass".to_string(),
        label: Some("new_label".to_string()),
        threshold: Some(0),
    };

    assert!(req.validate().unwrap_err().contains("threshold"));
}

#[test]
fn test_secrets_push_request_roundtrip() {
    let req = SecretsPushRequest {
        from_compartment: 1,
        to_compartment: 2,
        key: "secret_key".to_string(),
        new_key: Some("renamed_key".to_string()),
        tier: Some(3),
    };
    roundtrip_test(req);
}

#[test]
fn test_transit_encrypt_request_roundtrip() {
    let req = TransitEncryptRequest {
        key: "encryption_key".to_string(),
        plaintext_hex: "48656c6c6f".to_string(),
        aad_hex: Some("aabbccdd".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_transit_decrypt_request_with_aad() {
    let req = TransitDecryptRequest {
        key: "encryption_key".to_string(),
        nonce_hex: "0123456789abcdef".to_string(),
        ciphertext_hex: "fedcba9876543210".to_string(),
        aad_hex: Some("aabbccdd".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_transit_decrypt_request_no_aad() {
    let req = TransitDecryptRequest {
        key: "encryption_key".to_string(),
        nonce_hex: "0123456789abcdef".to_string(),
        ciphertext_hex: "fedcba9876543210".to_string(),
        aad_hex: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_export_request_roundtrip() {
    let req = EthStealthExportRequest {
        wallet: "0xabc123".to_string(),
        short_name: Some("my_wallet".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_export_request_no_short_name() {
    let req = EthStealthExportRequest {
        wallet: "0xabc123".to_string(),
        short_name: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_generate_request_roundtrip() {
    let req = EthStealthGenerateRequest {
        stealth_meta_address: "st:0x...".to_string(),
        ephemeral_private_key_hex: Some("abcd1234".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_check_request_roundtrip() {
    let req = EthStealthCheckRequest {
        wallet: "0xwallet".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xephemeral".to_string(),
            view_tag_hex: Some("0xaa".to_string()),
            stealth_hash_convention: None,
        },
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_rpc_nonce_request_full() {
    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: Some("token_key".to_string()),
            compartment_id: Some(1),
        },
        address: "0x1111111111111111111111111111111111111111".to_string(),
        block_tag: Some("latest".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_rpc_nonce_request_minimal() {
    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "0x1111111111111111111111111111111111111111".to_string(),
        block_tag: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_rpc_balance_request_roundtrip() {
    let req = EvmRpcBalanceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: Some(2),
        },
        address: "0x1111111111111111111111111111111111111111".to_string(),
        block_tag: Some("safe".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_rpc_erc20_balance_request_roundtrip() {
    let req = EvmRpcErc20BalanceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: Some("key".to_string()),
            compartment_id: None,
        },
        token_address: "0x2222222222222222222222222222222222222222".to_string(),
        owner_address: "0x3333333333333333333333333333333333333333".to_string(),
        block_tag: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_address_validation_accepts_valid_prefixed_address() {
    use crate::validation::Validate;

    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "0x000000000000000000000000000000000000dEaD".to_string(),
        block_tag: None,
    };

    req.validate().unwrap();
}

#[test]
fn test_eth_address_validation_accepts_bare_mixed_case_address() {
    use crate::validation::Validate;

    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "000000000000000000000000000000000000dEaD".to_string(),
        block_tag: None,
    };

    req.validate().unwrap();
}

#[test]
fn test_eth_address_validation_rejects_too_short_address() {
    use crate::validation::Validate;

    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "0x1234".to_string(),
        block_tag: None,
    };

    let err = req.validate().unwrap_err();
    assert!(err.contains("address"), "error should mention field: {err}");
}

#[test]
fn test_eth_address_validation_rejects_non_hex_40_char_address() {
    use crate::validation::Validate;

    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "0x000000000000000000000000000000000000000g".to_string(),
        block_tag: None,
    };

    let err = req.validate().unwrap_err();
    assert!(err.contains("address"), "error should mention field: {err}");
}

#[test]
fn test_eth_address_validation_rejects_missing_required_length() {
    use crate::validation::Validate;

    let req = EvmRpcNonceRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        address: "000000000000000000000000000000000000000".to_string(),
        block_tag: None,
    };

    let err = req.validate().unwrap_err();
    assert!(err.contains("address"), "error should mention field: {err}");
}

#[test]
fn test_optional_eth_address_validation_allows_omitted_address() {
    use crate::validation::Validate;

    let req = EthStealthSendWithProfileRequest {
        wallet_profile: "my_profile".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: None,
            stealth_hash_convention: None,
        },
        value_wei_hex: "0x100".to_string(),
        destination_address: None,
        nonce: None,
        gas_limit: None,
        estimate_fees: None,
        broadcast: None,
    };

    req.validate().unwrap();
}

#[test]
fn test_evm_rpc_broadcast_request_roundtrip() {
    let req = EvmRpcBroadcastRequest {
        provider: EvmProviderRef {
            rpc_url: "https://rpc.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        raw_transaction_hex: "0xrxn".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_send_transfer_request_full() {
    let req = EthStealthSendTransferRequest {
        rpc_url: "https://rpc.example.com".to_string(),
        wallet: "0xwallet".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: Some("0xaa".to_string()),
            stealth_hash_convention: None,
        },
        fees: Eip1559Fees {
            chain_id: 1,
            max_priority_fee_per_gas_hex: "0x1".to_string(),
            max_fee_per_gas_hex: "0x2".to_string(),
        },
        destination_address: "0x000000000000000000000000000000000000dEaD".to_string(),
        value_wei_hex: "0x100".to_string(),
        auth_token_key: Some("key".to_string()),
        provider_compartment_id: Some(1),
        wallet_compartment_id: Some(2),
        nonce: Some(5),
        gas_limit: Some(21000),
        broadcast: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_send_transfer_request_minimal() {
    let req = EthStealthSendTransferRequest {
        rpc_url: "https://rpc.example.com".to_string(),
        wallet: "0xwallet".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: None,
            stealth_hash_convention: None,
        },
        fees: Eip1559Fees {
            chain_id: 1,
            max_priority_fee_per_gas_hex: "0x1".to_string(),
            max_fee_per_gas_hex: "0x2".to_string(),
        },
        destination_address: "0x000000000000000000000000000000000000dEaD".to_string(),
        value_wei_hex: "0x100".to_string(),
        auth_token_key: None,
        provider_compartment_id: None,
        wallet_compartment_id: None,
        nonce: None,
        gas_limit: None,
        broadcast: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_seed_wallet_profile_upsert_request_roundtrip() {
    let req = EthSeedWalletProfileUpsertRequest {
            name: "treasury_seed".to_string(),
            label: Some("Treasury seed wallet".to_string()),
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
            mnemonic_passphrase: Some("optional-passphrase".to_string()),
            project_account: 0,
            provider_profile: "mainnet".to_string(),
            compartment_id: Some(1),
            chain_id: Some(1),
            default_destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
            execution_enabled: Some(false),
        };
    roundtrip_test(req);
}

#[test]
fn test_eth_seed_wallet_create_request_roundtrip() {
    let req = EthSeedWalletCreateRequest {
        name: "fresh_seed".to_string(),
        label: Some("Generated treasury wallet".to_string()),
        word_count: Some(12),
        mnemonic_passphrase: Some("optional-passphrase".to_string()),
        project_account: 0,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(1),
        chain_id: Some(1),
        default_destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        execution_enabled: Some(false),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_seed_wallet_create_request_minimal() {
    let req = EthSeedWalletCreateRequest {
        name: "fresh_seed".to_string(),
        label: None,
        word_count: None,
        mnemonic_passphrase: None,
        project_account: 0,
        provider_profile: "mainnet".to_string(),
        compartment_id: None,
        chain_id: None,
        default_destination_address: None,
        execution_enabled: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("word_count"), "optional fields stay absent");
    assert!(!json.contains("mnemonic"), "no mnemonic field on create");
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_send_erc20_transfer_request_roundtrip() {
    let req = EthStealthSendErc20TransferRequest {
        rpc_url: "https://rpc.example.com".to_string(),
        wallet: "0xwallet".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: None,
            stealth_hash_convention: None,
        },
        fees: Eip1559Fees {
            chain_id: 1,
            max_priority_fee_per_gas_hex: "0x1".to_string(),
            max_fee_per_gas_hex: "0x2".to_string(),
        },
        token_address: "0x2222222222222222222222222222222222222222".to_string(),
        recipient_address: "0x4444444444444444444444444444444444444444".to_string(),
        amount_hex: "0x100".to_string(),
        auth_token_key: None,
        provider_compartment_id: None,
        wallet_compartment_id: None,
        nonce: Some(3),
        gas_limit: None,
        broadcast: Some(false),
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_provider_profile_upsert_request_full() {
    let req = EvmProviderProfileUpsertRequest {
        name: "mainnet".to_string(),
        provider: EvmProviderRef {
            rpc_url: "https://eth.example.com".to_string(),
            auth_token_key: Some("key".to_string()),
            compartment_id: Some(1),
        },
        chain_id: 1,
        max_priority_fee_per_gas_hex: Some("0x3b9aca00".to_string()),
        max_fee_per_gas_hex: Some("0x5f5e100".to_string()),
        native_gas_limit: Some(21000),
        erc20_gas_limit: Some(65000),
        fee_estimation_enabled: Some(false),
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_provider_profile_upsert_request_minimal() {
    let req = EvmProviderProfileUpsertRequest {
        name: "testnet".to_string(),
        provider: EvmProviderRef {
            rpc_url: "https://test.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        chain_id: 5,
        max_priority_fee_per_gas_hex: None,
        max_fee_per_gas_hex: None,
        native_gas_limit: None,
        erc20_gas_limit: None,
        fee_estimation_enabled: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_evm_profile_delete_request_prune_inventory_is_additive() {
    // Legacy payloads (no flag) deserialize with the cascade off...
    let legacy: EvmProfileDeleteRequest = serde_json::from_str(r#"{"name":"seed-main"}"#).unwrap();
    assert_eq!(legacy.prune_inventory, None);

    // ...and serializing without the flag stays byte-identical to legacy.
    let req = EvmProfileDeleteRequest {
        name: "seed-main".to_string(),
        prune_inventory: None,
    };
    assert_eq!(
        serde_json::to_string(&req).unwrap(),
        r#"{"name":"seed-main"}"#
    );

    roundtrip_test(EvmProfileDeleteRequest {
        name: "seed-main".to_string(),
        prune_inventory: Some(true),
    });
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_request_roundtrip() {
    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: Some("xpub-imported".to_string()),
        external_receive_path: Some("m/44'/60'/7'/1".to_string()),
        external_account_xpub: None,
        external_account_path: None,
        default_destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        execution_enabled: Some(false),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_rejects_mixed_external_xpubs() {
    use crate::validation::Validate;

    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: Some("xpub-receive".to_string()),
        external_receive_path: None,
        external_account_xpub: Some("xpub-account".to_string()),
        external_account_path: None,
        default_destination_address: None,
        execution_enabled: Some(false),
    };

    assert_eq!(
        req.validate().unwrap_err(),
        "external_receive_xpub and external_account_xpub are mutually exclusive"
    );
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_rejects_path_without_xpub() {
    use crate::validation::Validate;

    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: None,
        external_receive_path: Some("m/44'/60'/7'/1".to_string()),
        external_account_xpub: None,
        external_account_path: None,
        default_destination_address: None,
        execution_enabled: Some(false),
    };

    assert_eq!(
        req.validate().unwrap_err(),
        "external_receive_path requires external_receive_xpub"
    );
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_rejects_malformed_path() {
    use crate::validation::Validate;

    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: Some("xpub-receive".to_string()),
        external_receive_path: Some("m/44'/60'/7'".to_string()),
        external_account_xpub: None,
        external_account_path: None,
        default_destination_address: None,
        execution_enabled: Some(false),
    };

    assert_eq!(
        req.validate().unwrap_err(),
        "external_receive_path must end at a public child branch"
    );
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_rejects_account_path_without_xpub() {
    use crate::validation::Validate;

    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: None,
        external_receive_path: None,
        external_account_xpub: None,
        external_account_path: Some("m/44'/60'/7'".to_string()),
        default_destination_address: None,
        execution_enabled: Some(false),
    };

    assert_eq!(
        req.validate().unwrap_err(),
        "external_account_path requires external_account_xpub"
    );
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_accepts_hardened_account_path() {
    use crate::validation::Validate;

    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: Some(2),
        chain_id: Some(1),
        external_receive_xpub: None,
        external_receive_path: None,
        external_account_xpub: Some("xpub-account".to_string()),
        external_account_path: Some("m/44'/60'/7'".to_string()),
        default_destination_address: None,
        execution_enabled: Some(false),
    };

    req.validate().unwrap();
}

#[test]
fn test_eth_xpub_export_request_roundtrip() {
    let req = EthXpubExportRequest {
        wallet_profile: "treasury_receive".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_xpub_derive_request_roundtrip() {
    let req = EthXpubDeriveRequest {
        xpub: "xpub661MyMwAqRbcFexample".to_string(),
        index: 12,
    };
    roundtrip_test(req);
}

#[test]
fn test_wallet_inventory_scan_request_roundtrip() {
    let req = WalletInventoryScanRequest {
        wallet_family: Some("eth-seed".to_string()),
        wallet_profile: Some("seed-main".to_string()),
        provider_profile: Some("mainnet".to_string()),
        all_configured_chains: Some(false),
        derivation_pattern: Some("standard".to_string()),
        account_limit: Some(3),
        watch_addresses: vec![WatchAddressProbe {
            address: "0x7777777777777777777777777777777777777777".to_string(),
            label: Some("old-ledger".to_string()),
        }],
        include_watch_book: Some(true),
        gap_limit: Some(20),
        max_index: Some(200),
        resume_from_latest_checkpoint: Some(true),
        run_async: Some(true),
        partition_providers: Some(true),
        token_addresses: vec!["0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string()],
        block_tag: Some("latest".to_string()),
        probe_token_registry: Some(true),
        discover_erc20_transfers: Some(true),
        token_discovery_from_block: Some("0x100".to_string()),
        token_discovery_to_block: Some("latest".to_string()),
        token_discovery_limit: Some(250),
        discover_erc20_allowances: Some(true),
        allowance_spender_addresses: vec!["0x2222222222222222222222222222222222222222".to_string()],
        allowance_discovery_limit: Some(16),
        discover_permit2_allowances: Some(true),
        permit2_contract_addresses: vec!["0x000000000022d473030f116ddee9f6b43ac78ba3".to_string()],
        permit2_spender_addresses: vec!["0x4444444444444444444444444444444444444444".to_string()],
        permit2_allowance_limit: Some(32),
        discover_erc721_transfers: Some(true),
        discover_erc1155_transfers: Some(true),
        discover_nft_operator_approvals: Some(true),
        nft_operator_addresses: vec!["0x3333333333333333333333333333333333333333".to_string()],
        nft_operator_approval_limit: Some(8),
        discover_defi_token_positions: Some(true),
        defi_token_probes: vec![DefiTokenProbe {
            protocol: "aave-v3".to_string(),
            token_address: "0x4d5f47fa6a74757f35c14fd3a6ef8e3c9bc514e8".to_string(),
            protocol_address: Some("0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2".to_string()),
        }],
        defi_position_limit: Some(16),
        discover_claim_candidates: Some(true),
        claim_candidate_probes: vec![ClaimCandidateProbe {
            kind: "airdrop".to_string(),
            protocol: "optimism".to_string(),
            claimant_address: "0x9858effd232b4033e47d90003d41ec34ecaeda94".to_string(),
            claim_contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            asset_address: "0x4200000000000000000000000000000000000042".to_string(),
            amount_hex: "0xf4240".to_string(),
            source_label: "op-token-list".to_string(),
            claim_adapter: Some("merkle-distributor-v1".to_string()),
            claim_index_hex: Some("0x7".to_string()),
            claim_proof: vec![
                format!("0x{}", "11".repeat(32)),
                format!("0x{}", "22".repeat(32)),
            ],
        }],
        claim_candidate_limit: Some(12),
        nft_discovery_from_block: Some("0x100".to_string()),
        nft_discovery_to_block: Some("latest".to_string()),
        nft_discovery_limit: Some(100),
    };
    roundtrip_test(req);
}

#[test]
fn test_nft_metadata_requests_roundtrip() {
    roundtrip_test(NftMetadataOptInUpsertRequest {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
        enabled: Some(true),
    });
    roundtrip_test(NftMetadataOptInDeleteRequest {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
    });
    roundtrip_test(NftMetadataSettingsUpdateRequest {
        ipfs_gateway_url: Some("http://127.0.0.1:1/ipfs/".to_string()),
    });
    roundtrip_test(NftMetadataFetchRequest {
        chain_id: Some(1),
        contract_address: Some("0xdead00000000000000000000000000000000dead".to_string()),
        limit: Some(10),
    });
}

#[test]
fn test_nft_metadata_request_validation() {
    use crate::validation::Validate;

    let upsert = NftMetadataOptInUpsertRequest {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
        enabled: None,
    };
    assert!(upsert.validate().is_ok());

    let delete = NftMetadataOptInDeleteRequest {
        chain_id: 1,
        contract_address: "0xdead00000000000000000000000000000000dead".to_string(),
    };
    assert!(delete.validate().is_ok());

    let clear_gateway = NftMetadataSettingsUpdateRequest {
        ipfs_gateway_url: Some(String::new()),
    };
    assert!(clear_gateway.validate().is_ok());

    let fetch = NftMetadataFetchRequest {
        chain_id: Some(1),
        contract_address: Some("0xdead00000000000000000000000000000000dead".to_string()),
        limit: Some(10),
    };
    assert!(fetch.validate().is_ok());

    let invalid_upsert = NftMetadataOptInUpsertRequest {
        chain_id: 1,
        contract_address: "0xdead".to_string(),
        enabled: None,
    };
    assert!(invalid_upsert.validate().is_err());

    let invalid_fetch = NftMetadataFetchRequest {
        chain_id: None,
        contract_address: Some("not-an-address".to_string()),
        limit: None,
    };
    assert!(invalid_fetch.validate().is_err());

    let oversized_gateway = NftMetadataSettingsUpdateRequest {
        ipfs_gateway_url: Some(format!("http://127.0.0.1/{}", "a".repeat(2_048))),
    };
    assert!(oversized_gateway.validate().is_err());
}

#[test]
fn test_watch_address_book_requests_roundtrip() {
    roundtrip_test(WatchAddressBookUpsertRequest {
        address: "0x7777777777777777777777777777777777777777".to_string(),
        label: Some("old-ledger".to_string()),
        tags: vec!["client".to_string(), "hardware".to_string()],
        enabled: Some(true),
    });
    roundtrip_test(WatchAddressBookDeleteRequest {
        address: "0x7777777777777777777777777777777777777777".to_string(),
    });
}

#[test]
fn test_token_registry_requests_roundtrip() {
    roundtrip_test(TokenRegistryImportRequest {
        name: "core-list".to_string(),
        entries_json: Some(
            r#"[{"chain_id":1,"address":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","symbol":"AAA","decimals":18}]"#
                .to_string(),
        ),
        file_path: None,
    });
    roundtrip_test(TokenRegistryDeleteRequest {
        name: "core-list".to_string(),
    });
}

#[test]
fn test_chain_profile_upsert_request_roundtrip() {
    let req = ChainProfileUpsertRequest {
        name: "base".to_string(),
        chain_family: "evm".to_string(),
        chain_id: Some(8453),
        provider_profile: Some("base-mainnet".to_string()),
        native_symbol: Some("ETH".to_string()),
        native_decimals: Some(18),
        finality_blocks: Some(12),
        dormancy_block_window: Some(crate::DEFAULT_DORMANCY_BLOCK_WINDOW),
        permit2_address: Some("0x000000000022d473030f116ddee9f6b43ac78ba3".to_string()),
        uniswap_v2_router_address: Some("0xdead100e2000000000000000000000000000cccc".to_string()),
        explorer_url: Some("https://basescan.org".to_string()),
        capabilities: vec!["native".to_string(), "erc20".to_string()],
        enabled: Some(true),
        builtin: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_chain_profile_upsert_validates_uniswap_v2_router_address() {
    let mut req = ChainProfileUpsertRequest {
        name: "custom-l2".to_string(),
        chain_family: "evm".to_string(),
        chain_id: Some(7777),
        provider_profile: Some("l2".to_string()),
        native_symbol: Some("ETH".to_string()),
        native_decimals: Some(18),
        finality_blocks: Some(12),
        dormancy_block_window: Some(crate::DEFAULT_DORMANCY_BLOCK_WINDOW),
        permit2_address: None,
        uniswap_v2_router_address: Some("0xdead100e2000000000000000000000000000cccc".to_string()),
        explorer_url: None,
        capabilities: Vec::new(),
        enabled: Some(true),
        builtin: None,
    };

    req.validate().unwrap();
    req.uniswap_v2_router_address = Some("not-an-address".to_string());
    assert!(
        req.validate()
            .unwrap_err()
            .contains("uniswap_v2_router_address")
    );
}

#[test]
fn test_discovery_job_mutation_request_roundtrip() {
    roundtrip_test(DiscoveryJobMutationRequest {
        id: "job_123".to_string(),
    });
}

#[test]
fn test_risk_catalog_requests_roundtrip() {
    roundtrip_test(RiskCatalogUpsertRequest {
        address: "0x4444444444444444444444444444444444444444".to_string(),
        label: Some("Known router".to_string()),
        risk_level: "trusted".to_string(),
        notes: vec!["Operator-approved spender".to_string()],
    });
    roundtrip_test(RiskCatalogDeleteRequest {
        address: "0x4444444444444444444444444444444444444444".to_string(),
    });
}

#[test]
fn test_consolidation_plan_generate_request_roundtrip() {
    let req = ConsolidationPlanGenerateRequest {
        destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        wallet_family: Some("eth-seed".to_string()),
        wallet_profile: Some("seed-main".to_string()),
        provider_profile: Some("mainnet".to_string()),
        chain_id: Some(1),
        include_watch_only: Some(true),
        auto_queue_low_risk: Some(false),
        routing_strategy: None,
        party_destinations: Vec::new(),
    };
    roundtrip_test(req);
}

#[test]
fn test_consolidation_plan_approve_request_roundtrip() {
    let req = ConsolidationPlanApproveRequest {
        plan_id: "plan_1".to_string(),
        step_ids: vec!["step_1".to_string()],
    };
    roundtrip_test(req);
}

#[test]
fn test_consolidation_plan_simulate_request_roundtrip() {
    let req = ConsolidationPlanSimulateRequest {
        plan_id: "plan_1".to_string(),
        step_ids: vec!["step_1".to_string()],
    };
    roundtrip_test(req);
}

#[test]
fn test_consolidation_plan_export_request_roundtrip() {
    let req = ConsolidationPlanExportRequest {
        plan_id: "plan_1".to_string(),
        step_ids: vec!["step_1".to_string()],
        format: Some("safe_tx_builder".to_string()),
        safe_address: Some("0x1111111111111111111111111111111111111111".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_send_with_profile_request_roundtrip() {
    let req = EthStealthSendWithProfileRequest {
        wallet_profile: "my_profile".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: None,
            stealth_hash_convention: None,
        },
        value_wei_hex: "0x100".to_string(),
        destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        nonce: Some(5),
        gas_limit: Some(21000),
        estimate_fees: Some(true),
        broadcast: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_send_erc20_with_profile_request_roundtrip() {
    let req = EthStealthSendErc20WithProfileRequest {
        wallet_profile: "profile1".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: Some("0xff".to_string()),
            stealth_hash_convention: None,
        },
        token_address: "0x2222222222222222222222222222222222222222".to_string(),
        recipient_address: "0x4444444444444444444444444444444444444444".to_string(),
        amount_hex: "0x64".to_string(),
        nonce: None,
        gas_limit: Some(100000),
        estimate_fees: Some(false),
        broadcast: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_queue_eth_stealth_native_sweep_request_full() {
    let req = QueueEthStealthNativeSweepRequest {
        wallet_profile: "profile".to_string(),
        stealth: StealthPaymentRef {
            stealth_address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ephemeral_public_key_hex: "0xeph".to_string(),
            view_tag_hex: Some("0xaa".to_string()),
            stealth_hash_convention: None,
        },
        destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        min_value_wei_hex: Some("0x1".to_string()),
        gas_limit: Some(21000),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_create_native_request_roundtrip() {
    let req = EthStealthDepositCreateNativeRequest {
        wallet_profile: "profile1".to_string(),
        expected_value_wei_hex: Some("0x100".to_string()),
        auto_queue_sweep: Some(true),
        sweep_destination_address: Some("0x5555555555555555555555555555555555555555".to_string()),
        min_sweep_value_wei_hex: Some("0x10".to_string()),
        note: Some("test deposit".to_string()),
        ephemeral_private_key_hex: None,
        request_gas: Some(true),
        gas_amount_wei_hex: Some("0x5208".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_create_erc20_request_roundtrip() {
    let req = EthStealthDepositCreateErc20Request {
        wallet_profile: "profile2".to_string(),
        token_address: "0x2222222222222222222222222222222222222222".to_string(),
        expected_amount_hex: Some("0x1000".to_string()),
        auto_queue_sweep: Some(false),
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        note: None,
        ephemeral_private_key_hex: Some("0xkey".to_string()),
        request_gas: Some(true),
        gas_amount_wei_hex: Some("0x5208".to_string()),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_create_gas_amount_requires_request_gas() {
    let base = EthStealthDepositCreateNativeRequest {
        wallet_profile: "profile1".to_string(),
        expected_value_wei_hex: None,
        auto_queue_sweep: None,
        sweep_destination_address: None,
        min_sweep_value_wei_hex: None,
        note: None,
        ephemeral_private_key_hex: None,
        request_gas: None,
        gas_amount_wei_hex: Some("0x5208".to_string()),
    };
    assert_eq!(
        base.validate().unwrap_err(),
        "gas_amount_wei_hex requires request_gas"
    );
    let erc20 = EthStealthDepositCreateErc20Request {
        wallet_profile: "profile1".to_string(),
        token_address: "0x2222222222222222222222222222222222222222".to_string(),
        expected_amount_hex: None,
        auto_queue_sweep: None,
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        note: None,
        ephemeral_private_key_hex: None,
        request_gas: Some(false),
        gas_amount_wei_hex: Some("0x5208".to_string()),
    };
    assert_eq!(
        erc20.validate().unwrap_err(),
        "gas_amount_wei_hex requires request_gas"
    );
    let ok = EthStealthDepositCreateErc20Request {
        request_gas: Some(true),
        ..erc20
    };
    ok.validate().unwrap();
}

#[test]
fn test_eth_stealth_deposit_delete_request_roundtrip() {
    let req = EthStealthDepositDeleteRequest {
        id: "deposit_123".to_string(),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_refresh_request_all_options() {
    let req = EthStealthDepositRefreshRequest {
        id: Some("deposit_456".to_string()),
        limit: Some(10),
        auto_enqueue: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_refresh_request_empty() {
    let req = EthStealthDepositRefreshRequest {
        id: None,
        limit: None,
        auto_enqueue: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_announcement_scan_request_roundtrip() {
    let req = EthStealthAnnouncementScanRequest {
        wallet_profile: "profile2".to_string(),
        from_block: Some("0x100".to_string()),
        to_block: Some("latest".to_string()),
        token_address: Some("0x2222222222222222222222222222222222222222".to_string()),
        limit: Some(250),
        auto_queue_sweep: Some(false),
        sweep_destination_address: Some("0x000000000000000000000000000000000000dEaD".to_string()),
        min_sweep_amount_hex: Some("0x10".to_string()),
        note: Some("scan known claim window".to_string()),
        reset_cursor: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_announcement_scan_request_cursor_resume_defaults() {
    // Plan task 2.6: a cursor-resuming scan omits `from_block` entirely;
    // legacy payloads without it (and without `reset_cursor`) deserialize to
    // the same shape.
    let req = EthStealthAnnouncementScanRequest {
        wallet_profile: "profile2".to_string(),
        from_block: None,
        to_block: None,
        token_address: None,
        limit: None,
        auto_queue_sweep: None,
        sweep_destination_address: None,
        min_sweep_amount_hex: None,
        note: None,
        reset_cursor: None,
    };
    let legacy: EthStealthAnnouncementScanRequest =
        serde_json::from_value(serde_json::json!({ "wallet_profile": "profile2" })).unwrap();
    assert_eq!(req, legacy);
    roundtrip_test(req);
}

#[test]
fn test_eth_stealth_deposit_enqueue_sweep_request_roundtrip() {
    let req = EthStealthDepositEnqueueSweepRequest {
        id: "deposit_789".to_string(),
        force: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_maintenance_run_request_full() {
    let req = MaintenanceRunRequest {
        deposit_refresh_limit: Some(50),
        queue_process_limit: Some(100),
        auto_enqueue: Some(true),
        run_async: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_maintenance_run_request_empty() {
    let req = MaintenanceRunRequest {
        deposit_refresh_limit: None,
        queue_process_limit: None,
        auto_enqueue: None,
        run_async: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_queue_process_request_with_id() {
    let req = QueueProcessRequest {
        id: Some("job_123".to_string()),
        limit: Some(5),
        run_async: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_queue_process_request_empty() {
    let req = QueueProcessRequest {
        id: None,
        limit: None,
        run_async: None,
    };
    roundtrip_test(req);
}

#[test]
fn test_treasury_policy_update_request_full() {
    let req = TreasuryPolicyUpdateRequest {
        enabled: true,
        allowed_destinations: vec![
            TreasuryAllowedDestinationInput {
                address: "0x9999999999999999999999999999999999999999".to_string(),
                label: Some("cold-treasury".to_string()),
            },
            TreasuryAllowedDestinationInput {
                address: "0x8888888888888888888888888888888888888888".to_string(),
                label: None,
            },
        ],
        max_step_native_wei_hex: Some("0xde0b6b3a7640000".to_string()),
        max_plan_native_wei_hex: Some("0x1bc16d674ec80000".to_string()),
        require_simulation: Some(false),
        allow_raw_digest_signing: Some(true),
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
        hot_overflow_wei_hex: Some("0x1bc16d674ec80000".to_string()),
        allow_treasury_automation: Some(true),
    };
    roundtrip_test(req);
}

#[test]
fn test_treasury_policy_update_request_minimal() {
    let req = TreasuryPolicyUpdateRequest {
        enabled: false,
        allowed_destinations: Vec::new(),
        max_step_native_wei_hex: None,
        max_plan_native_wei_hex: None,
        require_simulation: None,
        allow_raw_digest_signing: None,
        block_cross_party_linkage: None,
        allow_claim_execution: None,
        allow_gas_topups: None,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: None,
        allow_sweep_execution: None,
        allow_revoke_execution: None,
        allow_exit_execution: None,
        execution_paused: None,
        max_fee_per_gas_cap_hex: None,
        simulation_freshness_secs: None,
        hot_floor_wei_hex: None,
        hot_target_wei_hex: None,
        hot_overflow_wei_hex: None,
        allow_treasury_automation: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("allow_plan_execution"));
    assert!(!json.contains("allow_sweep_execution"));
    assert!(!json.contains("allow_revoke_execution"));
    assert!(!json.contains("allow_exit_execution"));
    assert!(!json.contains("execution_paused"));
    assert!(!json.contains("max_fee_per_gas_cap_hex"));
    assert!(!json.contains("hot_overflow_wei_hex"));
    assert!(!json.contains("allow_treasury_automation"));

    let omitted: TreasuryPolicyUpdateRequest =
        serde_json::from_str(r#"{"enabled":false}"#).unwrap();
    assert_eq!(omitted.allow_plan_execution, None);
    assert_eq!(omitted.allow_sweep_execution, None);
    assert_eq!(omitted.allow_revoke_execution, None);
    assert_eq!(omitted.allow_exit_execution, None);
    assert_eq!(omitted.execution_paused, None);
    assert_eq!(omitted.max_fee_per_gas_cap_hex, None);
    assert_eq!(omitted.hot_overflow_wei_hex, None);
    assert_eq!(omitted.allow_treasury_automation, None);

    roundtrip_test(req);
}

#[test]
fn test_treasury_receive_allocate_request_roundtrip() {
    roundtrip_test(TreasuryReceiveAllocateRequest {
        wallet_profile: "seed-main".to_string(),
        purpose: "counterparty-acme".to_string(),
        label: Some("Acme invoices".to_string()),
        counterparty_id: None,
    });
    roundtrip_test(TreasuryReceiveAllocateRequest {
        wallet_profile: "seed-main".to_string(),
        purpose: "grant-payout".to_string(),
        label: None,
        counterparty_id: None,
    });
    roundtrip_test(TreasuryReceiveAllocateRequest {
        wallet_profile: "seed-main".to_string(),
        purpose: "counterparty-acme".to_string(),
        label: None,
        counterparty_id: Some("cp_1".to_string()),
    });
}

#[test]
fn test_treasury_receive_rotate_request_roundtrip() {
    roundtrip_test(TreasuryReceiveRotateRequest {
        allocation_id: "alloc_123".to_string(),
    });
}

#[test]
fn test_treasury_receive_purge_request_roundtrip() {
    roundtrip_test(TreasuryReceivePurgeRequest {
        allocation_id: "alloc_123".to_string(),
    });
}

#[test]
fn test_wallet_inventory_address_prune_request_roundtrip_and_validation() {
    roundtrip_test(WalletInventoryAddressPruneRequest {
        address: Some("0x1111111111111111111111111111111111111111".to_string()),
        wallet_family: Some("eth-seed".to_string()),
        wallet_profile: Some("seed-main".to_string()),
        provider_profile: Some("mainnet".to_string()),
        chain_id: Some(1),
        account_index: Some(0),
    });

    let by_profile = WalletInventoryAddressPruneRequest {
        wallet_profile: Some("seed-main".to_string()),
        ..WalletInventoryAddressPruneRequest::default()
    };
    assert!(by_profile.validate().is_ok());
    let by_account = WalletInventoryAddressPruneRequest {
        account_index: Some(3),
        ..WalletInventoryAddressPruneRequest::default()
    };
    assert!(by_account.validate().is_ok());

    // Fail closed: no selector, a malformed address, or an unknown family.
    assert!(
        WalletInventoryAddressPruneRequest::default()
            .validate()
            .is_err()
    );
    let bad_address = WalletInventoryAddressPruneRequest {
        address: Some("not-an-address".to_string()),
        ..WalletInventoryAddressPruneRequest::default()
    };
    assert!(bad_address.validate().is_err());
    let bad_family = WalletInventoryAddressPruneRequest {
        wallet_family: Some("btc".to_string()),
        ..WalletInventoryAddressPruneRequest::default()
    };
    assert!(bad_family.validate().is_err());
}

#[test]
fn test_counterparty_create_request_roundtrip() {
    roundtrip_test(CounterpartyCreateRequest {
        name: "Acme".into(),
        note: Some("net-30".into()),
        sweep_destination_address: Some("0x1111111111111111111111111111111111111111".into()),
    });
    roundtrip_test(CounterpartyCreateRequest {
        name: "Beta".into(),
        note: None,
        sweep_destination_address: None,
    });
}

#[test]
fn test_counterparty_update_request_roundtrip() {
    roundtrip_test(CounterpartyUpdateRequest {
        id: "cp_1".into(),
        name: "Acme".into(),
        note: None,
        sweep_destination_address: Some("0x6666666666666666666666666666666666666666".into()),
    });
}

#[test]
fn test_counterparty_delete_request_roundtrip() {
    roundtrip_test(CounterpartyDeleteRequest { id: "cp_1".into() });
}

#[test]
fn test_self_check_run_request_roundtrip() {
    roundtrip_test(SelfCheckRunRequest {
        domains: vec!["provider".to_string(), "policy".to_string()],
    });
}

#[test]
fn test_self_check_run_request_empty_domains_default() {
    // An empty body selects every domain and the field is skipped on the wire.
    let req: SelfCheckRunRequest = serde_json::from_str("{}").unwrap();
    assert!(req.domains.is_empty());
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("domains"));
    roundtrip_test(req);
}

#[test]
fn test_self_check_run_request_validation() {
    use crate::validation::Validate;

    for domain in crate::validation::SELF_CHECK_DOMAINS {
        let req = SelfCheckRunRequest {
            domains: vec![domain.to_string()],
        };
        assert!(req.validate().is_ok(), "domain {domain} should validate");
    }

    let empty = SelfCheckRunRequest::default();
    assert!(empty.validate().is_ok());

    let unknown = SelfCheckRunRequest {
        domains: vec!["bogus".to_string()],
    };
    assert!(unknown.validate().unwrap_err().contains("bogus"));

    let oversize = SelfCheckRunRequest {
        domains: vec!["p".repeat(65)],
    };
    assert!(oversize.validate().is_err());
}

// ── Field-level validation (validate_fields) ─────────────────────

#[test]
fn test_validate_fields_default_wraps_single_string_without_fields() {
    // DTOs that only implement `validate()` report one failure with no
    // per-field breakdown.
    let req = PassphraseRequest {
        passphrase: "x".repeat(2048),
    };
    let failure = req.validate_fields().unwrap_err();
    assert!(failure.message().contains("passphrase"));
    assert!(failure.fields().is_empty());
}

#[test]
fn test_evm_provider_profile_upsert_fields_accumulate_with_flattened_paths() {
    let req = EvmProviderProfileUpsertRequest {
        name: "n".repeat(300),
        provider: EvmProviderRef {
            rpc_url: "u".repeat(2100),
            auth_token_key: Some("k".repeat(600)),
            compartment_id: None,
        },
        chain_id: 1,
        max_priority_fee_per_gas_hex: None,
        max_fee_per_gas_hex: Some("f".repeat(4100)),
        native_gas_limit: None,
        erc20_gas_limit: None,
        fee_estimation_enabled: None,
    };
    let failure = req.validate_fields().unwrap_err();
    let paths: Vec<&str> = failure.fields().iter().map(|f| f.field.as_str()).collect();
    // `provider` is #[serde(flatten)] on the wire, so rpc_url/auth_token_key
    // stay top-level field paths.
    assert_eq!(
        paths,
        vec!["name", "rpc_url", "auth_token_key", "max_fee_per_gas_hex"]
    );
    // Legacy single-string contract: first field message, byte-identical.
    assert_eq!(req.validate().unwrap_err(), failure.message());
    assert!(failure.fields()[0].message.contains("name"));
}

#[test]
fn test_eth_xpub_wallet_profile_upsert_fields_cover_cross_field_rules() {
    let req = EthXpubWalletProfileUpsertRequest {
        name: "treasury_receive".to_string(),
        project_account: 7,
        provider_profile: "mainnet".to_string(),
        compartment_id: None,
        chain_id: None,
        external_receive_xpub: None,
        external_receive_path: Some("m/44'/60'/7'/1".to_string()),
        external_account_xpub: None,
        external_account_path: Some("m/44'/60'/7'".to_string()),
        default_destination_address: Some("not-an-address".to_string()),
        execution_enabled: None,
    };
    let failure = req.validate_fields().unwrap_err();
    let paths: Vec<&str> = failure.fields().iter().map(|f| f.field.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "external_receive_path",
            "external_account_path",
            "external_receive_path",
            "default_destination_address"
        ]
    );
    assert_eq!(
        failure.fields()[0].message,
        "external_receive_path requires external_receive_xpub"
    );
    assert_eq!(
        failure.fields()[2].message,
        "external_receive_path and external_account_path are mutually exclusive"
    );
    // Single-error case keeps the historical message exactly.
    assert_eq!(
        req.validate().unwrap_err(),
        "external_receive_path requires external_receive_xpub"
    );
}

#[test]
fn test_eth_seed_wallet_profile_upsert_fields_accumulate() {
    let req = EthSeedWalletProfileUpsertRequest {
        name: "n".repeat(300),
        label: None,
        mnemonic: "abandon abandon abandon".to_string(),
        mnemonic_passphrase: None,
        project_account: 0,
        provider_profile: "mainnet".to_string(),
        compartment_id: None,
        chain_id: None,
        default_destination_address: Some("0xnothex".to_string()),
        execution_enabled: None,
    };
    let failure = req.validate_fields().unwrap_err();
    let paths: Vec<&str> = failure.fields().iter().map(|f| f.field.as_str()).collect();
    assert_eq!(paths, vec!["name", "default_destination_address"]);
    assert!(
        failure.fields()[1]
            .message
            .contains("must be a valid ethereum address")
    );
}

#[test]
fn test_wallet_inventory_scan_fields_use_indexed_paths() {
    let req = WalletInventoryScanRequest {
        wallet_family: None,
        wallet_profile: None,
        provider_profile: Some("mainnet".to_string()),
        all_configured_chains: Some(true),
        derivation_pattern: None,
        account_limit: None,
        watch_addresses: vec![WatchAddressProbe {
            address: "0xnothex".to_string(),
            label: None,
        }],
        include_watch_book: None,
        gap_limit: None,
        max_index: None,
        resume_from_latest_checkpoint: None,
        run_async: None,
        partition_providers: None,
        token_addresses: vec![
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
            "0xtooshort".to_string(),
        ],
        block_tag: None,
        probe_token_registry: None,
        discover_erc20_transfers: None,
        token_discovery_from_block: None,
        token_discovery_to_block: None,
        token_discovery_limit: None,
        discover_erc20_allowances: None,
        allowance_spender_addresses: Vec::new(),
        allowance_discovery_limit: None,
        discover_permit2_allowances: None,
        permit2_contract_addresses: Vec::new(),
        permit2_spender_addresses: Vec::new(),
        permit2_allowance_limit: None,
        discover_erc721_transfers: None,
        discover_erc1155_transfers: None,
        discover_nft_operator_approvals: None,
        nft_operator_addresses: Vec::new(),
        nft_operator_approval_limit: None,
        discover_defi_token_positions: None,
        defi_token_probes: Vec::new(),
        defi_position_limit: None,
        discover_claim_candidates: None,
        claim_candidate_probes: Vec::new(),
        claim_candidate_limit: None,
        nft_discovery_from_block: None,
        nft_discovery_to_block: None,
        nft_discovery_limit: None,
    };
    let failure = req.validate_fields().unwrap_err();
    let paths: Vec<&str> = failure.fields().iter().map(|f| f.field.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "provider_profile",
            "token_addresses[1]",
            "watch_addresses[0].address"
        ]
    );
    assert_eq!(
        failure.fields()[0].message,
        "provider_profile cannot be combined with all_configured_chains"
    );
    assert_eq!(req.validate().unwrap_err(), failure.message());
}

#[test]
fn test_treasury_policy_update_fields_use_indexed_paths() {
    let req = TreasuryPolicyUpdateRequest {
        enabled: true,
        allowed_destinations: vec![
            TreasuryAllowedDestinationInput {
                address: " ".to_string(),
                label: None,
            },
            TreasuryAllowedDestinationInput {
                address: "0xnothex".to_string(),
                label: Some("l".repeat(300)),
            },
        ],
        max_step_native_wei_hex: Some("h".repeat(4100)),
        max_plan_native_wei_hex: None,
        require_simulation: None,
        allow_raw_digest_signing: None,
        block_cross_party_linkage: None,
        allow_claim_execution: None,
        allow_gas_topups: None,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: None,
        allow_sweep_execution: None,
        allow_revoke_execution: None,
        allow_exit_execution: None,
        execution_paused: None,
        max_fee_per_gas_cap_hex: None,
        simulation_freshness_secs: None,
        hot_floor_wei_hex: None,
        hot_target_wei_hex: None,
        hot_overflow_wei_hex: None,
        allow_treasury_automation: None,
    };
    let failure = req.validate_fields().unwrap_err();
    let paths: Vec<&str> = failure.fields().iter().map(|f| f.field.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "allowed_destinations[0].address",
            "allowed_destinations[1].address",
            "allowed_destinations[1].label",
            "max_step_native_wei_hex"
        ]
    );
    assert_eq!(
        failure.fields()[0].message,
        "allowed_destinations[0].address must not be empty"
    );
}

#[test]
fn test_validate_fields_pass_for_valid_dtos() {
    let provider = EvmProviderProfileUpsertRequest {
        name: "mainnet".to_string(),
        provider: EvmProviderRef {
            rpc_url: "https://eth.example.com".to_string(),
            auth_token_key: None,
            compartment_id: None,
        },
        chain_id: 1,
        max_priority_fee_per_gas_hex: None,
        max_fee_per_gas_hex: None,
        native_gas_limit: None,
        erc20_gas_limit: None,
        fee_estimation_enabled: None,
    };
    assert!(provider.validate_fields().is_ok());

    let policy = TreasuryPolicyUpdateRequest {
        enabled: true,
        allowed_destinations: vec![TreasuryAllowedDestinationInput {
            address: "0x9999999999999999999999999999999999999999".to_string(),
            label: Some("cold".to_string()),
        }],
        max_step_native_wei_hex: None,
        max_plan_native_wei_hex: None,
        require_simulation: None,
        allow_raw_digest_signing: None,
        block_cross_party_linkage: None,
        allow_claim_execution: None,
        allow_gas_topups: None,
        max_gas_topup_wei_hex: None,
        allow_plan_execution: None,
        allow_sweep_execution: None,
        allow_revoke_execution: None,
        allow_exit_execution: None,
        execution_paused: None,
        max_fee_per_gas_cap_hex: None,
        simulation_freshness_secs: None,
        hot_floor_wei_hex: None,
        hot_target_wei_hex: None,
        hot_overflow_wei_hex: None,
        allow_treasury_automation: None,
    };
    assert!(policy.validate_fields().is_ok());
}

// ── List pagination / filtering / sorting ──────────────────────────

#[test]
fn test_pagination_query_roundtrip() {
    roundtrip_test(PaginationQuery {
        limit: Some(50),
        offset: Some(100),
    });
    roundtrip_test(PaginationQuery::default());
}

#[test]
fn test_list_options_roundtrip() {
    roundtrip_test(QueueJobListOptions {
        page: PaginationQuery {
            limit: Some(25),
            offset: None,
        },
        state: Some("queued".to_string()),
        kind: Some("plan_step_execution".to_string()),
        chain_id: Some(1),
        sort: Some("created".to_string()),
        order: Some("desc".to_string()),
    });
    roundtrip_test(WalletInventoryListOptions {
        chain_id: Some(1),
        funded: Some(true),
        sort: Some("last_scanned".to_string()),
        ..Default::default()
    });
    roundtrip_test(EthStealthDepositListOptions {
        status: Some("sweep_queued".to_string()),
        counterparty_id: Some("party_client_alpha".to_string()),
        ..Default::default()
    });
    roundtrip_test(ConsolidationPlanListOptions {
        status: Some("review_required".to_string()),
        sort: Some("updated".to_string()),
        order: Some("asc".to_string()),
        ..Default::default()
    });
    roundtrip_test(RiskFindingListOptions {
        severity: Some("critical".to_string()),
        kind: Some("risky_approval".to_string()),
        chain_id: Some(1),
        sort: Some("severity".to_string()),
        ..Default::default()
    });
    roundtrip_test(DiscoveryJobListOptions {
        state: Some("running".to_string()),
        sort: Some("created".to_string()),
        ..Default::default()
    });
}

#[test]
fn test_list_options_default_serializes_to_empty_object() {
    // A default options struct adds no query parameters: the legacy
    // request shape (and therefore the legacy response) is unchanged.
    assert_eq!(
        serde_json::to_value(QueueJobListOptions::default()).unwrap(),
        serde_json::json!({})
    );
    assert_eq!(
        serde_json::to_value(PaginationQuery::default()).unwrap(),
        serde_json::json!({})
    );
}
