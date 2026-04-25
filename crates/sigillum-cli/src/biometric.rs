use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{self, Command, Stdio};

use serde::Serialize;
use sigillum_api::request::BiometricEnrollRequest;
use sigillum_client::SigillumClient;
use sigillum_core::payload::biometric::BiometricHelperOutput;
use zeroize::Zeroize;

use crate::daemon_api::{daemon_base_url, ensure_daemon_ready, require_session_token};

const HELPER_BINARY_NAME: &str = "sigillum-auth";

pub(crate) fn cmd_unlock_biometric(args: &[String]) {
    let base_url = daemon_base_url(args);
    if let Err(error) = ensure_daemon_ready(&base_url) {
        eprintln!("{error}");
        process::exit(1);
    }

    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });

    let client = SigillumClient::new(base_url);
    let response = runtime.block_on(async {
        let challenge = client.biometric_challenge().await?;
        let challenge_id = decode_fixed_hex::<16>(&challenge.challenge_id_hex, "challenge_id_hex")
            .map_err(sigillum_client::ClientError::Encoding)?;
        let nonce = decode_fixed_hex::<32>(&challenge.nonce_hex, "nonce_hex")
            .map_err(sigillum_client::ClientError::Encoding)?;
        let helper = run_helper(&nonce).map_err(sigillum_client::ClientError::Encoding)?;
        let payload = helper
            .into_payload(challenge_id)
            .map_err(|error| sigillum_client::ClientError::Encoding(error.to_string()))?;
        client.biometric_unlock(hex::encode(payload.encode())).await
    });

    match response {
        Ok(response) => print_json(&response),
        Err(error) => {
            eprintln!("Biometric unlock failed: {error}");
            process::exit(1);
        }
    }
}

pub(crate) fn cmd_biometric(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: sigillum biometric enroll --public-key-hex <HEX> [--session TOKEN]");
        process::exit(1);
    }

    match args[0].as_str() {
        "enroll" => enroll(&args[1..]),
        other => {
            eprintln!("Unknown biometric command: {other}");
            process::exit(1);
        }
    }
}

fn enroll(args: &[String]) {
    let public_key_hex = parse_flag(args, "--public-key-hex").unwrap_or_else(|| {
        eprintln!("Usage: sigillum biometric enroll --public-key-hex <HEX> [--session TOKEN]");
        process::exit(1);
    });
    let passphrase = read_sensitive_input(
        args,
        "--passphrase-env",
        "--passphrase-stdin",
        "Daemon passphrase: ",
    );

    let base_url = daemon_base_url(args);
    if let Err(error) = ensure_daemon_ready(&base_url) {
        eprintln!("{error}");
        process::exit(1);
    }

    let client = SigillumClient::new(base_url);
    client.set_session_token(require_session_token(args));

    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|error| {
        eprintln!("Failed to start async runtime: {error}");
        process::exit(1);
    });
    let response = runtime.block_on(async {
        client
            .biometric_enroll(BiometricEnrollRequest {
                public_key_hex,
                passphrase,
            })
            .await
    });

    match response {
        Ok(response) => print_json(&response),
        Err(error) => {
            eprintln!("Biometric enrollment failed: {error}");
            process::exit(1);
        }
    }
}

fn run_helper(nonce: &[u8; 32]) -> Result<BiometricHelperOutput, String> {
    let helper_path = locate_helper()?;
    let output = Command::new(&helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(nonce)?;
            }
            child.wait_with_output()
        })
        .map_err(|error| format!("failed to execute {}: {error}", helper_path.display()))?;

    match output.status.code().unwrap_or(3) {
        0 => {}
        1 => return Err("biometric prompt was cancelled by the user".into()),
        2 => return Err("biometric subsystem is locked out".into()),
        4 => return Err("helper could not access the enrolled keychain item".into()),
        code => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("helper exited with code {code}: {stderr}"));
        }
    }

    let mut stdout = output.stdout;
    let decoded = BiometricHelperOutput::decode(&stdout)
        .map_err(|error| format!("invalid helper payload: {error}"))?;
    stdout.zeroize();
    Ok(decoded)
}

fn locate_helper() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("SIGILLUM_AUTH_HELPER") {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Ok(path);
        }
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    let sibling = current_exe
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(HELPER_BINARY_NAME);
    if sibling.exists() {
        return Ok(sibling);
    }

    Err(format!(
        "unable to locate {HELPER_BINARY_NAME}; set SIGILLUM_AUTH_HELPER to the compiled helper path"
    ))
}

fn decode_fixed_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N], String> {
    let bytes = hex::decode(value).map_err(|error| format!("invalid {label}: {error}"))?;
    if bytes.len() != N {
        return Err(format!(
            "invalid {label}: expected {N} bytes, got {}",
            bytes.len()
        ));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn read_sensitive_input(args: &[String], env_flag: &str, stdin_flag: &str, prompt: &str) -> String {
    if let Some(var_name) = parse_flag(args, env_flag) {
        return std::env::var(&var_name).unwrap_or_else(|_| {
            eprintln!("Environment variable {var_name} is not set.");
            process::exit(1);
        });
    }
    if has_flag(args, stdin_flag) {
        let mut value = String::new();
        io::stdin()
            .read_to_string(&mut value)
            .unwrap_or_else(|error| {
                eprintln!("Failed to read from stdin: {error}");
                process::exit(1);
            });
        return value.trim_end_matches(['\r', '\n']).to_string();
    }
    rpassword::prompt_password(prompt).unwrap_or_else(|error| {
        eprintln!("Failed to read secret input: {error}");
        process::exit(1);
    })
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find_map(|window| {
        if window[0] == flag {
            Some(window[1].clone())
        } else {
            None
        }
    })
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn print_json<T: Serialize>(value: &T) {
    let encoded = serde_json::to_string_pretty(value).unwrap_or_else(|error| {
        eprintln!("Failed to encode JSON response: {error}");
        process::exit(1);
    });
    println!("{encoded}");
}
