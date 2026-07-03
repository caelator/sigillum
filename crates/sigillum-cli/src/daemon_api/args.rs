use std::io::{self, Read};
use std::process;

// ── Sensitive input handling ─────────────────────────────────────

pub(crate) fn read_passphrase(args: &[String]) -> String {
    read_sensitive_input(
        args,
        "--passphrase-env",
        "--passphrase-stdin",
        "Daemon passphrase: ",
    )
}

pub(crate) fn read_optional_pin(args: &[String]) -> Option<String> {
    read_optional_sensitive_input(args, "--pin-env", "--pin-stdin")
}

/// Read an optional BIP-39 mnemonic passphrase without accepting it as a raw
/// CLI argument (so it never lands in shell history or the process list).
pub(crate) fn read_optional_mnemonic_passphrase(args: &[String]) -> Option<String> {
    if let Some(env_key) = parse_flag(args, "--mnemonic-passphrase-env") {
        return Some(std::env::var(&env_key).unwrap_or_else(|_| {
            eprintln!("Environment variable {env_key} is not set.");
            process::exit(1);
        }));
    }
    if has_flag(args, "--mnemonic-passphrase-stdin") {
        return Some(read_stdin_secret("mnemonic passphrase"));
    }
    None
}

/// Read a sensitive value using one of three delivery modes (in priority order):
/// 1. Environment variable (`env_flag`): `--*-env VAR`
/// 2. Standard input (`stdin_flag`): `--*-stdin`
/// 3. Interactive terminal prompt (`prompt_label`)
pub(crate) fn read_sensitive_input(
    args: &[String],
    env_flag: &str,
    stdin_flag: &str,
    prompt_label: &str,
) -> String {
    if let Some(env_key) = parse_flag(args, env_flag) {
        return std::env::var(&env_key).unwrap_or_else(|_| {
            eprintln!("Environment variable {env_key} is not set.");
            process::exit(1);
        });
    }
    if has_flag(args, stdin_flag) {
        let name = prompt_label.trim_end_matches([':', ' ']);
        return read_stdin_secret(name);
    }
    rpassword::prompt_password(prompt_label).unwrap_or_else(|error| {
        eprintln!("Failed to read {prompt_label} {error}");
        process::exit(1);
    })
}

pub(crate) fn read_optional_sensitive_input(
    args: &[String],
    env_flag: &str,
    stdin_flag: &str,
) -> Option<String> {
    if let Some(env_key) = parse_flag(args, env_flag) {
        return Some(std::env::var(&env_key).unwrap_or_else(|_| {
            eprintln!("Environment variable {env_key} is not set.");
            process::exit(1);
        }));
    }
    if has_flag(args, stdin_flag) {
        return Some(read_stdin_secret("FIDO2 PIN"));
    }
    None
}

pub(crate) fn read_stdin_secret(name: &str) -> String {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .unwrap_or_else(|error| {
            eprintln!("Failed to read {name} from stdin: {error}");
            process::exit(1);
        });
    let value = buf.trim().to_string();
    if value.is_empty() {
        eprintln!("Expected non-empty {name} on stdin.");
        process::exit(1);
    }
    value
}

// ── Flag parsing ────────────────────────────────────────────────

/// Find a `--flag <value>` pair in `args`, returning the value if present.
pub(crate) fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

pub(crate) fn parse_multi_flag(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            values.push(args[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    values
}

pub(crate) fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

pub(crate) fn flag_option(args: &[String], flag: &str) -> Option<bool> {
    if has_flag(args, flag) {
        Some(true)
    } else {
        None
    }
}

pub(crate) fn bool_switch(args: &[String], positive: &str, negative: &str) -> Option<bool> {
    if has_flag(args, positive) {
        Some(true)
    } else if has_flag(args, negative) {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn require_flag(args: &[String], flag: &str, usage: &str) -> String {
    parse_flag(args, flag).unwrap_or_else(|| {
        eprintln!("Usage: {usage}");
        process::exit(1);
    })
}

pub(crate) fn parse_usize_flag(args: &[String], flag: &str) -> Option<usize> {
    parse_flag(args, flag).map(|value| parse_usize_value(&value, flag))
}

pub(crate) fn require_usize_flag(args: &[String], flag: &str, usage: &str) -> usize {
    parse_flag(args, flag)
        .map(|value| parse_usize_value(&value, flag))
        .unwrap_or_else(|| {
            eprintln!("Usage: {usage}");
            process::exit(1);
        })
}

pub(crate) fn parse_u64_flag(args: &[String], flag: &str) -> Option<u64> {
    parse_flag(args, flag).map(|value| parse_u64_value(&value, flag))
}

pub(crate) fn parse_u32_flag(args: &[String], flag: &str) -> Option<u32> {
    parse_flag(args, flag).map(|value| parse_u32_value(&value, flag))
}

pub(crate) fn require_u64_flag(args: &[String], flag: &str, usage: &str) -> u64 {
    parse_flag(args, flag)
        .map(|value| parse_u64_value(&value, flag))
        .unwrap_or_else(|| {
            eprintln!("Usage: {usage}");
            process::exit(1);
        })
}

pub(crate) fn require_u32_flag(args: &[String], flag: &str, usage: &str) -> u32 {
    parse_flag(args, flag)
        .map(|value| parse_u32_value(&value, flag))
        .unwrap_or_else(|| {
            eprintln!("Usage: {usage}");
            process::exit(1);
        })
}

pub(crate) fn parse_usize_value(value: &str, flag: &str) -> usize {
    value.parse::<usize>().unwrap_or_else(|_| {
        eprintln!("Invalid value for {flag}: {value}");
        process::exit(1);
    })
}

pub(crate) fn parse_u64_value(value: &str, flag: &str) -> u64 {
    value.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Invalid value for {flag}: {value}");
        process::exit(1);
    })
}

pub(crate) fn parse_u32_value(value: &str, flag: &str) -> u32 {
    value.parse::<u32>().unwrap_or_else(|_| {
        eprintln!("Invalid value for {flag}: {value}");
        process::exit(1);
    })
}
