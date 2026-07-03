//! Read-only EVM RPC daemon API commands.

use std::process;

use sigillum_api::request::{
    EvmFeeEstimateRequest, EvmProviderRef, EvmRpcBalanceRequest, EvmRpcErc20BalanceRequest,
    EvmRpcNonceRequest,
};

use super::{
    parse_flag, parse_u64_flag, parse_usize_flag, require_flag, require_u64_flag, run_api_command,
};

const USAGE: &str = "Usage: sigillum api evm <nonce|balance|erc20-balance|fees> [...]";
const NONCE_USAGE: &str =
    "sigillum api evm nonce --rpc-url <URL> --address <ADDR> [--block-tag <TAG>]";
const BALANCE_USAGE: &str =
    "sigillum api evm balance --rpc-url <URL> --address <ADDR> [--block-tag <TAG>]";
const ERC20_BALANCE_USAGE: &str = "sigillum api evm erc20-balance --rpc-url <URL> --token-address <ADDR> --owner-address <ADDR> [--block-tag <TAG>]";
const FEES_USAGE: &str = "sigillum api evm fees --rpc-url <URL> --chain-id <N> [--gas-limit <N>]";

/// Dispatch `sigillum api evm <nonce|balance|erc20-balance|fees>`.
pub(super) fn cmd_api_evm(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "nonce" => nonce(args),
        "balance" => balance(args),
        "erc20-balance" => erc20_balance(args),
        "fees" | "estimate" => estimate_fees(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

fn nonce(args: &[String]) {
    let request = EvmRpcNonceRequest {
        provider: provider_ref(args, NONCE_USAGE),
        address: require_flag(args, "--address", NONCE_USAGE),
        block_tag: parse_flag(args, "--block-tag"),
    };
    run_api_command(args, true, move |client| async move {
        client.evm_nonce(request).await
    });
}

fn balance(args: &[String]) {
    let request = EvmRpcBalanceRequest {
        provider: provider_ref(args, BALANCE_USAGE),
        address: require_flag(args, "--address", BALANCE_USAGE),
        block_tag: parse_flag(args, "--block-tag"),
    };
    run_api_command(args, true, move |client| async move {
        client.evm_balance(request).await
    });
}

fn erc20_balance(args: &[String]) {
    let request = EvmRpcErc20BalanceRequest {
        provider: provider_ref(args, ERC20_BALANCE_USAGE),
        token_address: require_flag(args, "--token-address", ERC20_BALANCE_USAGE),
        owner_address: require_flag(args, "--owner-address", ERC20_BALANCE_USAGE),
        block_tag: parse_flag(args, "--block-tag"),
    };
    run_api_command(args, true, move |client| async move {
        client.evm_erc20_balance(request).await
    });
}

fn estimate_fees(args: &[String]) {
    let request = EvmFeeEstimateRequest {
        provider: provider_ref(args, FEES_USAGE),
        chain_id: require_u64_flag(args, "--chain-id", FEES_USAGE),
        gas_limit: parse_u64_flag(args, "--gas-limit"),
    };
    run_api_command(args, true, move |client| async move {
        client.evm_estimate_fees(request).await
    });
}

fn provider_ref(args: &[String], usage: &str) -> EvmProviderRef {
    EvmProviderRef {
        rpc_url: require_flag(args, "--rpc-url", usage),
        auth_token_key: parse_flag(args, "--auth-token-key"),
        compartment_id: parse_usize_flag(args, "--compartment-id"),
    }
}
