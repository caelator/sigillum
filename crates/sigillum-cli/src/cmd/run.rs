//! sigillum run — inject resolved secrets into a child process.
//!
//! Usage:
//!     sigillum run --env NAME=ref [--env NAME=ref ...] [--] -- command [args...]
//!     sigillum run --clear-env --env DB_PASS=prod:db.password -- npm start
//!
//! Secret refs: compartment:key or compartment:key.field
//!              (active compartment if compartment: is omitted)

use std::collections::HashMap;
use std::env;
use std::process::{Command, Stdio};

use crate::daemon_api::{daemon_base_url, ensure_daemon_ready, require_session_token};
use sigillum_client::SigillumClient;

use crate::exec::supervisor::ChildSupervisor;

/// Minimum env vars to always preserve when --clear-env is used.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL",
];

/// Parse --env NAME=ref arguments from the command line.
/// Returns (env_vars, positional_args) where env_vars maps NAME -> ref.
/// Everything after a bare `--` is treated as the command to run.
fn parse_env_flags(args: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let mut env_vars = HashMap::new();
    let mut positional = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Remaining args are the command
            positional.extend_from_slice(&args[i + 1..]);
            break;
        }
        if let Some(rest) = arg.strip_prefix("--env=") {
            // --env=NAME=ref (no space)
            if let Some((name, ref_part)) = rest.split_once('=') {
                env_vars.insert(name.to_string(), ref_part.to_string());
            } else {
                eprintln!("error: --env= requires NAME=ref format, got: {rest}");
                std::process::exit(1);
            }
        } else if arg == "--env" && i + 1 < args.len() {
            // --env NAME=ref (with space)
            if let Some((name, ref_part)) = args[i + 1].split_once('=') {
                env_vars.insert(name.to_string(), ref_part.to_string());
                i += 1;
            } else {
                eprintln!(
                    "error: --env requires NAME=ref format, got: {}",
                    args[i + 1]
                );
                std::process::exit(1);
            }
        } else if arg == "--clear-env" {
            // No-op marker; handled in build_child_env
        } else if !arg.starts_with('-') {
            // First non-flag is the command
            positional.extend_from_slice(&args[i..]);
            break;
        } else {
            eprintln!("error: unknown flag: {arg}");
            eprintln!("Usage: sigillum run --env NAME=ref [--] -- command [args...]");
            std::process::exit(1);
        }
        i += 1;
    }

    (env_vars, positional)
}

/// Check if --clear-env is present.
fn has_clear_env(args: &[String]) -> bool {
    args.iter().any(|a| a == "--clear-env")
}

/// Build the environment for the child process.
/// If clear_env is true, start from scratch and only add safe vars + injected secrets.
/// Otherwise, inherit current env and add injected secrets.
fn build_child_env(clear_env: bool, injected: HashMap<String, String>) -> HashMap<String, String> {
    let mut env = if clear_env {
        HashMap::new()
    } else {
        env::vars().collect()
    };

    // Always add safe vars when clearing env
    if clear_env {
        for &key in SAFE_ENV_VARS {
            if let Ok(val) = env::var(key) {
                env.insert(key.to_string(), val);
            }
        }
        // Also include any LC_* vars that are set
        for (key, val) in env::vars() {
            if key.starts_with("LC_") {
                env.insert(key, val);
            }
        }
    }

    // Inject resolved secrets
    for (name, value) in injected {
        env.insert(name, value);
    }

    env
}

/// Resolve a batch of secret refs to plaintext values.
fn resolve_secrets(
    base_url: &str,
    session_token: &str,
    refs: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    use sigillum_api::request::SecretResolveRequest;

    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;

    let client =
        SigillumClient::new(base_url).map_err(|e| format!("failed to build daemon client: {e}"))?;
    client.set_session_token(session_token);

    let entries: Vec<SecretResolveRequest> = refs
        .iter()
        .map(|(name, reference)| SecretResolveRequest {
            env_name: name.clone(),
            reference: reference.clone(),
        })
        .collect();

    let request = sigillum_api::request::SecretResolveBatchRequest { entries };

    let values = runtime
        .block_on(client.resolve_secret_batch(request))
        .map_err(|e| format!("failed to resolve secrets: {e}"))?;

    Ok(values.into_iter().map(|v| (v.env_name, v.value)).collect())
}

/// Record run completion audit event.
fn record_audit(
    base_url: &str,
    session_token: &str,
    program: &str,
    args: &[String],
    exit_code: Option<i32>,
    signal: Option<i32>,
    success: bool,
) -> Result<(), String> {
    use sigillum_api::request::RunAuditRequest;

    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let client =
        SigillumClient::new(base_url).map_err(|e| format!("failed to build daemon client: {e}"))?;
    client.set_session_token(session_token);

    let request = RunAuditRequest {
        program: program.to_string(),
        args: args.to_vec(),
        exit_code,
        signal,
        success,
    };

    runtime
        .block_on(client.record_run_audit(request))
        .map_err(|e| format!("failed to record audit: {e}"))?;
    Ok(())
}

/// Main entry point for `sigillum run`.
pub fn cmd_run(args: &[String]) {
    let (env_flags, positional) = parse_env_flags(args);

    if positional.is_empty() {
        eprintln!("error: no command specified");
        eprintln!("Usage: sigillum run --env NAME=ref [--] -- command [args...]");
        std::process::exit(1);
    }

    let clear_env = has_clear_env(args);
    let base_url = daemon_base_url(args);
    let session_token = require_session_token(args);

    // Ensure daemon is running
    if let Err(e) = ensure_daemon_ready(&base_url) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }

    // Resolve all secret refs
    let resolved = match resolve_secrets(&base_url, &session_token, &env_flags) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    // Build child environment
    let child_env = build_child_env(clear_env, resolved);

    // Split positional into program + args
    let program = &positional[0];
    let child_args = &positional[1..];

    // Spawn child with env
    let mut cmd = Command::new(program);
    cmd.args(child_args)
        .env_clear()
        .envs(&child_env)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit());

    // Spawn + wait (no explicit signal forwarding; see ChildSupervisor docs).
    let mut supervisor = match ChildSupervisor::spawn(&mut cmd) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to spawn child: {e}");
            std::process::exit(1);
        }
    };

    // Wait for child and record audit
    let (exit_code, signal) = match supervisor.wait() {
        Ok(status) => status,
        Err(e) => {
            eprintln!("error: failed to wait for child: {e}");
            std::process::exit(1);
        }
    };
    let success = exit_code == Some(0);

    if let Err(e) = record_audit(
        &base_url,
        &session_token,
        program,
        child_args,
        exit_code,
        signal,
        success,
    ) {
        eprintln!("warning: failed to record audit: {e}");
    }

    // Exit with child's status
    std::process::exit(exit_code.unwrap_or(128));
}
