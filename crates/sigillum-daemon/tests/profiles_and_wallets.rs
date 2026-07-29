mod common;

use common::mock_evm::spawn_mock_evm_provider;
use common::{get, post_json, spawn_daemon};
use std::collections::HashSet;

use reqwest::StatusCode;
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn xpub_profiles_export_and_derive_receive_addresses() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:8545/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    let mixed_external_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "mixed-external",
            "project_account": 7,
            "provider_profile": "mainnet",
            "external_receive_xpub": "xpub-receive",
            "external_account_xpub": "xpub-account",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(mixed_external_profile.status(), StatusCode::BAD_REQUEST);

    let path_without_xpub = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "path-without-xpub",
            "project_account": 7,
            "provider_profile": "mainnet",
            "external_receive_path": "m/44'/60'/7'/1",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(path_without_xpub.status(), StatusCode::BAD_REQUEST);

    let custom_export =
        sigillum_core::derive_ethereum_xpub_control_branch_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            None,
            7,
        )
        .unwrap();
    let custom_xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "custom-control",
            "project_account": 99,
            "provider_profile": "mainnet",
            "external_receive_xpub": custom_export.receive_xpub.clone(),
            "external_receive_path": custom_export.receive_path.clone(),
            "execution_enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_xpub_profile.status(), StatusCode::OK);
    let custom_xpub_profile_json: serde_json::Value = custom_xpub_profile.json().await.unwrap();
    assert_eq!(
        custom_xpub_profile_json["profile"]["external_receive_path"],
        custom_export.receive_path
    );
    assert_eq!(
        custom_xpub_profile_json["profile"]["execution_enabled"],
        false
    );

    let xpub_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-xpub/upsert",
        json!({
            "name": "treasury-receive",
            "project_account": 7,
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(xpub_profile.status(), StatusCode::OK);
    let xpub_profile_json: serde_json::Value = xpub_profile.json().await.unwrap();
    assert_eq!(xpub_profile_json["profile"]["project_account"], 7);

    let list = get(&client, addr, "/api/profiles/eth-xpub", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    let profile_names: Vec<&str> = list_json["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|profile| profile["name"].as_str())
        .collect();
    assert!(profile_names.contains(&"custom-control"));
    assert!(profile_names.contains(&"treasury-receive"));

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/export",
        json!({ "wallet_profile": "treasury-receive" }),
        Some(&token),
    )
    .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json: serde_json::Value = export.json().await.unwrap();
    let receive_xpub = export_json["receive_xpub"].as_str().unwrap().to_string();
    assert_eq!(export_json["wallet_profile"], "treasury-receive");
    assert_eq!(export_json["project_account"], 7);
    assert_eq!(export_json["account_path"], "m/44'/60'/7'");
    assert_eq!(export_json["receive_path"], "m/44'/60'/7'/0");
    assert!(receive_xpub.starts_with("xpub"));

    let custom_export_resp = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/export",
        json!({ "wallet_profile": "custom-control" }),
        Some(&token),
    )
    .await;
    assert_eq!(custom_export_resp.status(), StatusCode::OK);
    let custom_export_json: serde_json::Value = custom_export_resp.json().await.unwrap();
    assert_eq!(custom_export_json["wallet_profile"], "custom-control");
    assert_eq!(custom_export_json["project_account"], 99);
    assert_eq!(
        custom_export_json["account_path"],
        custom_export.account_path
    );
    assert_eq!(
        custom_export_json["receive_path"],
        custom_export.receive_path
    );
    assert_eq!(
        custom_export_json["receive_xpub"],
        custom_export.receive_xpub
    );

    let derive_zero = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/derive",
        json!({
            "xpub": receive_xpub,
            "index": 0,
        }),
        None,
    )
    .await;
    assert_eq!(derive_zero.status(), StatusCode::OK);
    let derive_zero_json: serde_json::Value = derive_zero.json().await.unwrap();
    let address_zero = derive_zero_json["address"].as_str().unwrap().to_string();
    assert_eq!(derive_zero_json["index"], 0);
    assert!(address_zero.starts_with("0x"));
    assert_eq!(address_zero.len(), 42);

    let derive_one = post_json(
        &client,
        addr,
        "/api/wallets/eth-xpub/derive",
        json!({
            "xpub": export_json["receive_xpub"],
            "index": 1,
        }),
        None,
    )
    .await;
    assert_eq!(derive_one.status(), StatusCode::OK);
    let derive_one_json: serde_json::Value = derive_one.json().await.unwrap();
    assert_ne!(derive_one_json["address"], address_zero);

    let audit = get(&client, addr, "/api/audit?limit=20", Some(&token)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: serde_json::Value = audit.json().await.unwrap();
    assert!(
        audit_json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["kind"] == "wallet.eth_xpub.export")
    );

    handle.abort();
}

#[tokio::test]
async fn seed_wallet_profiles_import_12_and_24_word_phrases() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:8545/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    let twelve_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let seed_12 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-12",
            "label": "Twelve word",
            "mnemonic": twelve_word,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed_12.status(), StatusCode::OK);
    let seed_12_json: serde_json::Value = seed_12.json().await.unwrap();
    assert_eq!(seed_12_json["profile"]["word_count"], 12);
    assert_eq!(seed_12_json["profile"]["account_path"], "m/44'/60'/0'");
    assert!(
        seed_12_json["profile"]["receive_xpub"]
            .as_str()
            .unwrap()
            .starts_with("xpub")
    );
    assert_eq!(
        seed_12_json["profile"]["first_receive_address"]
            .as_str()
            .unwrap()
            .len(),
        42
    );

    let twenty_four_word = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    let seed_24 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-24",
            "label": "Twenty four word",
            "mnemonic": twenty_four_word,
            "mnemonic_passphrase": "optional",
            "project_account": 1,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(seed_24.status(), StatusCode::OK);
    let seed_24_json: serde_json::Value = seed_24.json().await.unwrap();
    assert_eq!(seed_24_json["profile"]["word_count"], 24);
    assert_eq!(seed_24_json["profile"]["account_path"], "m/44'/60'/1'");

    let list = get(&client, addr, "/api/profiles/eth-seed", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    let profiles = list_json["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["name"], "seed-12");
    assert_eq!(profiles[1]["name"], "seed-24");

    let invalid = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/upsert",
        json!({
            "name": "seed-invalid",
            "mnemonic": "abandon abandon",
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let delete = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/delete",
        json!({ "name": "seed-12" }),
        Some(&token),
    )
    .await;
    assert_eq!(delete.status(), StatusCode::OK);

    handle.abort();
}

/// Collect lowercase word tokens from every JSON string *value*, skipping
/// `kind` discriminants (compile-time constants of the audit schema, which
/// legitimately contain words like "seed" and "wallet"). Any mnemonic leak
/// would have to travel through a dynamic string value, so scanning these
/// tokens proves the audit feed carries no mnemonic words.
fn collect_audit_value_tokens(value: &serde_json::Value, tokens: &mut HashSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
                if !token.is_empty() {
                    tokens.insert(token.to_ascii_lowercase());
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_audit_value_tokens(item, tokens);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                if key == "kind" {
                    continue;
                }
                collect_audit_value_tokens(item, tokens);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn seed_wallet_create_generates_mnemonic_and_returns_it_exactly_once() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    let provider = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": "http://127.0.0.1:8545/",
            "chain_id": 1,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider.status(), StatusCode::OK);

    // Create a wallet with an explicit word_count of 12.
    let create_12 = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-12",
            "word_count": 12,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(create_12.status(), StatusCode::OK);
    let create_12_json: serde_json::Value = create_12.json().await.unwrap();
    assert_eq!(create_12_json["status"], "created");
    let mnemonic = create_12_json["mnemonic"].as_str().unwrap().to_string();
    assert_eq!(mnemonic.split_whitespace().count(), 12);

    // The returned phrase is valid BIP-39 and reproduces the stored profile
    // material when derived independently through sigillum-core.
    assert_eq!(
        sigillum_core::ethereum_mnemonic_word_count(&mnemonic).unwrap(),
        12
    );
    let export =
        sigillum_core::derive_ethereum_xpub_receive_branch_from_mnemonic(&mnemonic, None, 0)
            .unwrap();
    assert_eq!(
        create_12_json["profile"]["receive_xpub"],
        json!(export.receive_xpub)
    );
    let derived_first_address =
        sigillum_core::derive_ethereum_address_from_xpub(&export.receive_xpub, 0)
            .unwrap()
            .address;
    assert_eq!(
        create_12_json["profile"]["first_receive_address"],
        json!(derived_first_address)
    );
    assert_eq!(create_12_json["profile"]["word_count"], 12);
    assert_eq!(create_12_json["profile"]["account_path"], "m/44'/60'/0'");
    assert_eq!(
        create_12_json["profile"]["mnemonic_secret_key"],
        "wallet.seed.gen-12.mnemonic"
    );

    // The created profile appears in the list with the matching address.
    let list = get(&client, addr, "/api/profiles/eth-seed", Some(&token)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let list_json: serde_json::Value = list.json().await.unwrap();
    let listed = list_json["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["name"] == "gen-12")
        .expect("created profile is listed");
    assert_eq!(
        listed["first_receive_address"],
        json!(derived_first_address)
    );

    // Creating the same name again must fail instead of overwriting.
    let duplicate = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-12",
            "word_count": 12,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    // Omitting word_count defaults to a 24-word mnemonic.
    let create_default = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-default",
            "project_account": 1,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(create_default.status(), StatusCode::OK);
    let create_default_json: serde_json::Value = create_default.json().await.unwrap();
    assert_eq!(create_default_json["status"], "created");
    let default_mnemonic = create_default_json["mnemonic"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(default_mnemonic.split_whitespace().count(), 24);
    assert_eq!(create_default_json["profile"]["word_count"], 24);
    assert_ne!(default_mnemonic, mnemonic);

    // Unsupported word counts are rejected up front.
    let invalid = post_json(
        &client,
        addr,
        "/api/profiles/eth-seed/create",
        json!({
            "name": "gen-invalid",
            "word_count": 13,
            "project_account": 0,
            "provider_profile": "mainnet",
        }),
        Some(&token),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    // The audit feed records the create event with metadata only.
    let audit = get(&client, addr, "/api/audit?limit=50", Some(&token)).await;
    assert_eq!(audit.status(), StatusCode::OK);
    let audit_json: serde_json::Value = audit.json().await.unwrap();
    let events = audit_json["events"].as_array().unwrap();
    let create_event = events
        .iter()
        .find(|event| {
            event["kind"] == "profiles.eth_seed_wallet.create"
                && event["details"]["name"] == "gen-12"
        })
        .expect("create audit event present");
    assert_eq!(
        create_event["details"],
        json!({
            "name": "gen-12",
            "provider_profile": "mainnet",
            "word_count": 12,
        })
    );

    // No dynamic string value anywhere in the audit feed contains any word of
    // either generated mnemonic.
    let mut value_tokens = HashSet::new();
    collect_audit_value_tokens(&audit_json, &mut value_tokens);
    for word in mnemonic
        .split_whitespace()
        .chain(default_mnemonic.split_whitespace())
    {
        assert!(
            !value_tokens.contains(word),
            "audit feed leaked mnemonic word: {word}"
        );
    }

    handle.abort();
}

#[tokio::test]
async fn profile_backed_send_and_queue_flow_persist_internal_configuration() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;

    let provider_profile = post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(provider_profile.status(), StatusCode::OK);

    let wallet_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111",
            "execution_enabled": true,
        }),
        Some(&token),
    )
    .await;
    assert_eq!(wallet_profile.status(), StatusCode::OK);

    let providers = get(&client, addr, "/api/profiles/evm", Some(&token)).await;
    let providers_json: serde_json::Value = providers.json().await.unwrap();
    assert_eq!(providers_json["profiles"][0]["name"], "mainnet");

    let wallets = get(&client, addr, "/api/profiles/eth-stealth", Some(&token)).await;
    let wallets_json: serde_json::Value = wallets.json().await.unwrap();
    assert_eq!(wallets_json["profiles"][0]["name"], "payments-mainnet");

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth",
        }),
        Some(&token),
    )
    .await;
    let export_json: serde_json::Value = export.json().await.unwrap();

    let generate = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": export_json["stealth_meta_address"],
            "ephemeral_private_key_hex": hex::encode([7u8; 32]),
        }),
        None,
    )
    .await;
    let generate_json: serde_json::Value = generate.json().await.unwrap();

    let disabled_wallet_profile = post_json(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-disabled",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(disabled_wallet_profile.status(), StatusCode::OK);
    let disabled_enqueue = post_json(
        &client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-disabled",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0x1"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(disabled_enqueue.status(), StatusCode::FORBIDDEN);
    let disabled_enqueue_json: serde_json::Value = disabled_enqueue.json().await.unwrap();
    assert_eq!(
        disabled_enqueue_json["error"],
        "Wallet profile execution is disabled."
    );

    let send = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-with-profile",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000",
            "broadcast": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send.status(), StatusCode::OK);
    let send_json: serde_json::Value = send.json().await.unwrap();
    assert_eq!(send_json["kind"], "eth-transfer");
    assert_eq!(
        send_json["to_address"],
        "0x1111111111111111111111111111111111111111"
    );

    let enqueue = post_json(
        &client,
        addr,
        "/api/queue/enqueue/eth-stealth-transfer",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(enqueue.status(), StatusCode::OK);
    let enqueue_json: serde_json::Value = enqueue.json().await.unwrap();
    let job_id = enqueue_json["job"]["id"].as_str().unwrap().to_string();

    let list_before = get(&client, addr, "/api/queue/jobs", Some(&token)).await;
    let list_before_json: serde_json::Value = list_before.json().await.unwrap();
    assert_eq!(list_before_json["jobs"][0]["state"], "queued");

    let process = post_json(
        &client,
        addr,
        "/api/queue/process",
        json!({ "id": job_id }),
        Some(&token),
    )
    .await;
    assert_eq!(process.status(), StatusCode::OK);
    let process_json: serde_json::Value = process.json().await.unwrap();
    assert_eq!(process_json["succeeded"], 1);
    assert_eq!(process_json["failures_by_cause"]["provider_error"], 0);
    assert_eq!(process_json["jobs"][0]["state"], "sent");

    let list_after = get(&client, addr, "/api/queue/jobs", Some(&token)).await;
    let list_after_json: serde_json::Value = list_after.json().await.unwrap();
    assert_eq!(list_after_json["jobs"][0]["state"], "sent");
    assert_eq!(
        list_after_json["jobs"][0]["broadcast_transaction_hash_hex"],
        list_after_json["jobs"][0]["transaction_hash_hex"]
    );

    handle.abort();
    rpc_handle.abort();
}

#[tokio::test]
async fn profile_bound_wallet_and_provider_work_after_session_switches_compartments() {
    let dir = TempDir::new().unwrap();
    let (addr, handle) = spawn_daemon(dir.path().to_path_buf()).await;
    let (rpc_addr, rpc_handle) = spawn_mock_evm_provider().await;
    let client = reqwest::Client::new();

    let init = post_json(
        &client,
        addr,
        "/api/compartment/init",
        json!({
            "id": 0,
            "label": "default",
            "threshold": 1,
            "passphrase": "correct horse battery staple",
        }),
        None,
    )
    .await;
    let init_json: serde_json::Value = init.json().await.unwrap();
    let token = init_json["session_token"].as_str().unwrap().to_string();

    post_json(
        &client,
        addr,
        "/api/api-keys/set",
        json!({ "key": "alchemy", "value": "rpc-test-token" }),
        Some(&token),
    )
    .await;

    post_json(
        &client,
        addr,
        "/api/profiles/evm/upsert",
        json!({
            "name": "mainnet",
            "rpc_url": format!("http://{rpc_addr}/"),
            "auth_token_key": "alchemy",
            "chain_id": 1,
            "max_priority_fee_per_gas_hex": "0x59682f00",
            "max_fee_per_gas_hex": "0x77359400",
            "native_gas_limit": 21000,
            "erc20_gas_limit": 65000
        }),
        Some(&token),
    )
    .await;

    post_json(
        &client,
        addr,
        "/api/profiles/eth-stealth/upsert",
        json!({
            "name": "payments-mainnet",
            "wallet": "payments",
            "short_name": "eth",
            "provider_profile": "mainnet",
            "default_destination_address": "0x1111111111111111111111111111111111111111"
        }),
        Some(&token),
    )
    .await;

    let export = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/export",
        json!({
            "wallet": "payments",
            "short_name": "eth"
        }),
        Some(&token),
    )
    .await;
    let export_json: serde_json::Value = export.json().await.unwrap();

    let generate = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/generate",
        json!({
            "stealth_meta_address": export_json["stealth_meta_address"],
            "ephemeral_private_key_hex": hex::encode([5u8; 32]),
        }),
        None,
    )
    .await;
    let generate_json: serde_json::Value = generate.json().await.unwrap();

    let add = post_json(
        &client,
        addr,
        "/api/compartment/add",
        json!({
            "label": "secure",
            "threshold": 2,
            "passphrase_mode": "wrapped"
        }),
        Some(&token),
    )
    .await;
    assert_eq!(add.status(), StatusCode::OK);
    let add_json: serde_json::Value = add.json().await.unwrap();
    assert_eq!(add_json["id"], 1);

    let switch = post_json(
        &client,
        addr,
        "/api/compartment/switch",
        json!({ "id": 1 }),
        Some(&token),
    )
    .await;
    assert_eq!(switch.status(), StatusCode::OK);

    let send = post_json(
        &client,
        addr,
        "/api/wallets/eth-stealth/send-with-profile",
        json!({
            "wallet_profile": "payments-mainnet",
            "stealth_address": generate_json["stealth_address"],
            "ephemeral_public_key_hex": generate_json["ephemeral_public_key_hex"],
            "view_tag_hex": generate_json["view_tag_hex"],
            "value_wei_hex": "0xde0b6b3a7640000",
            "broadcast": false
        }),
        Some(&token),
    )
    .await;
    assert_eq!(send.status(), StatusCode::OK);
    let send_json: serde_json::Value = send.json().await.unwrap();
    assert_eq!(send_json["kind"], "eth-transfer");
    assert_eq!(
        send_json["to_address"],
        "0x1111111111111111111111111111111111111111"
    );

    handle.abort();
    rpc_handle.abort();
}
