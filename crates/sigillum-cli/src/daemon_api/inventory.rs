//! Inventory daemon API commands.

use std::process;

use sigillum_api::request::{
    ChainProfileUpsertRequest, WalletInventoryScanRequest, WatchAddressBookUpsertRequest,
};

use super::inventory_args::{
    parse_claim_candidate_probes, parse_defi_token_probes, parse_watch_address_probes,
};
use super::{
    bool_switch, flag_option, parse_flag, parse_multi_flag, parse_u32_flag, parse_u64_flag,
    parse_usize_flag, require_flag, run_api_command,
};

const USAGE: &str = "Usage: sigillum api inventory <list|chains|watch|scan-evm> [...]";

/// Dispatch `sigillum api inventory <list|chains|watch|scan-evm>`.
pub(super) fn cmd_api_inventory(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_wallet_inventory().await
        }),
        "chains" => cmd_inventory_chains(args),
        "watch" => cmd_inventory_watch(args),
        "scan-evm" => scan_evm(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

fn cmd_inventory_chains(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: sigillum api inventory chains <list|upsert|delete> [...]");
        process::exit(1);
    }
    match args[2].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_chain_profiles().await
        }),
        "upsert" => {
            let request = ChainProfileUpsertRequest {
                name: require_flag(
                    args,
                    "--name",
                    "sigillum api inventory chains upsert --name <NAME> --family <FAMILY>",
                ),
                chain_family: require_flag(
                    args,
                    "--family",
                    "sigillum api inventory chains upsert --name <NAME> --family <FAMILY>",
                ),
                chain_id: parse_u64_flag(args, "--chain-id"),
                provider_profile: parse_flag(args, "--provider-profile"),
                native_symbol: parse_flag(args, "--native-symbol"),
                explorer_url: parse_flag(args, "--explorer-url"),
                capabilities: parse_multi_flag(args, "--capability"),
                enabled: bool_switch(args, "--enabled", "--disabled"),
            };
            run_api_command(args, true, move |client| async move {
                client.upsert_chain_profile(request).await
            });
        }
        "delete" => {
            let name = require_flag(
                args,
                "--name",
                "sigillum api inventory chains delete --name <NAME>",
            );
            run_api_command(args, true, move |client| async move {
                client.delete_chain_profile(&name).await
            });
        }
        _ => {
            eprintln!("Usage: sigillum api inventory chains <list|upsert|delete> [...]");
            process::exit(1);
        }
    }
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

fn scan_evm(args: &[String]) {
    let request = WalletInventoryScanRequest {
        wallet_family: parse_flag(args, "--wallet-family"),
        wallet_profile: parse_flag(args, "--wallet-profile"),
        provider_profile: parse_flag(args, "--provider-profile"),
        watch_addresses: parse_watch_address_probes(args),
        include_watch_book: flag_option(args, "--include-watch-book"),
        gap_limit: parse_u32_flag(args, "--gap-limit"),
        max_index: parse_u32_flag(args, "--max-index"),
        token_addresses: parse_multi_flag(args, "--token-address"),
        block_tag: parse_flag(args, "--block-tag"),
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
