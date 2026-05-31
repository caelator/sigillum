//! Deposit daemon API commands.

use std::process;

use sigillum_api::request::{
    EthStealthAnnouncementScanRequest, EthStealthDepositCreateErc20Request,
    EthStealthDepositCreateNativeRequest, EthStealthDepositDeleteRequest,
    EthStealthDepositEnqueueSweepRequest, EthStealthDepositRefreshRequest,
};

use super::{
    bool_switch, flag_option, parse_flag, parse_usize_flag, require_flag, run_api_command,
};

const USAGE: &str = "Usage: sigillum api deposits <list|create-native|create-erc20|scan-announcements|refresh|enqueue-sweep|delete> [...]";

/// Dispatch `sigillum api deposits <list|create-native|create-erc20|scan-announcements|refresh|enqueue-sweep|delete>`.
pub(super) fn cmd_api_deposits(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "list" => run_api_command(args, true, |client| async move {
            client.list_eth_stealth_deposits().await
        }),
        "create-native" => create_native(args),
        "create-erc20" => create_erc20(args),
        "scan-announcements" => scan_announcements(args),
        "refresh" => refresh(args),
        "enqueue-sweep" => enqueue_sweep(args),
        "delete" => delete(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

fn create_native(args: &[String]) {
    let request = EthStealthDepositCreateNativeRequest {
        wallet_profile: require_flag(
            args,
            "--wallet-profile",
            "sigillum api deposits create-native --wallet-profile <NAME>",
        ),
        expected_value_wei_hex: parse_flag(args, "--expected-value-wei-hex"),
        auto_queue_sweep: flag_option(args, "--auto-queue-sweep"),
        sweep_destination_address: parse_flag(args, "--sweep-destination-address"),
        min_sweep_value_wei_hex: parse_flag(args, "--min-sweep-value-wei-hex"),
        note: parse_flag(args, "--note"),
        ephemeral_private_key_hex: parse_flag(args, "--ephemeral-private-key-hex"),
    };
    run_api_command(args, true, move |client| async move {
        client.create_eth_stealth_native_deposit(request).await
    });
}

fn create_erc20(args: &[String]) {
    let usage = "sigillum api deposits create-erc20 --wallet-profile <NAME> --token-address <ADDR>";
    let request = EthStealthDepositCreateErc20Request {
        wallet_profile: require_flag(args, "--wallet-profile", usage),
        token_address: require_flag(args, "--token-address", usage),
        expected_amount_hex: parse_flag(args, "--expected-amount-hex"),
        auto_queue_sweep: flag_option(args, "--auto-queue-sweep"),
        sweep_destination_address: parse_flag(args, "--sweep-destination-address"),
        min_sweep_amount_hex: parse_flag(args, "--min-sweep-amount-hex"),
        note: parse_flag(args, "--note"),
        ephemeral_private_key_hex: parse_flag(args, "--ephemeral-private-key-hex"),
    };
    run_api_command(args, true, move |client| async move {
        client.create_eth_stealth_erc20_deposit(request).await
    });
}

fn scan_announcements(args: &[String]) {
    let usage =
        "sigillum api deposits scan-announcements --wallet-profile <NAME> --from-block <TAG|0xN>";
    let request = EthStealthAnnouncementScanRequest {
        wallet_profile: require_flag(args, "--wallet-profile", usage),
        from_block: require_flag(args, "--from-block", usage),
        to_block: parse_flag(args, "--to-block"),
        token_address: parse_flag(args, "--token-address"),
        limit: parse_usize_flag(args, "--limit"),
        auto_queue_sweep: flag_option(args, "--auto-queue-sweep"),
        sweep_destination_address: parse_flag(args, "--sweep-destination-address"),
        min_sweep_amount_hex: parse_flag(args, "--min-sweep-amount-hex"),
        note: parse_flag(args, "--note"),
    };
    run_api_command(args, true, move |client| async move {
        client.scan_eth_stealth_announcements(request).await
    });
}

fn refresh(args: &[String]) {
    let request = EthStealthDepositRefreshRequest {
        id: parse_flag(args, "--id"),
        limit: parse_usize_flag(args, "--limit"),
        auto_enqueue: bool_switch(args, "--auto-enqueue", "--no-auto-enqueue"),
    };
    run_api_command(args, true, move |client| async move {
        client.refresh_eth_stealth_deposits(request).await
    });
}

fn enqueue_sweep(args: &[String]) {
    let request = EthStealthDepositEnqueueSweepRequest {
        id: require_flag(
            args,
            "--id",
            "sigillum api deposits enqueue-sweep --id <ID>",
        ),
        force: flag_option(args, "--force"),
    };
    run_api_command(args, true, move |client| async move {
        client.enqueue_eth_stealth_deposit_sweep(request).await
    });
}

fn delete(args: &[String]) {
    let request = EthStealthDepositDeleteRequest {
        id: require_flag(args, "--id", "sigillum api deposits delete --id <ID>"),
    };
    run_api_command(args, true, move |client| async move {
        client.delete_eth_stealth_deposit(request).await
    });
}
