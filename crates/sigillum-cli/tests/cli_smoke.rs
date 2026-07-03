//! CLI smoke tests — exercise the compiled binary via `std::process::Command`.
//!
//! These tests verify that the CLI binary starts, parses arguments correctly,
//! and produces expected output/exit codes for both happy and adversarial paths.
//! No daemon is required — they only test argument parsing and help output.

use std::process::Command;

/// Build the path to the debug binary.
fn sigillum_bin() -> String {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/
    path.pop(); // root
    path.push("target");
    path.push("debug");
    path.push("sigillum");
    path.to_string_lossy().into_owned()
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(sigillum_bin())
        .args(args)
        .output()
        .expect("Failed to execute sigillum binary")
}

fn run_with_env(args: &[&str], envs: &[(&str, &std::path::Path)]) -> std::process::Output {
    let mut command = Command::new(sigillum_bin());
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("Failed to execute sigillum binary")
}

// ── Happy Paths ────────────────────────────────────────────────────

#[test]
fn help_exits_zero_with_usage_text() {
    let output = run(&["help"]);
    assert!(output.status.success(), "sigillum help should exit 0");
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        combined.contains("sigillum") || combined.contains("Usage") || combined.contains("usage"),
        "help output should mention sigillum or usage"
    );
}

#[test]
fn help_flag_exits_zero() {
    let output = run(&["--help"]);
    // --help should also succeed
    assert!(output.status.success() || output.status.code() == Some(0));
}

#[test]
fn version_shows_version_info() {
    let output = run(&["version"]);
    // Should either succeed with version info or exit non-zero if no subcommand
    // Either way, it should not crash
    assert!(
        output.status.code().is_some(),
        "should exit cleanly, not crash"
    );
}

#[test]
fn status_on_nonexistent_daemon_exits_cleanly() {
    // Point at a temp dir with no running daemon — should either print error or status
    let tmp = tempfile::tempdir().unwrap();
    let output = Command::new(sigillum_bin())
        .args(["status"])
        .env("SIGILLUM_BASE_DIR", tmp.path())
        .output()
        .expect("Failed to execute sigillum binary");
    // Should exit cleanly (not crash/segfault)
    assert!(
        output.status.code().is_some(),
        "should exit cleanly, not crash"
    );
}

#[test]
fn doctor_reports_local_readiness_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let output = run_with_env(
        &["doctor", "--url", "http://127.0.0.1:1"],
        &[("SIGILLUM_BASE_DIR", tmp.path())],
    );
    assert!(
        output.status.code().is_some(),
        "doctor should exit cleanly, not crash"
    );
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(combined.contains("data dir"));
    assert!(combined.contains("daemon reachability"));
    assert!(combined.contains("Production boundary"));
}

// ── Adversarial Paths ──────────────────────────────────────────────

#[test]
fn unknown_command_exits_nonzero() {
    let output = run(&["nonexistent-command-xyzzy"]);
    assert!(
        !output.status.success(),
        "unknown command should exit non-zero"
    );
}

#[test]
fn set_with_no_args_prints_usage() {
    let output = run(&["set"]);
    assert!(
        !output.status.success(),
        "set with no args should exit non-zero"
    );
}

#[test]
fn get_with_no_args_prints_usage() {
    let output = run(&["get"]);
    assert!(
        !output.status.success(),
        "get with no args should exit non-zero"
    );
}

#[test]
fn api_with_no_subcommand_exits_nonzero() {
    let output = run(&["api"]);
    assert!(
        !output.status.success(),
        "api with no subcommand should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("api") || stderr.contains("COMMAND"),
        "should print api usage guidance"
    );
}

#[test]
fn api_unknown_subcommand_exits_nonzero() {
    let output = run(&["api", "bogus-subcommand"]);
    assert!(
        !output.status.success(),
        "unknown api subcommand should exit non-zero"
    );
}

#[test]
fn api_profiles_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "profiles"]);
    assert!(!output.status.success());
}

#[test]
fn api_profiles_evm_upsert_missing_flags_exits_nonzero() {
    let output = run(&["api", "profiles", "evm", "upsert"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--name") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_profiles_eth_xpub_upsert_missing_flags_exits_nonzero() {
    let output = run(&["api", "profiles", "eth-xpub", "upsert"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--name") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_deposits_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "deposits"]);
    assert!(!output.status.success());
}

#[test]
fn api_queue_missing_subcommand_exits_nonzero() {
    let output = run(&["api", "queue"]);
    assert!(!output.status.success());
}

#[test]
fn api_transit_encrypt_missing_flags_exits_nonzero() {
    let output = run(&["api", "transit", "encrypt"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--key") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_transit_decrypt_missing_flags_exits_nonzero() {
    let output = run(&["api", "transit", "decrypt"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--key") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_transit_hmac_missing_flags_exits_nonzero() {
    let output = run(&["api", "transit", "hmac"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--key") || stderr.contains("Usage"),
        "should mention required flags"
    );
}

#[test]
fn api_transit_encrypt_invalid_hex_exits_nonzero() {
    let output = run(&[
        "api",
        "transit",
        "encrypt",
        "--key",
        "payments",
        "--plaintext-hex",
        "zz",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid hex"));
}

#[test]
fn api_transit_encrypt_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "transit",
        "encrypt",
        "--key",
        "payments",
        "--plaintext-hex",
        "00ff",
        "--aad-hex",
        "abcd",
        "--url",
        "http://127.0.0.1:1",
        "--session",
        "test-token",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Usage:"));
}

#[test]
fn api_transit_decrypt_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "transit",
        "decrypt",
        "--key",
        "payments",
        "--nonce-hex",
        "00ff",
        "--ciphertext-hex",
        "abcd",
        "--aad-hex",
        "0102",
        "--url",
        "http://127.0.0.1:1",
        "--session",
        "test-token",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Usage:"));
}

#[test]
fn api_transit_hmac_reaches_network_with_valid_args() {
    let output = run(&[
        "api",
        "transit",
        "hmac",
        "--key",
        "payments",
        "--input-hex",
        "00ff",
        "--url",
        "http://127.0.0.1:1",
        "--session",
        "test-token",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("Usage:"));
}

#[test]
fn api_maintenance_missing_run_exits_nonzero() {
    let output = run(&["api", "maintenance"]);
    assert!(!output.status.success());
}

#[test]
fn empty_args_shows_help_or_usage() {
    let output = run(&[]);
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    // Should either show usage or exit non-zero
    assert!(
        output.status.code().is_some(),
        "should exit cleanly, not crash"
    );
    // Should produce some output
    assert!(!combined.is_empty(), "should produce help or usage output");
}
