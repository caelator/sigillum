//! Wallet export/derive daemon API commands.

use std::process;

use super::{
    parse_flag, read_optional_sensitive_input, require_flag, require_u32_flag, run_api_command,
};

const USAGE: &str = "Usage: sigillum api wallets <xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check> [...]";
const XPUB_EXPORT_USAGE: &str = "sigillum api wallets xpub-export --wallet-profile <NAME>";
const XPUB_DERIVE_USAGE: &str = "sigillum api wallets xpub-derive --xpub <XPUB> --index <N>";
const STEALTH_EXPORT_USAGE: &str =
    "sigillum api wallets stealth-export --wallet <WALLET> [--short-name <NAME>]";
const STEALTH_GENERATE_USAGE: &str = "sigillum api wallets stealth-generate --meta-address <SMA> [--ephemeral-key-env VAR|--ephemeral-key-stdin]";
const STEALTH_CHECK_USAGE: &str = "sigillum api wallets stealth-check --wallet <WALLET> --stealth-address <ADDR> --ephemeral-public-key-hex <HEX> [--view-tag-hex <HH>]";

/// Dispatch `sigillum api wallets <xpub-export|xpub-derive|stealth-export|stealth-generate|stealth-check>`.
pub(super) fn cmd_api_wallets(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "xpub-export" => xpub_export(args),
        "xpub-derive" => xpub_derive(args),
        "stealth-export" => stealth_export(args),
        "stealth-generate" => stealth_generate(args),
        "stealth-check" => stealth_check(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

fn xpub_export(args: &[String]) {
    let wallet_profile = require_flag(args, "--wallet-profile", XPUB_EXPORT_USAGE);
    run_api_command(args, true, move |client| async move {
        client.export_eth_xpub_receive_branch(&wallet_profile).await
    });
}

fn xpub_derive(args: &[String]) {
    let xpub = require_flag(args, "--xpub", XPUB_DERIVE_USAGE);
    let index = require_u32_flag(args, "--index", XPUB_DERIVE_USAGE);
    run_api_command(args, true, move |client| async move {
        client.derive_eth_xpub_receive_address(&xpub, index).await
    });
}

fn stealth_export(args: &[String]) {
    let wallet = require_flag(args, "--wallet", STEALTH_EXPORT_USAGE);
    let short_name = parse_flag(args, "--short-name");
    run_api_command(args, true, move |client| async move {
        client
            .export_eth_stealth_meta_address(&wallet, short_name.as_deref())
            .await
    });
}

fn stealth_generate(args: &[String]) {
    reject_raw_ephemeral_key_flags(args);
    let meta_address = require_flag(args, "--meta-address", STEALTH_GENERATE_USAGE);
    let ephemeral = read_optional_ephemeral_private_key(args);
    run_api_command(args, true, move |client| async move {
        client
            .generate_eth_stealth_address(&meta_address, ephemeral.as_ref())
            .await
    });
}

fn stealth_check(args: &[String]) {
    let wallet = require_flag(args, "--wallet", STEALTH_CHECK_USAGE);
    let stealth_address = require_flag(args, "--stealth-address", STEALTH_CHECK_USAGE);
    let ephemeral_public_key =
        require_hex_flag(args, "--ephemeral-public-key-hex", STEALTH_CHECK_USAGE);
    let view_tag = parse_view_tag(args);
    run_api_command(args, true, move |client| async move {
        client
            .check_eth_stealth_address(&wallet, &stealth_address, &ephemeral_public_key, view_tag)
            .await
    });
}

fn read_optional_ephemeral_private_key(args: &[String]) -> Option<[u8; 32]> {
    read_optional_sensitive_input(args, "--ephemeral-key-env", "--ephemeral-key-stdin")
        .map(|value| decode_32_byte_hex("ephemeral private key", &value))
}

fn reject_raw_ephemeral_key_flags(args: &[String]) {
    for flag in [
        "--ephemeral-key",
        "--ephemeral-private-key",
        "--ephemeral-private-key-hex",
    ] {
        if args.iter().any(|arg| arg == flag) {
            eprintln!(
                "Do not pass ephemeral private keys as CLI arguments; use --ephemeral-key-env VAR or --ephemeral-key-stdin."
            );
            process::exit(1);
        }
    }
}

fn parse_view_tag(args: &[String]) -> Option<u8> {
    parse_flag(args, "--view-tag-hex").map(|value| {
        let bytes = decode_hex_flag("--view-tag-hex", &value);
        if bytes.len() != 1 {
            eprintln!(
                "Invalid value for --view-tag-hex: expected exactly 1 byte, got {}.",
                bytes.len()
            );
            process::exit(1);
        }
        bytes[0]
    })
}

fn require_hex_flag(args: &[String], flag: &str, usage: &str) -> Vec<u8> {
    let value = require_flag(args, flag, usage);
    decode_hex_flag(flag, &value)
}

fn decode_32_byte_hex(name: &str, value: &str) -> [u8; 32] {
    let bytes = hex::decode(value).unwrap_or_else(|error| {
        eprintln!("Invalid hex for {name}: {error}");
        process::exit(1);
    });
    if bytes.len() != 32 {
        eprintln!(
            "Invalid value for {name}: expected exactly 32 bytes, got {}.",
            bytes.len()
        );
        process::exit(1);
    }
    bytes.try_into().expect("length checked")
}

fn decode_hex_flag(flag: &str, value: &str) -> Vec<u8> {
    hex::decode(value).unwrap_or_else(|error| {
        eprintln!("Invalid hex for {flag}: {error}");
        process::exit(1);
    })
}
