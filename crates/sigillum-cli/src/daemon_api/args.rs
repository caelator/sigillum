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

/// Read a BIP-39 mnemonic without accepting it as a raw CLI argument (so it
/// never lands in shell history or the process list).
///
/// Delivery modes follow [`read_sensitive_input`]: `--mnemonic-env VAR`,
/// `--mnemonic-stdin`, or a hidden interactive terminal prompt. The phrase is
/// trimmed of surrounding whitespace and must be non-empty.
pub(crate) fn read_mnemonic(args: &[String]) -> String {
    let mnemonic = read_sensitive_input(
        args,
        "--mnemonic-env",
        "--mnemonic-stdin",
        "BIP-39 mnemonic: ",
    );
    let mnemonic = mnemonic.trim().to_string();
    if mnemonic.is_empty() {
        eprintln!("Expected non-empty mnemonic.");
        process::exit(1);
    }
    mnemonic
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

pub(crate) fn reject_raw_ephemeral_key_flags(args: &[String]) {
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

pub(crate) fn decode_32_byte_hex(name: &str, value: &str) -> [u8; 32] {
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

// ── Flag parsing ────────────────────────────────────────────────

/// Find a `--flag <value>` or `--flag=<value>` pair in `args`, returning the
/// value if present. Space-form values take precedence over equals-form values.
pub(crate) fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find_map(|window| (window[0] == flag).then(|| window[1].clone()))
        .or_else(|| {
            let prefix = format!("{flag}=");
            args.iter()
                .find_map(|arg| arg.strip_prefix(&prefix).map(str::to_string))
        })
}

pub(crate) fn parse_multi_flag(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let prefix = format!("{flag}=");
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            if let Some(value) = args.get(i + 1) {
                values.push(value.clone());
                i += 2;
                continue;
            }
        } else if let Some(value) = args[i].strip_prefix(&prefix) {
            values.push(value.to_string());
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
        eprintln!("error: missing {flag}");
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

#[cfg(test)]
mod tests {
    use super::{parse_flag, parse_multi_flag};

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn single_value_parser_accepts_space_and_equals_forms() {
        assert_eq!(
            parse_flag(&args(&["--name", "space"]), "--name"),
            Some("space".into())
        );
        assert_eq!(
            parse_flag(&args(&["--name=equals"]), "--name"),
            Some("equals".into())
        );
    }

    #[test]
    fn single_value_parser_prefers_space_form_even_when_equals_form_appears_first() {
        assert_eq!(
            parse_flag(&args(&["--name=equals", "--name", "space"]), "--name"),
            Some("space".into())
        );
    }

    #[test]
    fn exact_flag_in_final_position_contributes_no_value() {
        let input = args(&["--name"]);
        assert_eq!(parse_flag(&input, "--name"), None);
        assert!(parse_multi_flag(&input, "--name").is_empty());
    }

    #[test]
    fn parse_multi_flag_collects_mixed_repeated_forms_in_order() {
        assert_eq!(
            parse_multi_flag(
                &args(&["--name=first", "--name", "second", "--name=third", "--name"]),
                "--name"
            ),
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn equals_form_preserves_empty_values() {
        let input = args(&["--name="]);
        assert_eq!(parse_flag(&input, "--name"), Some(String::new()));
        assert_eq!(parse_multi_flag(&input, "--name"), vec![""]);
    }
}
