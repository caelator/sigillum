//! Inventory daemon API commands.

use std::collections::BTreeMap;
use std::process;

use sigillum_api::request::{
    ChainProfileUpsertRequest, TokenRegistryImportRequest, WalletInventoryAddressPruneRequest,
    WalletInventoryScanRequest, WatchAddressBookUpsertRequest,
};

use super::inventory_args::{
    parse_claim_candidate_probes, parse_defi_token_probes, parse_watch_address_probes,
};
use super::{
    bool_switch, build_client, flag_option, parse_flag, parse_multi_flag, parse_u32_flag,
    parse_u64_flag, parse_usize_flag, print_json, report_client_error, require_flag,
    run_api_command,
};

const USAGE: &str = "Usage: sigillum api inventory <list|chains|watch|token-registry|scan-evm|prune-addresses> [...]";

/// Dispatch `sigillum api inventory <list|chains|watch|token-registry|scan-evm|prune-addresses>`.
pub(super) fn cmd_api_inventory(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "list" => list_inventory_with_chain_labels(args),
        "chains" => cmd_chains(args, 2, "sigillum api inventory chains"),
        "watch" => cmd_inventory_watch(args),
        "token-registry" => cmd_inventory_token_registry(args),
        "scan-evm" => scan_evm(args),
        "prune-addresses" => prune_addresses(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

/// Dispatch `sigillum api inventory prune-addresses` — delete scanned-address
/// records (and their holdings) from the wallet inventory. Selectors combine
/// with AND semantics; at least one is required.
fn prune_addresses(args: &[String]) {
    const PRUNE_USAGE: &str = "sigillum api inventory prune-addresses \
        [--address 0xADDR] [--wallet-family eth-seed|eth-xpub|eth-watch] \
        [--wallet-profile <NAME>] [--provider-profile <NAME>] [--chain-id <N>] \
        [--account-index <N>]  (at least one selector required)";
    let request = WalletInventoryAddressPruneRequest {
        address: parse_flag(args, "--address"),
        wallet_family: parse_flag(args, "--wallet-family"),
        wallet_profile: parse_flag(args, "--wallet-profile"),
        provider_profile: parse_flag(args, "--provider-profile"),
        chain_id: parse_u64_flag(args, "--chain-id"),
        account_index: parse_u32_flag(args, "--account-index"),
    };
    if request.address.is_none()
        && request.wallet_family.is_none()
        && request.wallet_profile.is_none()
        && request.provider_profile.is_none()
        && request.chain_id.is_none()
        && request.account_index.is_none()
    {
        eprintln!("Usage: {PRUNE_USAGE}");
        process::exit(1);
    }
    run_api_command(args, true, move |client| async move {
        client.prune_wallet_inventory_addresses(request).await
    });
}

fn list_inventory_with_chain_labels(args: &[String]) {
    let client = build_client(args, true);
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });
    let value = match runtime.block_on(async move {
        let inventory = client.list_wallet_inventory().await?;
        let chains = client.list_chain_profiles().await?;
        Ok::<serde_json::Value, sigillum_client::ClientError>(annotated_inventory_json(
            inventory, chains,
        ))
    }) {
        Ok(value) => value,
        Err(error) => report_client_error(error),
    };
    print_json(&value);
}

fn annotated_inventory_json(
    inventory: sigillum_api::WalletInventoryListResponse,
    chains: sigillum_api::ChainProfileListResponse,
) -> serde_json::Value {
    let labels: BTreeMap<u64, String> = chains
        .profiles
        .iter()
        .filter(|profile| profile.enabled)
        .filter_map(|profile| {
            profile
                .chain_id
                .map(|chain_id| (chain_id, profile.name.clone()))
        })
        .collect();
    let mut value = serde_json::to_value(inventory).unwrap_or_else(|error| {
        eprintln!("Failed to encode inventory response: {error}");
        process::exit(1);
    });
    if let Some(addresses) = value
        .get_mut("addresses")
        .and_then(|value| value.as_array_mut())
    {
        for address in addresses {
            annotate_chain_object(address, &labels);
        }
    }
    if let Some(holdings) = value
        .get_mut("holdings")
        .and_then(|value| value.as_array_mut())
    {
        for holding in holdings {
            annotate_chain_object(holding, &labels);
        }
    }
    if let Some(jobs) = value.get_mut("jobs").and_then(|value| value.as_array_mut()) {
        for job in jobs {
            annotate_job_chains(job, &labels);
            annotate_job_block_cursors(job, &labels);
        }
    }
    value["chain_profiles"] = serde_json::to_value(chains.profiles).unwrap_or_else(|error| {
        eprintln!("Failed to encode chain profiles: {error}");
        process::exit(1);
    });
    value
}

fn annotate_chain_object(value: &mut serde_json::Value, labels: &BTreeMap<u64, String>) {
    let Some(chain_id) = value.get("chain_id").and_then(|value| value.as_u64()) else {
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(name) = labels.get(&chain_id) {
        object.insert("chain_name".into(), serde_json::Value::String(name.clone()));
        object.insert(
            "chain_label".into(),
            serde_json::Value::String(format!("{chain_id} ({name})")),
        );
    } else {
        object.insert(
            "chain_label".into(),
            serde_json::Value::String(chain_id.to_string()),
        );
    }
}

fn annotate_job_chains(value: &mut serde_json::Value, labels: &BTreeMap<u64, String>) {
    let Some(chain_ids) = value.get("chain_ids").and_then(|value| value.as_array()) else {
        return;
    };
    let chain_labels = chain_ids
        .iter()
        .filter_map(|chain_id| chain_id.as_u64())
        .map(|chain_id| {
            labels
                .get(&chain_id)
                .map(|name| format!("{chain_id} ({name})"))
                .unwrap_or_else(|| chain_id.to_string())
        })
        .collect::<Vec<_>>();
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("chain_labels".into(), serde_json::json!(chain_labels));
}

fn annotate_job_block_cursors(value: &mut serde_json::Value, labels: &BTreeMap<u64, String>) {
    let Some(cursors) = value
        .get("block_cursors")
        .and_then(|value| value.as_array())
    else {
        return;
    };
    let cursor_labels = cursors
        .iter()
        .filter_map(|cursor| {
            let chain_id = cursor.get("chain_id").and_then(|value| value.as_u64())?;
            let topic_family = cursor
                .get("topic_family")
                .and_then(|value| value.as_str())?;
            let block = cursor
                .get("last_scanned_block")
                .and_then(|value| value.as_u64())?;
            let chain_label = labels
                .get(&chain_id)
                .map(|name| format!("{chain_id} ({name})"))
                .unwrap_or_else(|| chain_id.to_string());
            Some(format!("{chain_label} {topic_family} to {block}"))
        })
        .collect::<Vec<_>>();
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "block_cursor_labels".into(),
        serde_json::json!(cursor_labels),
    );
}

/// Dispatch `sigillum api chains <list|upsert|delete>`.
pub(super) fn cmd_api_chains(args: &[String]) {
    cmd_chains(args, 1, "sigillum api chains");
}

fn cmd_chains(args: &[String], command_index: usize, command: &str) {
    if args.len() <= command_index {
        eprintln!("Usage: {command} <list|upsert|delete> [...]");
        process::exit(1);
    }
    match args[command_index].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_chain_profiles().await
        }),
        "upsert" => {
            let usage = format!("{command} upsert --name <NAME> --family <FAMILY>");
            let request = ChainProfileUpsertRequest {
                name: require_flag(args, "--name", &usage),
                chain_family: require_flag(args, "--family", &usage),
                chain_id: parse_u64_flag(args, "--chain-id"),
                provider_profile: parse_flag(args, "--provider-profile"),
                native_symbol: parse_flag(args, "--native-symbol"),
                native_decimals: parse_u8_flag(args, "--native-decimals"),
                finality_blocks: parse_u64_flag(args, "--finality-blocks"),
                dormancy_block_window: parse_u64_flag(args, "--dormancy-block-window"),
                permit2_address: parse_flag(args, "--permit2-address"),
                uniswap_v2_router_address: parse_flag(args, "--uniswap-v2-router-address"),
                explorer_url: parse_flag(args, "--explorer-url"),
                capabilities: parse_multi_flag(args, "--capability"),
                enabled: bool_switch(args, "--enabled", "--disabled"),
                builtin: None,
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_chain_profile(request).await
            });
        }
        "delete" => {
            let usage = format!("{command} delete --name <NAME>");
            let name = require_flag(args, "--name", &usage);
            run_api_command(args, true, move |client| async move {
                client.delete_chain_profile(&name).await
            });
        }
        _ => {
            eprintln!("Usage: {command} <list|upsert|delete> [...]");
            process::exit(1);
        }
    }
}

fn parse_u8_flag(args: &[String], flag: &str) -> Option<u8> {
    parse_u32_flag(args, flag).map(|value| {
        u8::try_from(value).unwrap_or_else(|_| {
            eprintln!("{flag} must be between 0 and 255");
            process::exit(1);
        })
    })
}

fn cmd_inventory_watch(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: sigillum api inventory watch <list|upsert|delete> [...]");
        process::exit(1);
    }
    match args[2].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_watch_address_book().await
        }),
        "upsert" => {
            let request = WatchAddressBookUpsertRequest {
                address: require_flag(
                    args,
                    "--address",
                    "sigillum api inventory watch upsert --address <ADDR>",
                ),
                label: parse_flag(args, "--label"),
                tags: parse_multi_flag(args, "--tag"),
                enabled: bool_switch(args, "--enabled", "--disabled"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_watch_address_book_entry(request).await
            });
        }
        "delete" => {
            let address = require_flag(
                args,
                "--address",
                "sigillum api inventory watch delete --address <ADDR>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_watch_address_book_entry(&address).await
            });
        }
        _ => {
            eprintln!("Usage: sigillum api inventory watch <list|upsert|delete> [...]");
            process::exit(1);
        }
    }
}

fn cmd_inventory_token_registry(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: sigillum api inventory token-registry <list|import|delete> [...]");
        process::exit(1);
    }
    match args[2].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_token_registry().await
        }),
        "import" => {
            let usage = "sigillum api inventory token-registry import --name <NAME> (--entries-json <JSON> | --file <PATH>)";
            let name = require_flag(args, "--name", usage);
            let entries_json = parse_flag(args, "--entries-json");
            let file_path = parse_flag(args, "--file");
            if entries_json.is_some() == file_path.is_some() {
                eprintln!("Usage: {usage}");
                process::exit(1);
            }
            let request = TokenRegistryImportRequest {
                name,
                entries_json,
                file_path,
            };
            run_api_command(args, true, move |client| async move {
                client.import_token_registry(request).await
            });
        }
        "delete" => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api inventory token-registry delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_token_registry_list(&name).await
            });
        }
        _ => {
            eprintln!("Usage: sigillum api inventory token-registry <list|import|delete> [...]");
            process::exit(1);
        }
    }
}

fn scan_evm(args: &[String]) {
    let request = WalletInventoryScanRequest {
        wallet_family: parse_flag(args, "--wallet-family"),
        wallet_profile: parse_flag(args, "--wallet-profile"),
        provider_profile: parse_flag(args, "--provider-profile"),
        all_configured_chains: flag_option(args, "--all-configured-chains"),
        derivation_pattern: parse_flag(args, "--derivation-pattern"),
        account_limit: parse_u32_flag(args, "--account-limit"),
        watch_addresses: parse_watch_address_probes(args),
        include_watch_book: flag_option(args, "--include-watch-book"),
        gap_limit: parse_u32_flag(args, "--gap-limit"),
        max_index: parse_u32_flag(args, "--max-index"),
        resume_from_latest_checkpoint: flag_option(args, "--resume-from-latest-checkpoint"),
        run_async: flag_option(args, "--run-async"),
        partition_providers: flag_option(args, "--partition-providers"),
        token_addresses: parse_multi_flag(args, "--token-address"),
        block_tag: parse_flag(args, "--block-tag"),
        probe_token_registry: flag_option(args, "--probe-token-registry"),
        discover_erc20_transfers: flag_option(args, "--discover-erc20-transfers"),
        token_discovery_from_block: parse_flag(args, "--token-discovery-from-block"),
        token_discovery_to_block: parse_flag(args, "--token-discovery-to-block"),
        token_discovery_limit: parse_usize_flag(args, "--token-discovery-limit"),
        discover_erc20_allowances: flag_option(args, "--discover-erc20-allowances"),
        allowance_spender_addresses: parse_multi_flag(args, "--allowance-spender"),
        allowance_discovery_limit: parse_usize_flag(args, "--allowance-discovery-limit"),
        discover_permit2_allowances: flag_option(args, "--discover-permit2-allowances"),
        permit2_contract_addresses: parse_multi_flag(args, "--permit2-contract"),
        permit2_spender_addresses: parse_multi_flag(args, "--permit2-spender"),
        permit2_allowance_limit: parse_usize_flag(args, "--permit2-allowance-limit"),
        discover_erc721_transfers: flag_option(args, "--discover-erc721-transfers"),
        discover_erc1155_transfers: flag_option(args, "--discover-erc1155-transfers"),
        discover_nft_operator_approvals: flag_option(args, "--discover-nft-operator-approvals"),
        nft_operator_addresses: parse_multi_flag(args, "--nft-operator"),
        nft_operator_approval_limit: parse_usize_flag(args, "--nft-operator-approval-limit"),
        discover_defi_token_positions: flag_option(args, "--discover-defi-token-positions"),
        defi_token_probes: parse_defi_token_probes(args),
        defi_position_limit: parse_usize_flag(args, "--defi-position-limit"),
        discover_claim_candidates: flag_option(args, "--discover-claim-candidates"),
        claim_candidate_probes: parse_claim_candidate_probes(args),
        claim_candidate_limit: parse_usize_flag(args, "--claim-candidate-limit"),
        nft_discovery_from_block: parse_flag(args, "--nft-discovery-from-block"),
        nft_discovery_to_block: parse_flag(args, "--nft-discovery-to-block"),
        nft_discovery_limit: parse_usize_flag(args, "--nft-discovery-limit"),
    };
    run_api_command(args, true, move |client| async move {
        client.scan_evm_wallet_inventory(request).await
    });
}

#[cfg(test)]
mod tests {
    use sigillum_api::{
        ChainProfile, ChainProfileListResponse, WalletAddressActivityState, WalletAssetHolding,
        WalletAssetKind, WalletInventoryAddress, WalletInventoryListResponse,
    };

    use super::*;

    #[test]
    fn annotated_inventory_json_surfaces_registry_chain_names() {
        let inventory = WalletInventoryListResponse {
            jobs: Vec::new(),
            addresses: vec![WalletInventoryAddress {
                id: "addr_1".into(),
                wallet_family: "eth-seed".into(),
                wallet_profile: "seed-main".into(),
                provider_profile: "mainnet".into(),
                chain_id: 1,
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
                derivation_pattern: None,
                account_index: None,
                address_index: 0,
                activity_state: WalletAddressActivityState::Funded,
                native_balance_wei_hex: "0x1".into(),
                transaction_count: 1,
                last_activity_block: Some(18),
                classifications: Vec::new(),
                source: "local-rpc".into(),
                first_seen_at_unix: 1,
                last_checked_at_unix: 2,
            }],
            holdings: vec![WalletAssetHolding {
                id: "holding_1".into(),
                wallet_family: "eth-seed".into(),
                wallet_profile: "seed-main".into(),
                provider_profile: "unknown".into(),
                chain_id: 999,
                address: "0x1111111111111111111111111111111111111111".into(),
                derivation_path: "m/44'/60'/0'/0/0".into(),
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
                amount_hex: "0x1".into(),
                source: "local-rpc".into(),
                status: "detected".into(),
                first_seen_at_unix: 1,
                last_checked_at_unix: 2,
            }],
            nft_metadata_cache: Vec::new(),
            pagination: None,
        };
        let chains = ChainProfileListResponse {
            profiles: vec![ChainProfile {
                name: "ethereum".into(),
                chain_family: "evm".into(),
                chain_id: Some(1),
                provider_profile: None,
                native_symbol: "ETH".into(),
                native_decimals: 18,
                finality_blocks: 0,
                dormancy_block_window: sigillum_api::DEFAULT_DORMANCY_BLOCK_WINDOW,
                permit2_address: None,
                uniswap_v2_router_address: None,
                explorer_url: None,
                capabilities: Vec::new(),
                enabled: true,
                source: "builtin".into(),
                builtin: true,
                created_at_unix: 0,
                updated_at_unix: 0,
            }],
        };

        let value = annotated_inventory_json(inventory, chains);

        assert_eq!(value["addresses"][0]["chain_name"], "ethereum");
        assert_eq!(value["addresses"][0]["chain_label"], "1 (ethereum)");
        assert_eq!(value["addresses"][0]["last_activity_block"], 18);
        assert_eq!(value["holdings"][0]["chain_label"], "999");
        assert!(value["holdings"][0].get("chain_name").is_none());
        assert_eq!(value["chain_profiles"][0]["builtin"], true);
    }
}
