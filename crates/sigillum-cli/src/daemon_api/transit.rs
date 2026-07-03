//! Transit encryption daemon API commands.

use std::process;

use serde_json::json;

use super::{parse_flag, require_flag, run_api_command};

const USAGE: &str = "Usage: sigillum api transit <encrypt|decrypt|hmac> [...]";
const ENCRYPT_USAGE: &str =
    "sigillum api transit encrypt --key <NAME> --plaintext-hex <HEX> [--aad-hex <HEX>]";
const DECRYPT_USAGE: &str = "sigillum api transit decrypt --key <NAME> --nonce-hex <HEX> --ciphertext-hex <HEX> [--aad-hex <HEX>]";
const HMAC_USAGE: &str = "sigillum api transit hmac --key <NAME> --input-hex <HEX>";

/// Dispatch `sigillum api transit <encrypt|decrypt|hmac>`.
pub(super) fn cmd_api_transit(args: &[String]) {
    if args.len() < 2 {
        eprintln!("{USAGE}");
        process::exit(1);
    }

    match args[1].as_str() {
        "encrypt" => encrypt(args),
        "decrypt" => decrypt(args),
        "hmac" => hmac(args),
        _ => {
            eprintln!("{USAGE}");
            process::exit(1);
        }
    }
}

fn encrypt(args: &[String]) {
    let key = require_flag(args, "--key", ENCRYPT_USAGE);
    let plaintext = require_hex_flag(args, "--plaintext-hex", ENCRYPT_USAGE);
    let aad = parse_hex_flag(args, "--aad-hex");
    run_api_command(args, true, move |client| async move {
        client
            .transit_encrypt(&key, &plaintext, aad.as_deref())
            .await
            .map(|(nonce, ciphertext)| {
                json!({
                    "nonce_hex": hex::encode(nonce),
                    "ciphertext_hex": hex::encode(ciphertext),
                })
            })
    });
}

fn decrypt(args: &[String]) {
    let key = require_flag(args, "--key", DECRYPT_USAGE);
    let nonce = require_hex_flag(args, "--nonce-hex", DECRYPT_USAGE);
    let ciphertext = require_hex_flag(args, "--ciphertext-hex", DECRYPT_USAGE);
    let aad = parse_hex_flag(args, "--aad-hex");
    run_api_command(args, true, move |client| async move {
        client
            .transit_decrypt(&key, &nonce, &ciphertext, aad.as_deref())
            .await
            .map(|plaintext| {
                json!({
                    "plaintext_hex": hex::encode(plaintext),
                })
            })
    });
}

fn hmac(args: &[String]) {
    let key = require_flag(args, "--key", HMAC_USAGE);
    let input = require_hex_flag(args, "--input-hex", HMAC_USAGE);
    run_api_command(args, true, move |client| async move {
        client.transit_hmac(&key, &input).await.map(|digest| {
            json!({
                "digest_hex": hex::encode(digest),
            })
        })
    });
}

fn parse_hex_flag(args: &[String], flag: &str) -> Option<Vec<u8>> {
    parse_flag(args, flag).map(|value| decode_hex_flag(flag, &value))
}

fn require_hex_flag(args: &[String], flag: &str, usage: &str) -> Vec<u8> {
    let value = require_flag(args, flag, usage);
    decode_hex_flag(flag, &value)
}

fn decode_hex_flag(flag: &str, value: &str) -> Vec<u8> {
    hex::decode(value).unwrap_or_else(|error| {
        eprintln!("Invalid hex for {flag}: {error}");
        process::exit(1);
    })
}
