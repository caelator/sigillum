//! sigillum generate — password, passphrase, and TOTP generation.

use std::process;

use sigillum_api::request::{GenerateStoreKind, GenerateStoreRequest, PasswordCharset};
use sigillum_client::SigillumClient;
use sigillum_generator::{
    DEFAULT_PASSPHRASE_WORDS, generate_passphrase, generate_password, generate_totp,
};

use crate::daemon_api::{daemon_base_url, ensure_daemon_ready, require_session_token};

pub fn cmd_generate(args: &[String]) {
    if args.is_empty() {
        print_usage_and_exit();
    }

    match args[0].as_str() {
        "password" => cmd_generate_password(&args[1..]),
        "passphrase" => cmd_generate_passphrase(&args[1..]),
        "totp" => cmd_generate_totp(&args[1..]),
        _ => print_usage_and_exit(),
    }
}

fn cmd_generate_password(args: &[String]) {
    let length = parse_usize_flag(args, "--length").unwrap_or(32);
    let charset = parse_flag(args, "--charset").unwrap_or_else(|| "mixalpha-numeric-symbol".into());
    let store_key = parse_flag(args, "--store");

    if let Some(key) = store_key {
        let response = generate_and_store(
            args,
            GenerateStoreRequest {
                key,
                kind: GenerateStoreKind::Password {
                    length,
                    charset: parse_charset(&charset),
                },
            },
        );
        println!("{}", response.value);
        return;
    }

    match generate_password(&charset, length) {
        Ok(value) => println!("{value}"),
        Err(error) => exit_with_error(&error.to_string()),
    }
}

fn cmd_generate_passphrase(args: &[String]) {
    let word_count = parse_usize_flag(args, "--words").unwrap_or(DEFAULT_PASSPHRASE_WORDS);
    let separator = parse_flag(args, "--separator").unwrap_or_else(|| "-".into());
    let store_key = parse_flag(args, "--store");

    if let Some(key) = store_key {
        let response = generate_and_store(
            args,
            GenerateStoreRequest {
                key,
                kind: GenerateStoreKind::Passphrase {
                    word_count,
                    separator,
                },
            },
        );
        println!("{}", response.value);
        return;
    }

    match generate_passphrase(word_count, &separator) {
        Ok(value) => println!("{value}"),
        Err(error) => exit_with_error(&error.to_string()),
    }
}

fn cmd_generate_totp(args: &[String]) {
    if parse_flag(args, "--store").is_some() {
        exit_with_error("--store is not supported for totp generation");
    }

    let secret =
        parse_flag(args, "--secret").unwrap_or_else(|| exit_with_error("--secret is required"));
    let period = parse_u64_flag(args, "--period").unwrap_or(30);
    let digits = parse_u32_flag(args, "--digits").unwrap_or(6);

    match generate_totp(&secret, period, digits) {
        Ok(value) => println!("{value}"),
        Err(error) => exit_with_error(&error.to_string()),
    }
}

fn generate_and_store(
    args: &[String],
    request: GenerateStoreRequest,
) -> sigillum_api::GenerateStoreResponse {
    let base_url = daemon_base_url(args);
    let session_token = require_session_token(args);
    if let Err(error) = ensure_daemon_ready(&base_url) {
        exit_with_error(&error);
    }

    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        exit_with_error(&format!("failed to start async runtime: {error}"))
    });
    let client = SigillumClient::new(&base_url);
    client.set_session_token(&session_token);
    runtime
        .block_on(client.generate_and_store(request))
        .unwrap_or_else(|error| exit_with_error(&format!("generation failed: {error}")))
}

fn parse_charset(value: &str) -> PasswordCharset {
    match value {
        "loweralpha" => PasswordCharset::Loweralpha,
        "mixalpha" => PasswordCharset::Mixalpha,
        "numeric" => PasswordCharset::Numeric,
        "alpha-numeric" => PasswordCharset::AlphaNumeric,
        "mixalpha-numeric" => PasswordCharset::MixalphaNumeric,
        "mixalpha-numeric-symbol" => PasswordCharset::MixalphaNumericSymbol,
        other => exit_with_error(&format!("unsupported charset '{other}'")),
    }
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
        .or_else(|| {
            args.iter()
                .find_map(|arg| arg.strip_prefix(&format!("{flag}=")).map(str::to_string))
        })
}

fn parse_usize_flag(args: &[String], flag: &str) -> Option<usize> {
    parse_flag(args, flag).map(|value| {
        value
            .parse::<usize>()
            .unwrap_or_else(|_| exit_with_error(&format!("{flag} must be a positive integer")))
    })
}

fn parse_u64_flag(args: &[String], flag: &str) -> Option<u64> {
    parse_flag(args, flag).map(|value| {
        value
            .parse::<u64>()
            .unwrap_or_else(|_| exit_with_error(&format!("{flag} must be an integer")))
    })
}

fn parse_u32_flag(args: &[String], flag: &str) -> Option<u32> {
    parse_flag(args, flag).map(|value| {
        value
            .parse::<u32>()
            .unwrap_or_else(|_| exit_with_error(&format!("{flag} must be an integer")))
    })
}

fn print_usage_and_exit() -> ! {
    eprintln!(
        "Usage:\n  sigillum generate password [--length N] [--charset NAME] [--store KEY]\n  sigillum generate passphrase [--words N] [--separator STR] [--store KEY]\n  sigillum generate totp --secret BASE32 [--period SECONDS] [--digits N]"
    );
    process::exit(1);
}

fn exit_with_error(message: &str) -> ! {
    eprintln!("error: {message}");
    process::exit(1);
}
